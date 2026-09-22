use foster_domain::FosterErrorCode;
use foster_protocol::{
    AgentEvent, FosterFailed, FosterStageChanged, FosterSucceeded,
};

use super::FosterExecution;

pub fn events_for_execution(job_id: i64, attempt: i32, execution: FosterExecution) -> Vec<AgentEvent> {
    let mut events = Vec::with_capacity(execution.stages.len() + 1);

    for checkpoint in execution.stages {
        events.push(AgentEvent::FosterStageChanged(FosterStageChanged {
            job_id,
            attempt,
            stage: checkpoint.stage,
            occurred_at: checkpoint.occurred_at,
        }));
    }

    if execution.success {
        events.push(AgentEvent::FosterSucceeded(FosterSucceeded {
            job_id,
            attempt,
            completed_at: execution.completed_at,
            remaining_seconds: execution.remaining_seconds,
            screenshot_url: execution.screenshot_url,
            detected_identity: execution.detected_identity,
        }));
    } else {
        events.push(AgentEvent::FosterFailed(FosterFailed {
            job_id,
            attempt,
            failed_at: execution.completed_at,
            error_code: execution.error_code.unwrap_or(FosterErrorCode::Unknown),
            message: execution.message,
            screenshot_url: execution.screenshot_url,
        }));
    }

    events
}
