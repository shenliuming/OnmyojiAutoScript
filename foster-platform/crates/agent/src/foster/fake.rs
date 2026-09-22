use async_trait::async_trait;
use foster_protocol::ExecuteFosterCommand;

use super::{FosterExecution, FosterExecutor, FosterExecutorError};

#[derive(Debug, Clone)]
pub struct FakeFosterExecutor {
    execution: FosterExecution,
}

impl FakeFosterExecutor {
    pub fn new(execution: FosterExecution) -> Self {
        Self { execution }
    }
}

#[async_trait]
impl FosterExecutor for FakeFosterExecutor {
    async fn execute(
        &self,
        _command: &ExecuteFosterCommand,
    ) -> Result<FosterExecution, FosterExecutorError> {
        Ok(self.execution.clone())
    }
}
