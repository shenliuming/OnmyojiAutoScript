use std::{
    collections::HashSet,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use foster_protocol::StartLoginCommand;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::emulator::{CommandRunner, GenericAdbEmulatorDriver};

use super::{
    LoginExecutor, LoginExecutorError, LoginIdentity, LoginPrepared,
};

#[derive(Clone)]
pub struct HttpOasLoginExecutor<R>
where
    R: CommandRunner,
{
    driver: GenericAdbEmulatorDriver<R>,
    client: reqwest::Client,
    base_url: String,
    qr_ttl: Duration,
    identity_timeout: Duration,
    poll_interval: Duration,
    cancelled: Arc<Mutex<HashSet<String>>>,
}

impl<R> HttpOasLoginExecutor<R>
where
    R: CommandRunner,
{
    pub fn new(
        driver: GenericAdbEmulatorDriver<R>,
        base_url: String,
        qr_ttl: Duration,
        identity_timeout: Duration,
        poll_interval: Duration,
    ) -> Self {
        Self {
            driver,
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            qr_ttl,
            identity_timeout,
            poll_interval,
            cancelled: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    async fn is_cancelled(&self, session_no: &str) -> bool {
        self.cancelled.lock().await.contains(session_no)
    }
}

#[derive(Debug, Serialize)]
struct DetectLoginRequest {
    config_name: String,
}

#[derive(Debug, Deserialize)]
struct DetectLoginResponse {
    ready: bool,
    ambiguous: bool,
    message: String,
    masked_account: Option<String>,
    character_name: Option<String>,
    server_name: Option<String>,
    game_uid: Option<String>,
}

#[async_trait]
impl<R> LoginExecutor for HttpOasLoginExecutor<R>
where
    R: CommandRunner,
{
    async fn prepare(
        &self,
        command: &StartLoginCommand,
    ) -> Result<LoginPrepared, LoginExecutorError> {
        if self.is_cancelled(&command.session_no).await {
            return Err(LoginExecutorError::Message("login cancelled".into()));
        }

        let png = self
            .driver
            .prepare_login_screen(&command.emulator_code)
            .await
            .map_err(|error| LoginExecutorError::Message(error.to_string()))?;

        let qr_payload = format!(
            "data:image/png;base64,{}",
            STANDARD.encode(png)
        );

        Ok(LoginPrepared {
            qr_payload,
            qr_ttl: self.qr_ttl,
        })
    }

    async fn wait_identity(
        &self,
        command: &StartLoginCommand,
    ) -> Result<LoginIdentity, LoginExecutorError> {
        let config = self
            .driver
            .instance_config(&command.emulator_code)
            .map_err(|error| LoginExecutorError::Message(error.to_string()))?;
        let deadline = tokio::time::Instant::now() + self.identity_timeout;

        loop {
            if self.is_cancelled(&command.session_no).await {
                return Err(LoginExecutorError::Message("login cancelled".into()));
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(LoginExecutorError::Message(
                    "timed out waiting for game identity after QR scan".into(),
                ));
            }

            let response = self
                .client
                .post(format!("{}/login/detect", self.base_url))
                .json(&DetectLoginRequest {
                    config_name: config.oas_config_name.clone(),
                })
                .send()
                .await;

            if let Ok(response) = response
                && response.status().is_success()
                && let Ok(detected) = response.json::<DetectLoginResponse>().await
            {
                if detected.ready {
                    return Ok(LoginIdentity {
                        masked_account: detected.masked_account,
                        character_name: detected.character_name,
                        server_name: detected.server_name,
                        game_uid: detected.game_uid,
                    });
                }

                if detected.ambiguous {
                    tracing::debug!(
                        session_no = %command.session_no,
                        message = %detected.message,
                        "login identity is still ambiguous"
                    );
                }
            }

            tokio::time::sleep(self.poll_interval).await;
        }
    }

    async fn cancel(
        &self,
        session_no: &str,
    ) -> Result<(), LoginExecutorError> {
        self.cancelled.lock().await.insert(session_no.to_string());
        Ok(())
    }
}
