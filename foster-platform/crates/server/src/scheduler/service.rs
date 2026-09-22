use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use foster_domain::{QuietWindow, ScheduleGate, evaluate_quiet_periods};
use sqlx::MySqlPool;
use uuid::Uuid;

use super::repository::{
    clear_next_run, has_active_binding, has_executing_job_for_account,
    has_executing_job_for_emulator, has_nonterminal_job, insert_pending_job,
    list_due_subscription_ids, list_enabled_quiet_periods, lock_active_binding_for_account,
    lock_due_subscription, lock_emulator_for_claim, lock_job_for_claim, lock_job_gate_context,
    resume_job_pending, set_job_deferred, set_job_switching_account, set_job_waiting_emulator,
};

#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("invalid quiet-period timezone: {0}")]
    InvalidTimezone(String),
    #[error("foster job not found")]
    JobNotFound,
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

#[derive(Clone)]
pub struct SchedulerService {
    pool: MySqlPool,
}

impl SchedulerService {
    pub fn new(pool: MySqlPool) -> Self {
        Self { pool }
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
