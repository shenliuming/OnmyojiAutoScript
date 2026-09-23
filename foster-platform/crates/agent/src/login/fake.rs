use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use foster_protocol::StartLoginCommand;
use tokio::sync::Mutex;

use super::{LoginExecutor, LoginExecutorError, LoginIdentity, LoginPrepared};

#[derive(Debug, Clone)]
pub struct FakeLoginScenario {
    pub qr_payload: String,
    pub qr_ttl: Duration,
    pub identity_delay: Duration,
    pub masked_account: Option<String>,
    pub character_name: Option<String>,
    pub server_name: Option<String>,
    pub game_uid: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FakeLoginExecutor {
    scenario: FakeLoginScenario,
    cancelled: Arc<Mutex<HashSet<String>>>,
    prepare_count: Arc<AtomicUsize>,
}

impl FakeLoginExecutor {
    pub fn new(scenario: FakeLoginScenario) -> Self {
        Self {
            scenario,
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            prepare_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub async fn was_cancelled(&self, session_no: &str) -> bool {
        self.cancelled.lock().await.contains(session_no)
    }

    pub fn prepare_count(&self) -> usize {
        self.prepare_count.load(Ordering::SeqCst)
    }

    async fn ensure_not_cancelled(&self, session_no: &str) -> Result<(), LoginExecutorError> {
        if self.was_cancelled(session_no).await {
            Err(LoginExecutorError::Message("login cancelled".into()))
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl LoginExecutor for FakeLoginExecutor {
    async fn prepare(
        &self,
        command: &StartLoginCommand,
    ) -> Result<LoginPrepared, LoginExecutorError> {
        self.prepare_count.fetch_add(1, Ordering::SeqCst);
        self.ensure_not_cancelled(&command.session_no).await?;

        Ok(LoginPrepared {
            qr_payload: self.scenario.qr_payload.clone(),
            qr_ttl: self.scenario.qr_ttl,
        })
    }

    async fn wait_identity(
        &self,
        command: &StartLoginCommand,
    ) -> Result<LoginIdentity, LoginExecutorError> {
        tokio::time::sleep(self.scenario.identity_delay).await;
        self.ensure_not_cancelled(&command.session_no).await?;

        Ok(LoginIdentity {
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
