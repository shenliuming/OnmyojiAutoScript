use std::{
    collections::HashSet,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use foster_protocol::StartLoginCommand;
use tokio::sync::Mutex;

use super::{LoginExecution, LoginExecutor, LoginExecutorError};

#[derive(Debug, Clone)]
pub struct FakeLoginScenario {
    pub qr_payload: String,
    pub qr_ttl: Duration,
    pub masked_account: Option<String>,
    pub character_name: Option<String>,
    pub server_name: Option<String>,
    pub game_uid: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FakeLoginExecutor {
    scenario: FakeLoginScenario,
    cancelled: Arc<Mutex<HashSet<String>>>,
}

impl FakeLoginExecutor {
    pub fn new(scenario: FakeLoginScenario) -> Self {
        Self {
            scenario,
            cancelled: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub async fn was_cancelled(&self, session_no: &str) -> bool {
        self.cancelled.lock().await.contains(session_no)
    }
}

#[async_trait]
impl LoginExecutor for FakeLoginExecutor {
    async fn execute(
        &self,
        _command: &StartLoginCommand,
    ) -> Result<LoginExecution, LoginExecutorError> {
        Ok(LoginExecution {
            qr_payload: self.scenario.qr_payload.clone(),
            qr_ttl: self.scenario.qr_ttl,
            masked_account: self.scenario.masked_account.clone(),
            character_name: self.scenario.character_name.clone(),
            server_name: self.scenario.server_name.clone(),
            game_uid: self.scenario.game_uid.clone(),
        })
    }

    async fn cancel(&self, session_no: &str) -> Result<(), LoginExecutorError> {
        self.cancelled.lock().await.insert(session_no.to_string());
        Ok(())
    }
}
