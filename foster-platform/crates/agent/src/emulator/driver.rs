use async_trait::async_trait;
use foster_protocol::EmulatorDescriptor;

#[derive(Debug, thiserror::Error)]
pub enum EmulatorDriverError {
    #[error("unknown emulator instance: {0}")]
    UnknownInstance(String),
    #[error("{0}")]
    Message(String),
}

#[async_trait]
pub trait EmulatorDriver: Send + Sync + 'static {
    async fn list_instances(&self) -> Result<Vec<EmulatorDescriptor>, EmulatorDriverError>;

    async fn start(&self, instance_id: &str) -> Result<(), EmulatorDriverError>;

    async fn stop(&self, instance_id: &str) -> Result<(), EmulatorDriverError>;

    async fn adb_serial(&self, instance_id: &str) -> Result<Option<String>, EmulatorDriverError>;

    async fn screenshot(&self, instance_id: &str) -> Result<Vec<u8>, EmulatorDriverError>;
}
