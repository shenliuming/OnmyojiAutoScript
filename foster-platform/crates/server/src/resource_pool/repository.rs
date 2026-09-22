use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{MySql, Transaction};

#[derive(Debug, sqlx::FromRow)]
pub struct LockedResourceJob {
    pub id: i64,
    pub game_account_id: i64,
    pub status: String,
    pub resource_mode: String,
    pub resource_type: Option<String>,
    pub interval_minutes: i32,
}

#[derive(Debug, sqlx::FromRow)]
pub struct AllocationRow {
    pub id: i64,
    pub resource_cycle_id: i64,
    pub provider_account_id: i64,
    pub provider_alias: String,
    pub resource_type: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ResourceCandidateRow {
    pub resource_cycle_id: i64,
    pub provider_account_id: i64,
    pub provider_alias: String,
}

pub async fn lock_resource_job(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
) -> Result<Option<LockedResourceJob>, sqlx::Error> {
    sqlx::query_as::<_, LockedResourceJob>(
        "SELECT
            j.id,
            j.game_account_id,
            j.status,
            s.resource_mode,
            s.resource_type,
            s.interval_minutes
         FROM foster_job j
         JOIN foster_subscription s ON s.id = j.subscription_id
         WHERE j.id = ?
         FOR UPDATE",
    )
    .bind(job_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn load_live_allocation(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
) -> Result<Option<AllocationRow>, sqlx::Error> {
    sqlx::query_as::<_, AllocationRow>(
        "SELECT
            a.id,
            a.job_id,
            a.resource_cycle_id,
            a.provider_account_id,
            p.provider_alias,
            c.resource_type,
            a.status,
            c.end_at
         FROM foster_resource_allocation a
         JOIN foster_resource_cycle c ON c.id = a.resource_cycle_id
         JOIN provider_account p ON p.id = a.provider_account_id
         WHERE a.job_id = ?
           AND a.status IN ('RESERVED', 'CONFIRMED')
         FOR UPDATE",
    )
    .bind(job_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn list_resource_candidates(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
    resource_type: &str,
    now: DateTime<Utc>,
    min_end_at: DateTime<Utc>,
) -> Result<Vec<ResourceCandidateRow>, sqlx::Error> {
    sqlx::query_as::<_, ResourceCandidateRow>(
        "SELECT
            c.id AS resource_cycle_id,
            c.provider_account_id,
            p.provider_alias
         FROM foster_resource_cycle c
         JOIN provider_account p
           ON p.id = c.provider_account_id
          AND p.status = 'ACTIVE'
         JOIN foster_friend_binding b
           ON b.provider_account_id = p.id
          AND b.game_account_id = ?
          AND b.status = 'VERIFIED'
         WHERE c.resource_type = ?
           AND c.status = 'AVAILABLE'
           AND c.start_at <= ?
           AND c.end_at >= ?
           AND c.occupied_slots < c.slot_capacity
         ORDER BY c.end_at ASC, c.occupied_slots DESC, c.id ASC",
    )
    .bind(game_account_id)
    .bind(resource_type)
    .bind(now.naive_utc())
    .bind(min_end_at.naive_utc())
    .fetch_all(&mut **tx)
    .await
}

pub async fn try_reserve_cycle(
    tx: &mut Transaction<'_, MySql>,
    cycle_id: i64,
    min_end_at: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE foster_resource_cycle
         SET occupied_slots = occupied_slots + 1,
             status = CASE
                 WHEN occupied_slots + 1 >= slot_capacity THEN 'FULL'
                 ELSE 'AVAILABLE'
             END
         WHERE id = ?
           AND status = 'AVAILABLE'
           AND occupied_slots < slot_capacity
           AND end_at >= ?",
    )
    .bind(cycle_id)
    .bind(min_end_at.naive_utc())
    .execute(&mut **tx)
    .await?;

    Ok(result.rows_affected() == 1)
}

pub async fn insert_reserved_allocation(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
    cycle_id: i64,
    provider_account_id: i64,
    now: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO foster_resource_allocation(
            job_id,
            resource_cycle_id,
            provider_account_id,
            status,
            reserved_at
         )
         VALUES (?, ?, ?, 'RESERVED', ?)",
    )
    .bind(job_id)
    .bind(cycle_id)
    .bind(provider_account_id)
    .bind(now.naive_utc())
    .execute(&mut **tx)
    .await?;

    Ok(result.last_insert_id() as i64)
}

pub async fn attach_allocation_to_job(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
    provider_account_id: i64,
    cycle_id: i64,
    allocation_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_job
         SET provider_account_id = ?,
             resource_cycle_id = ?,
             resource_allocation_id = ?,
             error_code = NULL,
             result_message = NULL
         WHERE id = ?",
    )
    .bind(provider_account_id)
    .bind(cycle_id)
    .bind(allocation_id)
    .bind(job_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn set_job_waiting_resource(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
    message: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_job
         SET status = 'WAITING_RESOURCE',
             provider_account_id = NULL,
             resource_cycle_id = NULL,
             resource_allocation_id = NULL,
             error_code = 'PROVIDER_NOT_FOUND',
             result_message = ?,
             retry_after = NULL
         WHERE id = ?
           AND status IN ('PENDING', 'SWITCHING_ACCOUNT', 'WAITING_RESOURCE')",
    )
    .bind(message)
    .bind(job_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn confirm_allocation(
    tx: &mut Transaction<'_, MySql>,
    allocation_id: i64,
    completed_at: DateTime<Utc>,
    occupied_until: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE foster_resource_allocation
         SET status = 'CONFIRMED',
             confirmed_at = COALESCE(confirmed_at, ?),
             occupied_until = COALESCE(occupied_until, ?)
         WHERE id = ?
           AND status = 'RESERVED'",
    )
    .bind(completed_at.naive_utc())
    .bind(occupied_until.naive_utc())
    .bind(allocation_id)
    .execute(&mut **tx)
    .await?;

    Ok(result.rows_affected() == 1)
}

#[derive(Debug, sqlx::FromRow)]
pub struct ReleasableAllocationRow {
    pub id: i64,
    pub resource_cycle_id: i64,
    pub provider_account_id: i64,
    pub status: String,
}

pub async fn lock_releasable_allocation(
    tx: &mut Transaction<'_, MySql>,
    job_id: i64,
) -> Result<Option<ReleasableAllocationRow>, sqlx::Error> {
    sqlx::query_as::<_, ReleasableAllocationRow>(
        "SELECT
            id,
            job_id,
            resource_cycle_id,
            provider_account_id,
            status
         FROM foster_resource_allocation
         WHERE job_id = ?
           AND status IN ('RESERVED', 'CONFIRMED')
         FOR UPDATE",
    )
    .bind(job_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn finish_allocation(
    tx: &mut Transaction<'_, MySql>,
    allocation_id: i64,
    next_status: &str,
    now: DateTime<Utc>,
    reason: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE foster_resource_allocation
         SET status = ?,
             released_at = ?,
             release_reason = ?
         WHERE id = ?
           AND status IN ('RESERVED', 'CONFIRMED')",
    )
    .bind(next_status)
    .bind(now.naive_utc())
    .bind(reason)
    .bind(allocation_id)
    .execute(&mut **tx)
    .await?;

    Ok(result.rows_affected() == 1)
}

pub async fn release_cycle_slot(
    tx: &mut Transaction<'_, MySql>,
    cycle_id: i64,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_resource_cycle
         SET occupied_slots = GREATEST(occupied_slots - 1, 0),
             status = CASE
                 WHEN status = 'DISABLED' THEN 'DISABLED'
                 WHEN end_at <= ? THEN 'ENDED'
                 ELSE 'AVAILABLE'
             END
         WHERE id = ?",
    )
    .bind(now.naive_utc())
    .bind(cycle_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn mark_friend_binding_suspect(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
    provider_account_id: i64,
    error_code: &str,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_friend_binding
         SET status = 'SUSPECT',
             last_failure_at = ?,
             last_failure_code = ?
         WHERE game_account_id = ?
           AND provider_account_id = ?
           AND status = 'VERIFIED'",
    )
    .bind(now.naive_utc())
    .bind(error_code)
    .bind(game_account_id)
    .bind(provider_account_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn list_expirable_job_ids(
    tx: &mut Transaction<'_, MySql>,
    now: DateTime<Utc>,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT DISTINCT a.job_id
         FROM foster_resource_allocation a
         JOIN foster_resource_cycle c ON c.id = a.resource_cycle_id
         WHERE (
             a.status = 'CONFIRMED'
             AND a.occupied_until IS NOT NULL
             AND a.occupied_until <= ?
         ) OR (
             a.status = 'RESERVED'
             AND c.end_at <= ?
         )
         ORDER BY a.job_id ASC",
    )
    .bind(now.naive_utc())
    .bind(now.naive_utc())
    .fetch_all(&mut **tx)
    .await
}

pub async fn mark_expired_cycles(
    tx: &mut Transaction<'_, MySql>,
    now: DateTime<Utc>,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE foster_resource_cycle
         SET status = 'ENDED'
         WHERE end_at <= ?
           AND status IN ('AVAILABLE', 'FULL')",
    )
    .bind(now.naive_utc())
    .execute(&mut **tx)
    .await?;

    Ok(result.rows_affected())
}
