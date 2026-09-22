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

pub async fn transition_job_status(
    pool: &MySqlPool,
    job_id: i64,
    expected_status: &str,
    next_status: &str,
    now: DateTime<Utc>,
    next_is_executing: bool,
    next_is_terminal: bool,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE foster_job
         SET status = ?,
             started_at = CASE
                 WHEN ? THEN COALESCE(started_at, ?)
                 ELSE started_at
             END,
             finished_at = CASE
                 WHEN ? THEN ?
                 ELSE finished_at
             END
         WHERE id = ?
           AND status = ?",
    )
    .bind(next_status)
    .bind(next_is_executing)
    .bind(now.naive_utc())
    .bind(next_is_terminal)
    .bind(now.naive_utc())
    .bind(job_id)
    .bind(expected_status)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() == 1)
}

#[derive(Debug, sqlx::FromRow)]
pub struct SuccessJobRow {
    pub id: i64,
    pub subscription_id: i64,
    pub status: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct SuccessSubscriptionRow {
    pub id: i64,
    pub interval_minutes: i32,
}

pub async fn lock_job_for_success(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
) -> Result<Option<SuccessJobRow>, sqlx::Error> {
    sqlx::query_as::<_, SuccessJobRow>(
        "SELECT id, subscription_id, status
         FROM foster_job
         WHERE id = ?
         FOR UPDATE",
    )
    .bind(job_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn lock_subscription_for_success(
    tx: &mut Transaction<'_, MySql>,
    subscription_id: i64,
) -> Result<Option<SuccessSubscriptionRow>, sqlx::Error> {
    sqlx::query_as::<_, SuccessSubscriptionRow>(
        "SELECT id, interval_minutes
         FROM foster_subscription
         WHERE id = ?
         FOR UPDATE",
    )
    .bind(subscription_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn mark_job_success(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
    success_at: DateTime<Utc>,
    remaining_seconds: Option<i32>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_job
         SET status = 'SUCCESS',
             finished_at = ?,
             remaining_seconds = ?,
             error_code = NULL,
             result_message = NULL,
             retry_after = NULL
         WHERE id = ?",
    )
    .bind(success_at.naive_utc())
    .bind(remaining_seconds)
    .bind(job_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn schedule_subscription_after_success(
    tx: &mut Transaction<'_, MySql>,
    subscription_id: i64,
    success_at: DateTime<Utc>,
    next_run_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_subscription
         SET last_success_at = ?,
             next_run_at = ?
         WHERE id = ?",
    )
    .bind(success_at.naive_utc())
    .bind(next_run_at.naive_utc())
    .bind(subscription_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn count_successes_between(
    pool: &MySqlPool,
    game_account_id: i64,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM foster_job
         WHERE game_account_id = ?
           AND status = 'SUCCESS'
           AND finished_at >= ?
           AND finished_at < ?",
    )
    .bind(game_account_id)
    .bind(start.naive_utc())
    .bind(end.naive_utc())
    .fetch_one(pool)
    .await
}


#[derive(Debug, sqlx::FromRow)]
pub struct FailureJobRow {
    pub id: i64,
    pub subscription_id: i64,
    pub game_account_id: i64,
    pub status: String,
    pub retry_count: i32,
}

#[derive(Debug, sqlx::FromRow)]
pub struct FailureSubscriptionRow {
    pub id: i64,
    pub interval_minutes: i32,
}

pub async fn lock_job_for_failure(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
) -> Result<Option<FailureJobRow>, sqlx::Error> {
    sqlx::query_as::<_, FailureJobRow>(
        "SELECT id, subscription_id, game_account_id, status, retry_count
         FROM foster_job
         WHERE id = ?
         FOR UPDATE",
    )
    .bind(job_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn lock_subscription_for_failure(
    tx: &mut Transaction<'_, MySql>,
    subscription_id: i64,
) -> Result<Option<FailureSubscriptionRow>, sqlx::Error> {
    sqlx::query_as::<_, FailureSubscriptionRow>(
        "SELECT id, interval_minutes
         FROM foster_subscription
         WHERE id = ?
         FOR UPDATE",
    )
    .bind(subscription_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn set_job_retry(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
    retry_count: i32,
    retry_after: DateTime<Utc>,
    error_code: &str,
    result_message: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_job
         SET status = 'RETRY',
             retry_count = ?,
             retry_after = ?,
             error_code = ?,
             result_message = ?,
             finished_at = NULL
         WHERE id = ?",
    )
    .bind(retry_count)
    .bind(retry_after.naive_utc())
    .bind(error_code)
    .bind(result_message)
    .bind(job_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn set_job_waiting_emulator_failure(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
    error_code: &str,
    result_message: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_job
         SET status = 'WAITING_EMULATOR',
             retry_after = NULL,
             error_code = ?,
             result_message = ?,
             finished_at = NULL
         WHERE id = ?",
    )
    .bind(error_code)
    .bind(result_message)
    .bind(job_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn set_job_terminal_failure(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
    status: &str,
    retry_count: i32,
    failed_at: DateTime<Utc>,
    error_code: &str,
    result_message: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_job
         SET status = ?,
             retry_count = ?,
             retry_after = NULL,
             error_code = ?,
             result_message = ?,
             finished_at = ?
         WHERE id = ?",
    )
    .bind(status)
    .bind(retry_count)
    .bind(error_code)
    .bind(result_message)
    .bind(failed_at.naive_utc())
    .bind(job_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn suspend_subscription(
    tx: &mut Transaction<'_, MySql>,
    subscription_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_subscription
         SET status = 'SUSPENDED',
             next_run_at = NULL
         WHERE id = ?",
    )
    .bind(subscription_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn restore_subscription_schedule(
    tx: &mut Transaction<'_, MySql>,
    subscription_id: i64,
    next_run_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_subscription
         SET next_run_at = ?
         WHERE id = ?",
    )
    .bind(next_run_at.naive_utc())
    .bind(subscription_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn mark_account_relogin_required(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE game_account
         SET login_status = 'RELOGIN_REQUIRED'
         WHERE id = ?",
    )
    .bind(game_account_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn mark_account_identity_mismatch(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE game_account
         SET verify_status = 'MISMATCH'
         WHERE id = ?",
    )
    .bind(game_account_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}


pub async fn list_schedulable_job_ids(
    pool: &MySqlPool,
    now: DateTime<Utc>,
    limit: u32,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id
         FROM foster_job
         WHERE status = 'PENDING'
            OR status = 'WAITING_EMULATOR'
            OR (
                status IN ('DEFERRED_QUIET', 'DEFERRED_MANUAL')
                AND deferred_until IS NOT NULL
                AND deferred_until <= ?
            )
            OR (
                status = 'RETRY'
                AND retry_after IS NOT NULL
                AND retry_after <= ?
            )
         ORDER BY scheduled_at ASC, id ASC
         LIMIT ?",
    )
    .bind(now.naive_utc())
    .bind(now.naive_utc())
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await
}
