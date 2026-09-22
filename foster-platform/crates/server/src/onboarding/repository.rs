use chrono::{DateTime, Utc};
use sqlx::{MySql, MySqlPool, Transaction};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OnboardingPlanRow {
    pub id: i64,
    pub plan_code: String,
    pub plan_name: String,
    pub daily_target_runs: i32,
    pub interval_minutes: i32,
    pub resource_mode: String,
    pub resource_type: Option<String>,
    pub status: String,
}

pub async fn load_active_plan(
    pool: &MySqlPool,
    plan_code: &str,
) -> Result<Option<OnboardingPlanRow>, sqlx::Error> {
    sqlx::query_as::<_, OnboardingPlanRow>(
        "SELECT
            id,
            plan_code,
            plan_name,
            daily_target_runs,
            interval_minutes,
            resource_mode,
            resource_type,
            status
         FROM foster_plan
         WHERE plan_code = ?
           AND status = 'ACTIVE'",
    )
    .bind(plan_code)
    .fetch_optional(pool)
    .await
}

pub async fn insert_pending_account(
    tx: &mut Transaction<'_, MySql>,
    customer_id: i64,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO game_account(
            customer_id,
            login_status,
            verify_status
         )
         VALUES (?, 'PENDING', 'PENDING')",
    )
    .bind(customer_id)
    .execute(&mut **tx)
    .await?;

    Ok(result.last_insert_id() as i64)
}

pub struct NewPendingSubscription<'a> {
    pub subscription_no: &'a str,
    pub game_account_id: i64,
    pub plan: &'a OnboardingPlanRow,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
}

pub async fn insert_pending_subscription(
    tx: &mut Transaction<'_, MySql>,
    input: NewPendingSubscription<'_>,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO foster_subscription(
            subscription_no,
            game_account_id,
            plan_id,
            resource_mode,
            resource_type,
            daily_target_runs,
            interval_minutes,
            status,
            start_at,
            end_at,
            next_run_at
         )
         VALUES (?, ?, ?, ?, ?, ?, ?, 'PENDING_LOGIN', ?, ?, NULL)",
    )
    .bind(input.subscription_no)
    .bind(input.game_account_id)
    .bind(input.plan.id)
    .bind(&input.plan.resource_mode)
    .bind(&input.plan.resource_type)
    .bind(input.plan.daily_target_runs)
    .bind(input.plan.interval_minutes)
    .bind(input.start_at.naive_utc())
    .bind(input.end_at.naive_utc())
    .execute(&mut **tx)
    .await?;

    Ok(result.last_insert_id() as i64)
}

pub async fn cleanup_failed_onboarding(
    pool: &MySqlPool,
    game_account_id: i64,
    subscription_id: i64,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "DELETE FROM foster_share_link
         WHERE subscription_id = ?",
    )
    .bind(subscription_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "DELETE FROM login_session
         WHERE game_account_id = ?",
    )
    .bind(game_account_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "DELETE FROM emulator_account_binding
         WHERE game_account_id = ?
           AND status IN ('PENDING', 'UNBOUND')",
    )
    .bind(game_account_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "DELETE FROM foster_subscription
         WHERE id = ?",
    )
    .bind(subscription_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "DELETE FROM game_account
         WHERE id = ?",
    )
    .bind(game_account_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}
