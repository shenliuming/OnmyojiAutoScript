use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use foster_domain::{
    FosterErrorCode, FosterJobStatus, QuietWindow, RetryDecision, ScheduleGate,
    evaluate_quiet_periods,
};
use sqlx::MySqlPool;
use uuid::Uuid;

use super::repository::{
    clear_next_run, count_successes_between, has_active_binding, has_executing_job_for_account,
    has_executing_job_for_emulator, has_nonterminal_job, insert_pending_job,
    list_due_subscription_ids, list_enabled_quiet_periods, list_schedulable_job_ids,
    lock_active_binding_for_account, lock_due_subscription, lock_emulator_for_claim,
    lock_job_for_claim, lock_job_for_failure, lock_job_for_success, lock_job_gate_context,
    lock_subscription_for_failure, lock_subscription_for_success, mark_account_identity_mismatch,
    mark_account_relogin_required, mark_job_success, restore_subscription_schedule,
    resume_job_pending, schedule_subscription_after_success, set_job_deferred, set_job_retry,
    set_job_switching_account, set_job_terminal_failure, set_job_waiting_emulator,
    set_job_waiting_emulator_failure, suspend_subscription, transition_job_status,
};

#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("invalid quiet-period timezone: {0}")]
    InvalidTimezone(String),
    #[error("foster job not found")]
    JobNotFound,
    #[error("foster job is not running")]
    JobNotRunning,
    #[error("remaining seconds is out of range: {0}")]
    InvalidRemainingSeconds(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobGateResult {
    Executable,
    DeferredManual(DateTime<Utc>),
    DeferredQuiet(DateTime<Utc>),
    BlockedAccount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimResult {
    Claimed { emulator_id: i64 },
    WaitingEmulator,
    DeferredManual(DateTime<Utc>),
    DeferredQuiet(DateTime<Utc>),
    BlockedAccount,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SchedulerRunReport {
    pub created_job_ids: Vec<i64>,
    pub claimed_job_ids: Vec<i64>,
    pub waiting_job_ids: Vec<i64>,
    pub deferred_job_ids: Vec<i64>,
    pub blocked_job_ids: Vec<i64>,
}

#[derive(Clone)]
pub struct SchedulerService {
    pool: MySqlPool,
}

impl SchedulerService {
    pub fn new(pool: MySqlPool) -> Self {
        Self { pool }
    }

    pub async fn run_once(&self, now: DateTime<Utc>) -> Result<SchedulerRunReport, SchedulerError> {
        const DUE_LIMIT: u32 = 100;
        const RUNNABLE_LIMIT: u32 = 200;

        let created_job_ids = self.create_due_jobs(now, DUE_LIMIT).await?;
        let candidate_job_ids = list_schedulable_job_ids(&self.pool, now, RUNNABLE_LIMIT).await?;

        let mut report = SchedulerRunReport {
            created_job_ids,
            ..Default::default()
        };

        for job_id in candidate_job_ids {
            match self.claim_for_execution(job_id, now).await? {
                ClaimResult::Claimed { .. } => report.claimed_job_ids.push(job_id),
                ClaimResult::WaitingEmulator => report.waiting_job_ids.push(job_id),
                ClaimResult::DeferredManual(_) | ClaimResult::DeferredQuiet(_) => {
                    report.deferred_job_ids.push(job_id);
                }
                ClaimResult::BlockedAccount => report.blocked_job_ids.push(job_id),
            }
        }

        Ok(report)
    }

    pub async fn handle_failure(
        &self,
        job_id: i64,
        error_code: FosterErrorCode,
        result_message: &str,
        now: DateTime<Utc>,
    ) -> Result<RetryDecision, SchedulerError> {
        const MAX_UNKNOWN_RETRIES: i32 = 3;

        let mut tx = self.pool.begin().await?;

        let job = lock_job_for_failure(&mut tx, job_id)
            .await?
            .ok_or(SchedulerError::JobNotFound)?;

        if job.status != "RUNNING" {
            tx.rollback().await?;
            return Err(SchedulerError::JobNotRunning);
        }

        let subscription = lock_subscription_for_failure(&mut tx, job.subscription_id)
            .await?
            .ok_or(SchedulerError::JobNotFound)?;

        let next_retry_count = job.retry_count.saturating_add(1);
        let error_name = error_code.as_str();

        let decision = match error_code {
            FosterErrorCode::NoSlot | FosterErrorCode::ProviderNotFound => {
                let retry_at = now + chrono::Duration::minutes(1);
                set_job_retry(
                    &mut tx,
                    job.id,
                    next_retry_count,
                    retry_at,
                    error_name,
                    result_message,
                )
                .await?;
                RetryDecision::RetryAt(retry_at)
            }
            FosterErrorCode::NetworkError | FosterErrorCode::GameBusy => {
                let retry_at = now + chrono::Duration::minutes(5);
                set_job_retry(
                    &mut tx,
                    job.id,
                    next_retry_count,
                    retry_at,
                    error_name,
                    result_message,
                )
                .await?;
                RetryDecision::RetryAt(retry_at)
            }
            FosterErrorCode::EmulatorOffline => {
                set_job_waiting_emulator_failure(&mut tx, job.id, error_name, result_message)
                    .await?;
                RetryDecision::WaitForEmulator
            }
            FosterErrorCode::AccountLoginExpired => {
                set_job_terminal_failure(
                    &mut tx,
                    job.id,
                    "FAILED",
                    next_retry_count,
                    now,
                    error_name,
                    result_message,
                )
                .await?;
                suspend_subscription(&mut tx, subscription.id).await?;
                mark_account_relogin_required(&mut tx, job.game_account_id).await?;
                RetryDecision::SuspendAccount
            }
            FosterErrorCode::IdentityMismatch => {
                set_job_terminal_failure(
                    &mut tx,
                    job.id,
                    "IDENTITY_MISMATCH",
                    next_retry_count,
                    now,
                    error_name,
                    result_message,
                )
                .await?;
                suspend_subscription(&mut tx, subscription.id).await?;
                mark_account_identity_mismatch(&mut tx, job.game_account_id).await?;
                RetryDecision::SuspendAccount
            }
            FosterErrorCode::Unknown if next_retry_count < MAX_UNKNOWN_RETRIES => {
                let retry_at = now + chrono::Duration::minutes(5);
                set_job_retry(
                    &mut tx,
                    job.id,
                    next_retry_count,
                    retry_at,
                    error_name,
                    result_message,
                )
                .await?;
                RetryDecision::RetryAt(retry_at)
            }
            FosterErrorCode::Unknown => {
                set_job_terminal_failure(
                    &mut tx,
                    job.id,
                    "FAILED",
                    next_retry_count,
                    now,
                    error_name,
                    result_message,
                )
                .await?;
                let next_run_at =
                    now + chrono::Duration::minutes(i64::from(subscription.interval_minutes));
                restore_subscription_schedule(&mut tx, subscription.id, next_run_at).await?;
                RetryDecision::FailTerminal
            }
        };

        tx.commit().await?;
        Ok(decision)
    }

    pub async fn complete_success(
        &self,
        job_id: i64,
        success_at: DateTime<Utc>,
        remaining_seconds: Option<i64>,
    ) -> Result<DateTime<Utc>, SchedulerError> {
        let mut tx = self.pool.begin().await?;

        let job = lock_job_for_success(&mut tx, job_id)
            .await?
            .ok_or(SchedulerError::JobNotFound)?;

        if job.status != "RUNNING" {
            tx.rollback().await?;
            return Err(SchedulerError::JobNotRunning);
        }

        let subscription = lock_subscription_for_success(&mut tx, job.subscription_id)
            .await?
            .ok_or(SchedulerError::JobNotFound)?;

        let positive_remaining = remaining_seconds.filter(|seconds| *seconds > 0);
        let stored_remaining = match positive_remaining {
            Some(seconds) => Some(
                i32::try_from(seconds)
                    .map_err(|_| SchedulerError::InvalidRemainingSeconds(seconds))?,
            ),
            None => None,
        };

        let next_run_at = match positive_remaining {
            Some(seconds) => success_at + chrono::Duration::seconds(seconds),
            None => {
                success_at + chrono::Duration::minutes(i64::from(subscription.interval_minutes))
            }
        };

        mark_job_success(&mut tx, job.id, success_at, stored_remaining).await?;
        schedule_subscription_after_success(&mut tx, subscription.id, success_at, next_run_at)
            .await?;

        tx.commit().await?;
        Ok(next_run_at)
    }

    pub async fn count_successes_between(
        &self,
        game_account_id: i64,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<i64, SchedulerError> {
        count_successes_between(&self.pool, game_account_id, start, end)
            .await
            .map_err(SchedulerError::Database)
    }

    pub async fn transition_job(
        &self,
        job_id: i64,
        expected: FosterJobStatus,
        next: FosterJobStatus,
        now: DateTime<Utc>,
    ) -> Result<bool, SchedulerError> {
        if !is_allowed_transition(expected, next) {
            return Ok(false);
        }

        transition_job_status(
            &self.pool,
            job_id,
            foster_job_status_name(expected),
            foster_job_status_name(next),
            now,
            next.is_executing(),
            next.is_terminal(),
        )
        .await
        .map_err(SchedulerError::Database)
    }

    pub async fn gate_pending_job(
        &self,
        job_id: i64,
        now: DateTime<Utc>,
    ) -> Result<JobGateResult, SchedulerError> {
        let mut tx = self.pool.begin().await?;

        let context = lock_job_gate_context(&mut tx, job_id)
            .await?
            .ok_or(SchedulerError::JobNotFound)?;

        if context.login_status != "LOGGED_IN" || context.verify_status != "VERIFIED" {
            tx.rollback().await?;
            return Ok(JobGateResult::BlockedAccount);
        }

        if let Some(pause_until) = context.manual_pause_until {
            let pause_until = DateTime::<Utc>::from_naive_utc_and_offset(pause_until, Utc);
            if pause_until > now {
                set_job_deferred(&mut tx, context.id, "DEFERRED_MANUAL", pause_until).await?;
                tx.commit().await?;
                return Ok(JobGateResult::DeferredManual(pause_until));
            }
        }

        if matches!(
            context.status.as_str(),
            "DEFERRED_MANUAL" | "DEFERRED_QUIET"
        ) {
            resume_job_pending(&mut tx, context.id).await?;
        }

        let quiet_rows = list_enabled_quiet_periods(&mut tx, context.game_account_id).await?;

        let mut quiet_until: Option<DateTime<Utc>> = None;

        for row in quiet_rows {
            let timezone = row
                .timezone
                .parse::<Tz>()
                .map_err(|_| SchedulerError::InvalidTimezone(row.timezone.clone()))?;

            let window = QuietWindow {
                weekday_mask: row.weekday_mask,
                start_time: row.start_time,
                end_time: row.end_time,
                before_buffer_minutes: i64::from(row.before_buffer_minutes),
                after_buffer_minutes: i64::from(row.after_buffer_minutes),
            };

            if let ScheduleGate::DeferredUntil(until) =
                evaluate_quiet_periods(now, timezone, &[window])
            {
                quiet_until = Some(match quiet_until {
                    Some(current) => current.max(until),
                    None => until,
                });
            }
        }

        if let Some(until) = quiet_until {
            set_job_deferred(&mut tx, context.id, "DEFERRED_QUIET", until).await?;
            tx.commit().await?;
            return Ok(JobGateResult::DeferredQuiet(until));
        }

        resume_job_pending(&mut tx, context.id).await?;
        tx.commit().await?;

        Ok(JobGateResult::Executable)
    }

    pub async fn claim_for_execution(
        &self,
        job_id: i64,
        now: DateTime<Utc>,
    ) -> Result<ClaimResult, SchedulerError> {
        match self.gate_pending_job(job_id, now).await? {
            JobGateResult::DeferredManual(until) => {
                return Ok(ClaimResult::DeferredManual(until));
            }
            JobGateResult::DeferredQuiet(until) => {
                return Ok(ClaimResult::DeferredQuiet(until));
            }
            JobGateResult::BlockedAccount => {
                return Ok(ClaimResult::BlockedAccount);
            }
            JobGateResult::Executable => {}
        }

        let mut tx = self.pool.begin().await?;

        let job = lock_job_for_claim(&mut tx, job_id)
            .await?
            .ok_or(SchedulerError::JobNotFound)?;

        if job.status != "PENDING" {
            tx.rollback().await?;
            return Ok(ClaimResult::WaitingEmulator);
        }

        let Some(binding) = lock_active_binding_for_account(&mut tx, job.game_account_id).await?
        else {
            set_job_waiting_emulator(&mut tx, job.id, None).await?;
            tx.commit().await?;
            return Ok(ClaimResult::WaitingEmulator);
        };

        let Some(emulator) = lock_emulator_for_claim(&mut tx, binding.emulator_id).await? else {
            set_job_waiting_emulator(&mut tx, job.id, Some(binding.emulator_id)).await?;
            tx.commit().await?;
            return Ok(ClaimResult::WaitingEmulator);
        };

        let busy = emulator.host_status != "ONLINE"
            || emulator.emulator_status != "IDLE"
            || has_executing_job_for_emulator(&mut tx, emulator.id, job.id).await?
            || has_executing_job_for_account(&mut tx, job.game_account_id, job.id).await?;

        if busy {
            set_job_waiting_emulator(&mut tx, job.id, Some(emulator.id)).await?;
            tx.commit().await?;
            return Ok(ClaimResult::WaitingEmulator);
        }

        set_job_switching_account(&mut tx, job.id, emulator.id, now).await?;
        tx.commit().await?;

        Ok(ClaimResult::Claimed {
            emulator_id: emulator.id,
        })
    }

    pub async fn create_due_jobs(
        &self,
        now: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<i64>, SchedulerError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let candidates = list_due_subscription_ids(&self.pool, now, limit).await?;
        let mut created = Vec::with_capacity(candidates.len());

        for subscription_id in candidates {
            let mut tx = self.pool.begin().await?;

            let Some(subscription) = lock_due_subscription(&mut tx, subscription_id, now).await?
            else {
                tx.rollback().await?;
                continue;
            };

            if !has_active_binding(&mut tx, subscription.game_account_id).await?
                || has_nonterminal_job(&mut tx, subscription.id).await?
            {
                tx.rollback().await?;
                continue;
            }

            let job_no = format!("JOB-{}", Uuid::new_v4());
            let job_id = insert_pending_job(
                &mut tx,
                &job_no,
                subscription.id,
                subscription.game_account_id,
                subscription.next_run_at,
            )
            .await?;

            clear_next_run(&mut tx, subscription.id).await?;
            tx.commit().await?;
            created.push(job_id);
        }

        Ok(created)
    }
}

fn is_allowed_transition(expected: FosterJobStatus, next: FosterJobStatus) -> bool {
    matches!(
        (expected, next),
        (
            FosterJobStatus::SwitchingAccount,
            FosterJobStatus::VerifyingAccount
        ) | (FosterJobStatus::VerifyingAccount, FosterJobStatus::Running)
            | (FosterJobStatus::Running, FosterJobStatus::Success)
            | (FosterJobStatus::Running, FosterJobStatus::Retry)
            | (FosterJobStatus::Running, FosterJobStatus::Failed)
            | (FosterJobStatus::Running, FosterJobStatus::IdentityMismatch)
    )
}

fn foster_job_status_name(status: FosterJobStatus) -> &'static str {
    match status {
        FosterJobStatus::Pending => "PENDING",
        FosterJobStatus::DeferredQuiet => "DEFERRED_QUIET",
        FosterJobStatus::DeferredManual => "DEFERRED_MANUAL",
        FosterJobStatus::WaitingEmulator => "WAITING_EMULATOR",
        FosterJobStatus::WaitingResource => "WAITING_RESOURCE",
        FosterJobStatus::SwitchingAccount => "SWITCHING_ACCOUNT",
        FosterJobStatus::VerifyingAccount => "VERIFYING_ACCOUNT",
        FosterJobStatus::Running => "RUNNING",
        FosterJobStatus::Success => "SUCCESS",
        FosterJobStatus::Retry => "RETRY",
        FosterJobStatus::Failed => "FAILED",
        FosterJobStatus::IdentityMismatch => "IDENTITY_MISMATCH",
        FosterJobStatus::Cancelled => "CANCELLED",
        FosterJobStatus::RecoveryRequired => "RECOVERY_REQUIRED",
    }
}
