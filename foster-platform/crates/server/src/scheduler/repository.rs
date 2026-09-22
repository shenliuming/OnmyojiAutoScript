use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{MySql, MySqlPool, Transaction};

#[derive(Debug, sqlx::FromRow)]
pub struct DueSubscriptionRow {
    pub id: i64,
    pub game_account_id: i64,
    pub next_run_at: NaiveDateTime,
}

pub async fn list_due_subscription_ids(
    pool: &MySqlPool,
    now: DateTime<Utc>,
    limit: u32,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id
         FROM foster_subscription
         WHERE status = 'ACTIVE'
           AND start_at <= ?
           AND end_at > ?
           AND next_run_at IS NOT NULL
           AND next_run_at <= ?
         ORDER BY next_run_at ASC, id ASC
         LIMIT ?",
    )
    .bind(now.naive_utc())
    .bind(now.naive_utc())
    .bind(now.naive_utc())
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await
}

pub async fn lock_due_subscription(
    tx: &mut Transaction<'_, MySql>,
    subscription_id: i64,
    now: DateTime<Utc>,
) -> Result<Option<DueSubscriptionRow>, sqlx::Error> {
    sqlx::query_as::<_, DueSubscriptionRow>(
        "SELECT id, game_account_id, next_run_at
         FROM foster_subscription
         WHERE id = ?
           AND status = 'ACTIVE'
           AND start_at <= ?
           AND end_at > ?
           AND next_run_at IS NOT NULL
           AND next_run_at <= ?
         FOR UPDATE",
    )
    .bind(subscription_id)
    .bind(now.naive_utc())
    .bind(now.naive_utc())
    .bind(now.naive_utc())
    .fetch_optional(&mut **tx)
    .await
}

pub async fn has_active_binding(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM emulator_account_binding
         WHERE game_account_id = ?
           AND status = 'ACTIVE'",
    )
    .bind(game_account_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(count > 0)
}

pub async fn has_nonterminal_job(
    tx: &mut Transaction<'_, MySql>,
    subscription_id: i64,
) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM foster_job
         WHERE subscription_id = ?
           AND status IN (
               'PENDING',
               'DEFERRED_QUIET',
               'DEFERRED_MANUAL',
               'WAITING_EMULATOR',
               'WAITING_RESOURCE',
               'SWITCHING_ACCOUNT',
               'VERIFYING_ACCOUNT',
               'RUNNING',
               'RETRY',
               'RECOVERY_REQUIRED'
           )",
    )
    .bind(subscription_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(count > 0)
}

pub async fn insert_pending_job(
    tx: &mut Transaction<'_, MySql>,
    job_no: &str,
    subscription_id: i64,
    game_account_id: i64,
    scheduled_at: NaiveDateTime,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO foster_job(
            job_no,
            subscription_id,
            game_account_id,
            status,
            scheduled_at
         )
         VALUES (?, ?, ?, 'PENDING', ?)",
    )
    .bind(job_no)
    .bind(subscription_id)
    .bind(game_account_id)
    .bind(scheduled_at)
    .execute(&mut **tx)
    .await?;

    Ok(result.last_insert_id() as i64)
}

pub async fn clear_next_run(
    tx: &mut Transaction<'_, MySql>,
    subscription_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_subscription
         SET next_run_at = NULL
         WHERE id = ?",
    )
    .bind(subscription_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
