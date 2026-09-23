use std::collections::HashMap;

use foster_protocol::{
    AgentCommandKind, AgentCommandState, AgentCommandStatus,
};
use sqlx::MySqlPool;

use crate::foster_dispatch::foster_command_id;

use super::repository::{
    list_host_executing_foster_jobs, mark_recovery_required,
};

#[derive(Debug, thiserror::Error)]
pub enum AgentReconciliationError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReconciliationReport {
    pub preserved: usize,
    pub recovery_required: usize,
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
        let mut trusted = HashMap::new();

        for state in command_states {
            if state.kind != AgentCommandKind::Foster {
                continue;
            }

            let (Some(job_id), Some(attempt)) = (state.job_id, state.attempt) else {
                continue;
            };

            if state.command_id != foster_command_id(job_id, attempt) {
                continue;
            }

            trusted.insert((job_id, attempt), state.status);
        }

        let mut report = ReconciliationReport::default();

        for (&(job_id, attempt), &status) in &trusted {
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
            match trusted.get(&(job.id, job.retry_count)) {
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

        Ok(report)
    }
}
