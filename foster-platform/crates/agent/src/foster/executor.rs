use async_trait::async_trait;
use chrono::{DateTime, Utc};
use foster_domain::FosterErrorCode;
use foster_protocol::{
    ExecuteFosterCommand, FosterDetectedIdentity, FosterStage,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FosterStageCheckpoint {
    pub stage: FosterStage,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct FosterExecution {
    pub stages: Vec<FosterStageCheckpoint>,
    pub completed_at: DateTime<Utc>,
    pub success: bool,
    pub error_code: Option<FosterErrorCode>,
    pub message: String,
    pub remaining_seconds: Option<i64>,
    pub screenshot_url: Option<String>,
    pub detected_identity: FosterDetectedIdentity,
}

#[derive(Debug, thiserror::Error)]
pub enum FosterExecutorError {
    #[error("{0}")]
    Message(String),
}

#[async_trait]
pub trait FosterExecutor: Send + Sync + 'static {
    async fn execute(
        &self,
        command: &ExecuteFosterCommand,
    ) -> Result<FosterExecution, FosterExecutorError>;
}
