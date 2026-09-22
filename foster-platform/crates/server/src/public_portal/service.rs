use std::fmt::Write as _;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Asia::Shanghai;
use foster_domain::{QuietWindow, ScheduleGate, evaluate_quiet_periods};
use rand::RngCore;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::MySqlPool;

use super::repository::{
    QuietPeriodRow, RecentJobRow, count_successes_between,
    clear_manual_pause, insert_share_link, load_portal_subscription, load_quiet_periods,
    load_recent_jobs, load_share_by_control_hash, load_share_by_public_hash, replace_quiet_periods,
    revoke_active_links, set_manual_pause_until, touch_share_link,
};

#[derive(Debug, thiserror::Error)]
pub enum PublicPortalError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("share link not found")]
    NotFound,
    #[error("share link expired")]
    Expired,
    #[error("invalid pause preset")]
    InvalidPausePreset,
    #[error("invalid quiet period")]
    InvalidQuietPeriod,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedShareLink {
    pub public_token: String,
    pub control_token: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublicQuietPeriod {
    pub weekday_mask: i32,
    pub start_time: String,
    pub end_time: String,
    pub timezone: String,
    pub before_buffer_minutes: i32,
    pub after_buffer_minutes: i32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublicJob {
    pub status: String,
    pub scheduled_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub result_message: Option<String>,
    pub screenshot_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicServiceStatus {
    pub subscription_no: String,
    pub service_status: String,
    pub character_name: Option<String>,
    pub server_name: Option<String>,
    pub login_status: String,
    pub verify_status: String,
    pub plan_name: String,
    pub resource_mode: String,
    pub resource_type: Option<String>,
    pub daily_target_runs: i32,
    pub today_success_count: i64,
    pub last_success_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub manual_pause_until: Option<DateTime<Utc>>,
    pub effective_blocked_until: Option<DateTime<Utc>>,
    pub block_reason: Option<String>,
    pub service_end_at: DateTime<Utc>,
    pub relogin_required: bool,
    pub quiet_periods: Vec<PublicQuietPeriod>,
    pub recent_jobs: Vec<PublicJob>,
}

#[derive(Debug, Clone)]
pub struct QuietPeriodInput {
    pub weekday_mask: i32,
    pub start_time: String,
    pub end_time: String,
    pub before_buffer_minutes: i32,
    pub after_buffer_minutes: i32,
}

#[derive(Clone)]
pub struct PublicPortalService {
    pool: MySqlPool,
}

impl PublicPortalService {
    pub fn new(pool: MySqlPool) -> Self {
        Self { pool }
    }

    pub async fn rotate_share_link(
        &self,
        subscription_id: i64,
        expire_at: Option<DateTime<Utc>>,
    ) -> Result<CreatedShareLink, PublicPortalError> {
        let public_token = random_token();
        let control_token = random_token();
        let public_hash = sha256_hex(&public_token);
        let control_hash = sha256_hex(&control_token);

        let mut tx = self.pool.begin().await?;
        revoke_active_links(&mut tx, subscription_id).await?;
        insert_share_link(
            &mut tx,
            subscription_id,
            &public_hash,
            &control_hash,
            expire_at,
        )
        .await?;
        tx.commit().await?;

        Ok(CreatedShareLink {
            public_token,
            control_token,
        })
    }

    pub async fn load_public_status(
        &self,
        public_token: &str,
        now: DateTime<Utc>,
    ) -> Result<PublicServiceStatus, PublicPortalError> {
        let share = load_share_by_public_hash(&self.pool, &sha256_hex(public_token))
            .await?
            .ok_or(PublicPortalError::NotFound)?;
        validate_share(&share.status, share.expire_at, now)?;

        let status = self
            .build_status(share.subscription_id, now)
            .await?;
        touch_share_link(&self.pool, share.id, now).await?;
        Ok(status)
    }

    pub async fn pause(
        &self,
        control_token: &str,
        preset: &str,
        now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, PublicPortalError> {
        let share = self.resolve_control(control_token, now).await?;
        let until = pause_until(preset, now)?;
        set_manual_pause_until(&self.pool, share.subscription_id, until).await?;

        let row = load_portal_subscription(&self.pool, share.subscription_id)
            .await?
            .ok_or(PublicPortalError::NotFound)?;
        Ok(row
            .manual_pause_until
            .map(to_utc)
            .unwrap_or(until))
    }

    pub async fn clear_pause(
        &self,
        control_token: &str,
        now: DateTime<Utc>,
    ) -> Result<(), PublicPortalError> {
        let share = self.resolve_control(control_token, now).await?;
        clear_manual_pause(&self.pool, share.subscription_id).await?;
        Ok(())
    }

    pub async fn replace_quiet_periods(
        &self,
        control_token: &str,
        inputs: Vec<QuietPeriodInput>,
        now: DateTime<Utc>,
    ) -> Result<Vec<PublicQuietPeriod>, PublicPortalError> {
        if inputs.len() > 8 {
            return Err(PublicPortalError::InvalidQuietPeriod);
        }

        let share = self.resolve_control(control_token, now).await?;
        let subscription = load_portal_subscription(&self.pool, share.subscription_id)
            .await?
            .ok_or(PublicPortalError::NotFound)?;

        let mut rows = Vec::with_capacity(inputs.len());
        for input in inputs {
            if !(1..=127).contains(&input.weekday_mask)
                || !(0..=120).contains(&input.before_buffer_minutes)
                || !(0..=120).contains(&input.after_buffer_minutes)
            {
                return Err(PublicPortalError::InvalidQuietPeriod);
            }

            let start_time = NaiveTime::parse_from_str(&input.start_time, "%H:%M")
                .map_err(|_| PublicPortalError::InvalidQuietPeriod)?;
            let end_time = NaiveTime::parse_from_str(&input.end_time, "%H:%M")
                .map_err(|_| PublicPortalError::InvalidQuietPeriod)?;

            rows.push(QuietPeriodRow {
                weekday_mask: input.weekday_mask,
                start_time,
                end_time,
                timezone: "Asia/Shanghai".to_string(),
                before_buffer_minutes: input.before_buffer_minutes,
                after_buffer_minutes: input.after_buffer_minutes,
            });
        }

        replace_quiet_periods(&self.pool, subscription.game_account_id, &rows).await?;

        Ok(rows.iter().map(public_quiet_period).collect())
    }

    async fn resolve_control(
        &self,
        control_token: &str,
        now: DateTime<Utc>,
    ) -> Result<super::repository::ShareLinkRow, PublicPortalError> {
        let share = load_share_by_control_hash(&self.pool, &sha256_hex(control_token))
            .await?
            .ok_or(PublicPortalError::NotFound)?;
        validate_share(&share.status, share.expire_at, now)?;
        Ok(share)
    }

    async fn build_status(
        &self,
        subscription_id: i64,
        now: DateTime<Utc>,
    ) -> Result<PublicServiceStatus, PublicPortalError> {
        let row = load_portal_subscription(&self.pool, subscription_id)
            .await?
            .ok_or(PublicPortalError::NotFound)?;
        let quiet_rows = load_quiet_periods(&self.pool, row.game_account_id).await?;
        let jobs = load_recent_jobs(&self.pool, subscription_id, 20).await?;

        let local_now = now.with_timezone(&Shanghai);
        let local_date = local_now.date_naive();
        let start_local = Shanghai
            .from_local_datetime(
                &local_date
                    .and_hms_opt(0, 0, 0)
                    .expect("midnight is valid"),
            )
            .single()
            .expect("Asia/Shanghai midnight is unique");
        let next_date = local_date.succ_opt().expect("valid next day");
        let end_local = Shanghai
            .from_local_datetime(
                &next_date
                    .and_hms_opt(0, 0, 0)
                    .expect("midnight is valid"),
            )
            .single()
            .expect("Asia/Shanghai midnight is unique");

        let today_success_count = count_successes_between(
            &self.pool,
            row.game_account_id,
            start_local.with_timezone(&Utc),
            end_local.with_timezone(&Utc),
        )
        .await?;

        let manual_pause_until = row.manual_pause_until.map(to_utc);
        let quiet_until = current_quiet_until(now, &quiet_rows);
        let (effective_blocked_until, block_reason) = match (manual_pause_until, quiet_until) {
            (Some(manual), Some(quiet)) if manual >= quiet => {
                (Some(manual), Some("MANUAL_PAUSE".to_string()))
            }
            (Some(_), Some(quiet)) => (Some(quiet), Some("QUIET_PERIOD".to_string())),
            (Some(manual), None) if manual > now => {
                (Some(manual), Some("MANUAL_PAUSE".to_string()))
            }
            (_, Some(quiet)) => (Some(quiet), Some("QUIET_PERIOD".to_string())),
            _ => (None, None),
        };

        Ok(PublicServiceStatus {
            subscription_no: row.subscription_no,
            service_status: row.service_status,
            character_name: row.character_name,
            server_name: row.server_name,
            login_status: row.login_status.clone(),
            verify_status: row.verify_status,
            plan_name: row.plan_name,
            resource_mode: row.resource_mode,
            resource_type: row.resource_type,
            daily_target_runs: row.daily_target_runs,
            today_success_count,
            last_success_at: row.last_success_at.map(to_utc),
            next_run_at: row.next_run_at.map(to_utc),
            manual_pause_until,
            effective_blocked_until,
            block_reason,
            service_end_at: to_utc(row.service_end_at),
            relogin_required: row.login_status == "RELOGIN_REQUIRED",
            quiet_periods: quiet_rows.iter().map(public_quiet_period).collect(),
            recent_jobs: jobs.into_iter().map(public_job).collect(),
        })
    }
}

fn validate_share(
    status: &str,
    expire_at: Option<chrono::NaiveDateTime>,
    now: DateTime<Utc>,
) -> Result<(), PublicPortalError> {
    if status != "ACTIVE" {
        return Err(PublicPortalError::NotFound);
    }
    if expire_at.is_some_and(|value| to_utc(value) <= now) {
        return Err(PublicPortalError::Expired);
    }
    Ok(())
}

fn pause_until(preset: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>, PublicPortalError> {
    match preset {
        "1H" => Ok(now + chrono::Duration::hours(1)),
        "2H" => Ok(now + chrono::Duration::hours(2)),
        "4H" => Ok(now + chrono::Duration::hours(4)),
        "TODAY" => {
            let local = now.with_timezone(&Shanghai);
            let tomorrow = local
                .date_naive()
                .succ_opt()
                .ok_or(PublicPortalError::InvalidPausePreset)?;
            Ok(Shanghai
                .from_local_datetime(
                    &tomorrow
                        .and_hms_opt(0, 0, 0)
                        .ok_or(PublicPortalError::InvalidPausePreset)?,
                )
                .single()
                .ok_or(PublicPortalError::InvalidPausePreset)?
                .with_timezone(&Utc))
        }
        _ => Err(PublicPortalError::InvalidPausePreset),
    }
}

fn current_quiet_until(
    now: DateTime<Utc>,
    rows: &[QuietPeriodRow],
) -> Option<DateTime<Utc>> {
    let mut until = None;
    for row in rows {
        let window = QuietWindow {
            weekday_mask: row.weekday_mask as u8,
            start_time: row.start_time,
            end_time: row.end_time,
            before_buffer_minutes: i64::from(row.before_buffer_minutes),
            after_buffer_minutes: i64::from(row.after_buffer_minutes),
        };
        if let ScheduleGate::DeferredUntil(value) =
            evaluate_quiet_periods(now, Shanghai, &[window])
        {
            until = Some(until.map_or(value, |current: DateTime<Utc>| current.max(value)));
        }
    }
    until
}

fn public_quiet_period(row: &QuietPeriodRow) -> PublicQuietPeriod {
    PublicQuietPeriod {
        weekday_mask: row.weekday_mask,
        start_time: row.start_time.format("%H:%M").to_string(),
        end_time: row.end_time.format("%H:%M").to_string(),
        timezone: row.timezone.clone(),
        before_buffer_minutes: row.before_buffer_minutes,
        after_buffer_minutes: row.after_buffer_minutes,
    }
}

fn public_job(row: RecentJobRow) -> PublicJob {
    PublicJob {
        status: row.status,
        scheduled_at: to_utc(row.scheduled_at),
        started_at: row.started_at.map(to_utc),
        finished_at: row.finished_at.map(to_utc),
        result_message: row.result_message,
        screenshot_url: row.screenshot_url,
    }
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing into String cannot fail");
    }
    output
}

fn to_utc(value: chrono::NaiveDateTime) -> DateTime<Utc> {
    DateTime::from_naive_utc_and_offset(value, Utc)
}
