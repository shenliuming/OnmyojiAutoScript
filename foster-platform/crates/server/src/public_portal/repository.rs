use chrono::{DateTime, NaiveDateTime, NaiveTime, Utc};
use sqlx::{FromRow, MySql, MySqlPool, Transaction};

#[derive(Debug, Clone, FromRow)]
pub struct ShareLinkRow {
    pub id: i64,
    pub subscription_id: i64,
    pub status: String,
    pub expire_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, FromRow)]
pub struct PortalSubscriptionRow {
    pub subscription_no: String,
    pub service_status: String,
    pub game_account_id: i64,
    pub character_name: Option<String>,
    pub server_name: Option<String>,
    pub login_status: String,
    pub verify_status: String,
    pub plan_name: String,
    pub resource_mode: String,
    pub resource_type: Option<String>,
    pub daily_target_runs: i32,
    pub last_success_at: Option<NaiveDateTime>,
    pub next_run_at: Option<NaiveDateTime>,
    pub manual_pause_until: Option<NaiveDateTime>,
    pub service_end_at: NaiveDateTime,
}

#[derive(Debug, Clone, FromRow)]
pub struct QuietPeriodRow {
    pub weekday_mask: u8,
    pub start_time: NaiveTime,
    pub end_time: NaiveTime,
    pub timezone: String,
    pub before_buffer_minutes: i32,
    pub after_buffer_minutes: i32,
}

#[derive(Debug, Clone, FromRow)]
pub struct RecentJobRow {
    pub status: String,
    pub scheduled_at: NaiveDateTime,
    pub started_at: Option<NaiveDateTime>,
    pub finished_at: Option<NaiveDateTime>,
    pub result_message: Option<String>,
    pub screenshot_url: Option<String>,
}

pub async fn revoke_active_links(
    tx: &mut Transaction<'_, MySql>,
    subscription_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_share_link
         SET status = 'REVOKED'
         WHERE subscription_id = ?
           AND status = 'ACTIVE'",
    )
    .bind(subscription_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn insert_share_link(
    tx: &mut Transaction<'_, MySql>,
    subscription_id: i64,
    public_hash: &str,
    control_hash: &str,
    expire_at: Option<DateTime<Utc>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO foster_share_link(
            subscription_id, public_token_hash, control_token_hash,
            status, expire_at
         )
         VALUES (?, ?, ?, 'ACTIVE', ?)",
    )
    .bind(subscription_id)
    .bind(public_hash)
    .bind(control_hash)
    .bind(expire_at.map(|value| value.naive_utc()))
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn load_share_by_public_hash(
    pool: &MySqlPool,
    token_hash: &str,
) -> Result<Option<ShareLinkRow>, sqlx::Error> {
    sqlx::query_as::<_, ShareLinkRow>(
        "SELECT id, subscription_id, status, expire_at
         FROM foster_share_link
         WHERE public_token_hash = ?",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

pub async fn load_share_by_control_hash(
    pool: &MySqlPool,
    token_hash: &str,
) -> Result<Option<ShareLinkRow>, sqlx::Error> {
    sqlx::query_as::<_, ShareLinkRow>(
        "SELECT id, subscription_id, status, expire_at
         FROM foster_share_link
         WHERE control_token_hash = ?",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

pub async fn touch_share_link(
    pool: &MySqlPool,
    share_link_id: i64,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_share_link
         SET last_access_at = ?,
             access_count = access_count + 1
         WHERE id = ?",
    )
    .bind(now.naive_utc())
    .bind(share_link_id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn load_portal_subscription(
    pool: &MySqlPool,
    subscription_id: i64,
) -> Result<Option<PortalSubscriptionRow>, sqlx::Error> {
    sqlx::query_as::<_, PortalSubscriptionRow>(
        "SELECT
            s.subscription_no,
            s.status AS service_status,
            s.game_account_id,
            a.character_name,
            a.server_name,
            a.login_status,
            a.verify_status,
            p.plan_name,
            s.resource_mode,
            s.resource_type,
            s.daily_target_runs,
            s.last_success_at,
            s.next_run_at,
            s.manual_pause_until,
            s.end_at AS service_end_at
         FROM foster_subscription s
         JOIN game_account a ON a.id = s.game_account_id
         JOIN foster_plan p ON p.id = s.plan_id
         WHERE s.id = ?",
    )
    .bind(subscription_id)
    .fetch_optional(pool)
    .await
}

pub async fn load_quiet_periods(
    pool: &MySqlPool,
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
         ORDER BY start_time ASC, id ASC",
    )
    .bind(game_account_id)
    .fetch_all(pool)
    .await
}

pub async fn load_recent_jobs(
    pool: &MySqlPool,
    subscription_id: i64,
    limit: u32,
) -> Result<Vec<RecentJobRow>, sqlx::Error> {
    sqlx::query_as::<_, RecentJobRow>(
        "SELECT
            status,
            scheduled_at,
            started_at,
            finished_at,
            result_message,
            screenshot_url
         FROM foster_job
         WHERE subscription_id = ?
         ORDER BY id DESC
         LIMIT ?",
    )
    .bind(subscription_id)
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await
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

pub async fn set_manual_pause_until(
    pool: &MySqlPool,
    subscription_id: i64,
    until: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_subscription
         SET manual_pause_until = CASE
             WHEN manual_pause_until IS NULL OR manual_pause_until < ?
             THEN ?
             ELSE manual_pause_until
         END
         WHERE id = ?",
    )
    .bind(until.naive_utc())
    .bind(until.naive_utc())
    .bind(subscription_id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn clear_manual_pause(pool: &MySqlPool, subscription_id: i64) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "UPDATE foster_subscription
         SET manual_pause_until = NULL
         WHERE id = ?",
    )
    .bind(subscription_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE foster_job
         SET status = 'PENDING',
             deferred_until = NULL
         WHERE subscription_id = ?
           AND status = 'DEFERRED_MANUAL'",
    )
    .bind(subscription_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn replace_quiet_periods(
    pool: &MySqlPool,
    game_account_id: i64,
    items: &[QuietPeriodRow],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "DELETE FROM foster_quiet_period
         WHERE game_account_id = ?",
    )
    .bind(game_account_id)
    .execute(&mut *tx)
    .await?;

    for item in items {
        sqlx::query(
            "INSERT INTO foster_quiet_period(
                game_account_id, weekday_mask, start_time, end_time,
                timezone, before_buffer_minutes, after_buffer_minutes, enabled
             )
             VALUES (?, ?, ?, ?, ?, ?, ?, 1)",
        )
        .bind(game_account_id)
        .bind(item.weekday_mask)
        .bind(item.start_time)
        .bind(item.end_time)
        .bind(&item.timezone)
        .bind(item.before_buffer_minutes)
        .bind(item.after_buffer_minutes)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "UPDATE foster_job j
         JOIN foster_subscription s ON s.id = j.subscription_id
         SET j.status = 'PENDING',
             j.deferred_until = NULL
         WHERE s.game_account_id = ?
           AND j.status = 'DEFERRED_QUIET'",
    )
    .bind(game_account_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}
