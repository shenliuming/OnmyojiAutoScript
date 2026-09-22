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

#[derive(Debug, sqlx::FromRow)]
pub struct JobGateRow {
    pub id: i64,
    pub status: String,
    pub game_account_id: i64,
    pub manual_pause_until: Option<NaiveDateTime>,
    pub login_status: String,
    pub verify_status: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct QuietPeriodRow {
    pub weekday_mask: u8,
    pub start_time: chrono::NaiveTime,
    pub end_time: chrono::NaiveTime,
    pub timezone: String,
    pub before_buffer_minutes: i32,
    pub after_buffer_minutes: i32,
}

pub async fn lock_job_gate_context(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
) -> Result<Option<JobGateRow>, sqlx::Error> {
    sqlx::query_as::<_, JobGateRow>(
        "SELECT
            j.id,
            j.status,
            j.game_account_id,
            s.manual_pause_until,
            a.login_status,
            a.verify_status
         FROM foster_job j
         JOIN foster_subscription s ON s.id = j.subscription_id
         JOIN game_account a ON a.id = j.game_account_id
         WHERE j.id = ?
         FOR UPDATE",
    )
    .bind(job_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn list_enabled_quiet_periods(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
) -> Result<Vec<QuietPeriodRow>, sqlx::Error> {
    sqlx::query_as::<_, QuietPeriodRow>(
        "SELECT
            weekday_mask,
            start_time,
            end_time,
            timezone,
            before_buffer_minutes,
            after_buffer_minutes
         FROM foster_quiet_period
         WHERE game_account_id = ?
           AND enabled = 1
         ORDER BY id ASC",
    )
    .bind(game_account_id)
    .fetch_all(&mut **tx)
    .await
}

pub async fn set_job_deferred(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
    status: &str,
    deferred_until: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_job
         SET status = ?,
             deferred_until = ?
         WHERE id = ?",
    )
    .bind(status)
    .bind(deferred_until.naive_utc())
    .bind(job_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn resume_job_pending(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_job
         SET status = 'PENDING',
             deferred_until = NULL
         WHERE id = ?",
    )
    .bind(job_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}


#[derive(Debug, sqlx::FromRow)]
pub struct ClaimJobRow {
    pub id: i64,
    pub status: String,
    pub game_account_id: i64,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ActiveBindingRow {
    pub emulator_id: i64,
}

#[derive(Debug, sqlx::FromRow)]
pub struct EmulatorClaimRow {
    pub id: i64,
    pub emulator_status: String,
    pub host_status: String,
}

pub async fn lock_job_for_claim(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
) -> Result<Option<ClaimJobRow>, sqlx::Error> {
    sqlx::query_as::<_, ClaimJobRow>(
        "SELECT id, status, game_account_id
         FROM foster_job
         WHERE id = ?
         FOR UPDATE",
    )
    .bind(job_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn lock_active_binding_for_account(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
) -> Result<Option<ActiveBindingRow>, sqlx::Error> {
    sqlx::query_as::<_, ActiveBindingRow>(
        "SELECT b.emulator_id
         FROM game_account a
         JOIN emulator_account_binding b
           ON b.game_account_id = a.id
          AND b.status = 'ACTIVE'
          AND b.emulator_id = a.active_emulator_id
         WHERE a.id = ?
         FOR UPDATE",
    )
    .bind(game_account_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn lock_emulator_for_claim(
    tx: &mut Transaction<'_, MySql>,
    emulator_id: i64,
) -> Result<Option<EmulatorClaimRow>, sqlx::Error> {
    sqlx::query_as::<_, EmulatorClaimRow>(
        "SELECT
            e.id,
            e.status AS emulator_status,
            h.status AS host_status
         FROM emulator_instance e
         JOIN host h ON h.id = e.host_id
         WHERE e.id = ?
         FOR UPDATE",
    )
    .bind(emulator_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn has_executing_job_for_emulator(
    tx: &mut Transaction<'_, MySql>,
    emulator_id: i64,
    excluding_job_id: i64,
) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM foster_job
         WHERE emulator_id = ?
           AND id <> ?
           AND status IN ('SWITCHING_ACCOUNT', 'VERIFYING_ACCOUNT', 'RUNNING')",
    )
    .bind(emulator_id)
    .bind(excluding_job_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(count > 0)
}

pub async fn has_executing_job_for_account(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
    excluding_job_id: i64,
) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM foster_job
         WHERE game_account_id = ?
           AND id <> ?
           AND status IN ('SWITCHING_ACCOUNT', 'VERIFYING_ACCOUNT', 'RUNNING')",
    )
    .bind(game_account_id)
    .bind(excluding_job_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(count > 0)
}

pub async fn set_job_waiting_emulator(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
    emulator_id: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_job
         SET status = 'WAITING_EMULATOR',
             emulator_id = ?,
             deferred_until = NULL
         WHERE id = ?",
    )
    .bind(emulator_id)
    .bind(job_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn set_job_switching_account(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
    emulator_id: i64,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_job
         SET status = 'SWITCHING_ACCOUNT',
             emulator_id = ?,
             started_at = COALESCE(started_at, ?),
             deferred_until = NULL
         WHERE id = ?",
    )
    .bind(emulator_id)
    .bind(now.naive_utc())
    .bind(job_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
