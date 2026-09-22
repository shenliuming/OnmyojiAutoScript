use async_trait::async_trait;
use foster_protocol::StartLoginCommand;

#[derive(Debug, Clone)]
pub struct LoginExecution {
    pub qr_payload: String,
    pub qr_ttl: std::time::Duration,
    pub masked_account: Option<String>,
    pub character_name: Option<String>,
    pub server_name: Option<String>,
    pub game_uid: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LoginExecutorError {
    #[error("{0}")]
    Message(String),
}

#[async_trait]
pub trait LoginExecutor: Send + Sync + 'static {
    async fn execute(
        &self,
        command: &StartLoginCommand,
    ) -> Result<LoginExecution, LoginExecutorError>;

    async fn cancel(&self, session_no: &str) -> Result<(), LoginExecutorError>;
}
