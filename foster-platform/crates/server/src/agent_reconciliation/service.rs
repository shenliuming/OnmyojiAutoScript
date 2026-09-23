use std::collections::HashMap;

use foster_protocol::{AgentCommandKind, AgentCommandState, AgentCommandStatus};
use sqlx::MySqlPool;

use crate::{
    enrollment::{EnrollmentError, EnrollmentService, login_command_id},
    foster_dispatch::foster_command_id,
};

use super::repository::{
    list_host_active_login_sessions, list_host_executing_foster_jobs,
    list_host_switching_foster_jobs, mark_recovery_required,
};

#[derive(Debug, thiserror::Error)]
pub enum AgentReconciliationError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Enrollment(#[from] EnrollmentError),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReconciliationReport {
    pub preserved: usize,
    pub recovery_required: usize,
    pub login_failed: usize,
    pub redispatch_job_ids: Vec<i64>,
}

#[derive(Clone)]
pub struct AgentReconciliationService {
    pool: MySqlPool,
}

impl AgentReconciliationService {
    pub fn new(pool: MySqlPool) -> Self {
        Self { pool }
    }

    pub async fn reconcile(
        &self,
        host_id: i64,
        command_states: &[AgentCommandState],
    ) -> Result<ReconciliationReport, AgentReconciliationError> {
        let mut trusted_foster = HashMap::new();
        let mut trusted_login = HashMap::new();

        for state in command_states {
            match state.kind {
                AgentCommandKind::Foster => {
                    let (Some(job_id), Some(attempt)) = (state.job_id, state.attempt) else {
                        continue;
                    };

                    if state.command_id != foster_command_id(job_id, attempt) {
                        continue;
                    }

                    trusted_foster.insert((job_id, attempt), state.status);
                }
                AgentCommandKind::Login => {
                    let Some(session_no) = state.session_no.as_deref() else {
                        continue;
                    };

                    if state.command_id != login_command_id(session_no) {
                        continue;
                    }

                    trusted_login.insert(session_no.to_string(), state.status);
                }
            }
        }

        let mut report = ReconciliationReport::default();

        for (&(job_id, attempt), &status) in &trusted_foster {
            if status == AgentCommandStatus::Interrupted
                && mark_recovery_required(
                    &self.pool,
                    host_id,
                    job_id,
                    attempt,
                    "agent restarted while foster command was running",
                )
                .await?
            {
                report.recovery_required += 1;
            }
        }

        for job in list_host_executing_foster_jobs(&self.pool, host_id).await? {
            match trusted_foster.get(&(job.id, job.retry_count)) {
                Some(AgentCommandStatus::Running | AgentCommandStatus::Finished) => {
                    report.preserved += 1;
                }
                Some(AgentCommandStatus::Interrupted) => {
                    // Already handled above.
                }
                None => {
                    if mark_recovery_required(
                        &self.pool,
                        host_id,
                        job.id,
                        job.retry_count,
                        "server expected an executing foster command but agent did not report it",
                    )
                    .await?
                    {
                        report.recovery_required += 1;
                    }
                }
            }
        }

        let enrollment = EnrollmentService::new(self.pool.clone());

        for (session_no, status) in &trusted_login {
            if *status == AgentCommandStatus::Interrupted {
                enrollment
                    .fail_login_for_recovery(
                        host_id,
                        session_no,
                        "AGENT_RECOVERY_REQUIRED: agent restarted while login command was running",
                    )
                    .await?;
                report.login_failed += 1;
            }
        }

        for session in list_host_active_login_sessions(&self.pool, host_id).await? {
            match trusted_login.get(&session.session_no) {
                Some(AgentCommandStatus::Running | AgentCommandStatus::Finished) => {
                    report.preserved += 1;
                }
                Some(AgentCommandStatus::Interrupted) => {
                    // Already failed above.
                }
                None => {
                    enrollment
                        .fail_login_for_recovery(
                            host_id,
                            &session.session_no,
                            "AGENT_RECOVERY_REQUIRED: server expected active login command but agent did not report it",
                        )
                        .await?;
                    report.login_failed += 1;
                }
            }
        }

        report.redispatch_job_ids = list_host_switching_foster_jobs(&self.pool, host_id)
            .await?
            .into_iter()
            .filter(|job| {
                !matches!(
                    trusted_foster.get(&(job.id, job.retry_count)),
                    Some(AgentCommandStatus::Running)
                )
            })
            .map(|job| job.id)
            .collect();

        Ok(report)
    }
}
