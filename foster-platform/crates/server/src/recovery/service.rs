use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{MySql, MySqlPool, Transaction};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecoveryAction {
    ConfirmSucceeded,
    ConfirmNotExecuted,
}

impl RecoveryAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::ConfirmSucceeded => "CONFIRM_SUCCEEDED",
            Self::ConfirmNotExecuted => "CONFIRM_NOT_EXECUTED",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveRecoveryRequest {
    pub expected_attempt: i32,
    pub action: RecoveryAction,
    pub operator: String,
    pub note: String,
    pub observed_remaining_seconds: Option<i32>,
    #[serde(default)]
    pub confirmed_stopped: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryJobView {
    pub job_id: i64,
    pub job_no: String,
    pub attempt: i32,
    pub game_account_id: i64,
    pub emulator_id: Option<i64>,
    pub host_id: Option<i64>,
    pub resource_mode: String,
    pub allocation_id: Option<i64>,
    pub allocation_status: Option<String>,
    pub resource_cycle_id: Option<i64>,
    pub started_at: Option<NaiveDateTime>,
    pub error_code: Option<String>,
    pub result_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryResolutionResult {
    pub job_id: i64,
    pub status: String,
    pub next_run_at: Option<DateTime<Utc>>,
    pub retry_after: Option<DateTime<Utc>>,
    pub allocation_status: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("foster job not found")]
    JobNotFound,
    #[error("job is not awaiting recovery")]
    StateConflict,
    #[error("expected attempt does not match")]
    AttemptMismatch,
    #[error("operator is required and must be at most 64 characters")]
    InvalidOperator,
    #[error("reason is required and must be 5 to 255 characters")]
    InvalidNote,
    #[error("old executor must be confirmed stopped before retry")]
    MustConfirmStopped,
    #[error("a positive observed remaining duration is required for platform success")]
    RemainingRequired,
    #[error("observed remaining duration must be between 1 and 86400 seconds")]
    InvalidRemaining,
    #[error("platform resource allocation is no longer available for verification")]
    AllocationUnavailable,
    #[error("subscription is not available")]
    SubscriptionUnavailable,
}

#[derive(Debug, sqlx::FromRow)]
struct LockedJob {
    id: i64,
    subscription_id: i64,
    status: String,
    retry_count: i32,
    resource_allocation_id: Option<i64>,
}

#[derive(Debug, sqlx::FromRow)]
struct LockedSubscription {
    id: i64,
    resource_mode: String,
    interval_minutes: i32,
    status: String,
}

#[derive(Debug, sqlx::FromRow)]
struct LockedAllocation {
    id: i64,
    resource_cycle_id: i64,
    status: String,
}

#[derive(Clone)]
pub struct RecoveryService {
    pool: MySqlPool,
}

impl RecoveryService {
    pub fn new(pool: MySqlPool) -> Self {
        Self { pool }
    }

    pub async fn list(&self, limit: u32) -> Result<Vec<RecoveryJobView>, RecoveryError> {
        let rows = sqlx::query_as::<_, RecoveryJobView>(
            "SELECT j.id AS job_id, j.job_no, j.retry_count AS attempt,
                    j.game_account_id, j.emulator_id, e.host_id,
                    s.resource_mode, a.id AS allocation_id,
                    a.status AS allocation_status, a.resource_cycle_id,
                    j.started_at, j.error_code, j.result_message
             FROM foster_job j
             JOIN foster_subscription s ON s.id = j.subscription_id
             LEFT JOIN emulator_instance e ON e.id = j.emulator_id
             LEFT JOIN foster_resource_allocation a ON a.id = j.resource_allocation_id
             WHERE j.status = 'RECOVERY_REQUIRED'
             ORDER BY j.updated_at ASC, j.id ASC
             LIMIT ?",
        )
        .bind(i64::from(limit.clamp(1, 100)))
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn resolve(
        &self,
        job_id: i64,
        request: ResolveRecoveryRequest,
        now: DateTime<Utc>,
    ) -> Result<RecoveryResolutionResult, RecoveryError> {
        let operator = request.operator.trim();
        if operator.is_empty() || operator.chars().count() > 64 {
            return Err(RecoveryError::InvalidOperator);
        }
        let note = request.note.trim();
        if note.chars().count() < 5 || note.chars().count() > 255 {
            return Err(RecoveryError::InvalidNote);
        }
        if request.action == RecoveryAction::ConfirmNotExecuted && !request.confirmed_stopped {
            return Err(RecoveryError::MustConfirmStopped);
        }
        if request
            .observed_remaining_seconds
            .is_some_and(|seconds| !(1..=86_400).contains(&seconds))
        {
            return Err(RecoveryError::InvalidRemaining);
        }

        let mut tx = self.pool.begin().await?;
        let job = sqlx::query_as::<_, LockedJob>(
            "SELECT id, subscription_id, status, retry_count, resource_allocation_id
             FROM foster_job WHERE id = ? FOR UPDATE",
        )
        .bind(job_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RecoveryError::JobNotFound)?;

        if job.status != "RECOVERY_REQUIRED" {
            return Err(RecoveryError::StateConflict);
        }
        if job.retry_count != request.expected_attempt {
            return Err(RecoveryError::AttemptMismatch);
        }

        let subscription = sqlx::query_as::<_, LockedSubscription>(
            "SELECT id, resource_mode, interval_minutes, status
             FROM foster_subscription WHERE id = ? FOR UPDATE",
        )
        .bind(job.subscription_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RecoveryError::SubscriptionUnavailable)?;

        let allocation = match job.resource_allocation_id {
            Some(id) => {
                sqlx::query_as::<_, LockedAllocation>(
                    "SELECT id, resource_cycle_id, status
                     FROM foster_resource_allocation
                     WHERE id = ? AND job_id = ? FOR UPDATE",
                )
                .bind(id)
                .bind(job.id)
                .fetch_optional(&mut *tx)
                .await?
            }
            None => None,
        };

        let platform = subscription.resource_mode == "PLATFORM";
        if platform && allocation.is_none() {
            return Err(RecoveryError::AllocationUnavailable);
        }

        let result = match request.action {
            RecoveryAction::ConfirmSucceeded => {
                self.confirm_success(
                    &mut tx,
                    &job,
                    &subscription,
                    allocation.as_ref(),
                    request.observed_remaining_seconds,
                    now,
                )
                .await?
            }
            RecoveryAction::ConfirmNotExecuted => {
                self.confirm_not_executed(&mut tx, &job, allocation.as_ref(), now)
                    .await?
            }
        };

        sqlx::query(
            "INSERT INTO foster_recovery_audit(
                job_id, expected_attempt, resolution,
                operator_name, operator_note, observed_remaining_seconds,
                prior_allocation_id, resolved_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(job.id)
        .bind(job.retry_count)
        .bind(request.action.as_str())
        .bind(operator)
        .bind(note)
        .bind(request.observed_remaining_seconds)
        .bind(job.resource_allocation_id)
        .bind(now.naive_utc())
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(result)
    }

    async fn confirm_success(
        &self,
        tx: &mut Transaction<'_, MySql>,
        job: &LockedJob,
        subscription: &LockedSubscription,
        allocation: Option<&LockedAllocation>,
        remaining: Option<i32>,
        now: DateTime<Utc>,
    ) -> Result<RecoveryResolutionResult, RecoveryError> {
        if subscription.resource_mode == "PLATFORM" && remaining.is_none() {
            return Err(RecoveryError::RemainingRequired);
        }

        let duration = remaining
            .map(i64::from)
            .unwrap_or_else(|| i64::from(subscription.interval_minutes) * 60);
        let next_run_at = now + chrono::Duration::seconds(duration);
        let mut allocation_status = None;

        if subscription.resource_mode == "PLATFORM" {
            let allocation = allocation.ok_or(RecoveryError::AllocationUnavailable)?;
            if allocation.status != "RESERVED" && allocation.status != "CONFIRMED" {
                return Err(RecoveryError::AllocationUnavailable);
            }

            let cycle_end: NaiveDateTime = sqlx::query_scalar(
                "SELECT end_at FROM foster_resource_cycle
                 WHERE id = ? FOR UPDATE",
            )
            .bind(allocation.resource_cycle_id)
            .fetch_one(&mut **tx)
            .await?;

            if cycle_end <= now.naive_utc() {
                return Err(RecoveryError::AllocationUnavailable);
            }
            let occupied_until = next_run_at.naive_utc().min(cycle_end);

            sqlx::query(
                "UPDATE foster_resource_allocation
                 SET status = 'CONFIRMED', confirmed_at = ?,
                     occupied_until = ?
                 WHERE id = ? AND status IN ('RESERVED', 'CONFIRMED')",
            )
            .bind(now.naive_utc())
            .bind(occupied_until)
            .bind(allocation.id)
            .execute(&mut **tx)
            .await?;

            allocation_status = Some("CONFIRMED".to_string());
        }

        sqlx::query(
            "UPDATE foster_job
             SET status = 'SUCCESS', finished_at = ?,
                 remaining_seconds = ?, retry_after = NULL,
                 error_code = NULL,
                 result_message = 'manually verified foster success'
             WHERE id = ? AND status = 'RECOVERY_REQUIRED'",
        )
        .bind(now.naive_utc())
        .bind(remaining)
        .bind(job.id)
        .execute(&mut **tx)
        .await?;

        let scheduled_at = (subscription.status == "ACTIVE").then_some(next_run_at);
        sqlx::query(
            "UPDATE foster_subscription
             SET last_success_at = ?, next_run_at = ?
             WHERE id = ?",
        )
        .bind(now.naive_utc())
        .bind(scheduled_at.map(|value| value.naive_utc()))
        .bind(subscription.id)
        .execute(&mut **tx)
        .await?;

        Ok(RecoveryResolutionResult {
            job_id: job.id,
            status: "SUCCESS".to_string(),
            next_run_at: scheduled_at,
            retry_after: None,
            allocation_status,
        })
    }

    async fn confirm_not_executed(
        &self,
        tx: &mut Transaction<'_, MySql>,
        job: &LockedJob,
        allocation: Option<&LockedAllocation>,
        now: DateTime<Utc>,
    ) -> Result<RecoveryResolutionResult, RecoveryError> {
        if let Some(allocation) = allocation {
            if allocation.status != "RESERVED" {
                return Err(RecoveryError::AllocationUnavailable);
            }

            sqlx::query(
                "UPDATE foster_resource_allocation
                 SET status = 'RELEASED', released_at = ?,
                     release_reason = 'operator verified no foster execution'
                 WHERE id = ? AND status = 'RESERVED'",
            )
            .bind(now.naive_utc())
            .bind(allocation.id)
            .execute(&mut **tx)
            .await?;

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
            .bind(allocation.resource_cycle_id)
            .execute(&mut **tx)
            .await?;
        }

        let retry_after = now + chrono::Duration::minutes(5);
        sqlx::query(
            "UPDATE foster_job
             SET status = 'RETRY', retry_count = retry_count + 1,
                 retry_after = ?, started_at = NULL, finished_at = NULL,
                 error_code = NULL,
                 result_message = 'operator verified no foster execution',
                 provider_account_id = NULL, resource_cycle_id = NULL,
                 resource_allocation_id = NULL
             WHERE id = ? AND status = 'RECOVERY_REQUIRED'",
        )
        .bind(retry_after.naive_utc())
        .bind(job.id)
        .execute(&mut **tx)
        .await?;

        Ok(RecoveryResolutionResult {
            job_id: job.id,
            status: "RETRY".to_string(),
            next_run_at: None,
            retry_after: Some(retry_after),
            allocation_status: allocation.map(|_| "RELEASED".to_string()),
        })
    }
}
