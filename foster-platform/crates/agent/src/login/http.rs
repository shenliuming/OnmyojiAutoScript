use std::{collections::HashSet, sync::Arc, time::Duration};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use foster_protocol::{
    LoginPlatform, SelectLoginIdentityCommand, SelectLoginPlatformCommand, StartLoginCommand,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::emulator::{CommandRunner, GenericAdbEmulatorDriver};


use super::{LoginExecutor, LoginExecutorError, LoginIdentity, LoginPrepared};

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
    explicit_identity_sessions: Arc<Mutex<HashSet<String>>>,
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
            explicit_identity_sessions: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    async fn is_cancelled(&self, session_no: &str) -> bool {
        self.cancelled.lock().await.contains(session_no)
    }
}

#[derive(Debug, Serialize)]
struct SelectPlatformRequest {
    config_name: String,
    platform: &'static str,
}

#[derive(Debug, Serialize)]
struct SelectIdentityRequest {
    config_name: String,
    server_name: String,
    character_name: String,
}

#[derive(Debug, Deserialize)]
struct DetectLoginResponse {
    ready: bool,
    message: String,
    masked_account: Option<String>,
    character_name: Option<String>,
    server_name: Option<String>,
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

        let qr_payload = format!("data:image/png;base64,{}", STANDARD.encode(png));

        Ok(LoginPrepared {
            qr_payload,
            qr_ttl: self.qr_ttl,
        })
    }

    async fn wait_identity(
        &self,
        command: &StartLoginCommand,
    ) -> Result<LoginIdentity, LoginExecutorError> {
        let deadline = tokio::time::Instant::now() + self.identity_timeout;
        let platform_command = SelectLoginPlatformCommand {
            session_no: command.session_no.clone(),
            emulator_code: command.emulator_code.clone(),
            platform: command.platform,
        };
        let identity_command = SelectLoginIdentityCommand {
            session_no: command.session_no.clone(),
            emulator_code: command.emulator_code.clone(),
            server_name: String::new(),
            character_name: command.character_name.clone(),
        };

        let mut platform_selected = false;
        loop {
            if self.is_cancelled(&command.session_no).await {
                return Err(LoginExecutorError::Message("login cancelled".into()));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(LoginExecutorError::Message(
                    "timed out waiting for QR scan and target character selection".into(),
                ));
            }

            if !platform_selected {
                match self.select_platform(&platform_command).await {
                    Ok(()) => {
                        platform_selected = true;
                    }
                    Err(error) => {
                        tracing::debug!(
                            session_no = %command.session_no,
                            error = %error,
                            "waiting for requested Android/iOS platform screen"
                        );
                        tokio::time::sleep(self.poll_interval).await;
                        continue;
                    }
                }
            }

            match self.select_identity(&identity_command).await {
                Ok(identity) => return Ok(identity),
                Err(error) => {
                    tracing::debug!(
                        session_no = %command.session_no,
                        character_name = %command.character_name,
                        error = %error,
                        "waiting for target character selection"
                    );
                    tokio::time::sleep(self.poll_interval).await;
                }
            }
        }
    }

    async fn select_platform(
        &self,
        command: &SelectLoginPlatformCommand,
    ) -> Result<(), LoginExecutorError> {
        let config = self
            .driver
            .instance_config(&command.emulator_code)
            .map_err(|error| LoginExecutorError::Message(error.to_string()))?;
        let platform = match command.platform {
            LoginPlatform::Android => "ANDROID",
            LoginPlatform::Ios => "IOS",
        };
        let response = self
            .client
            .post(format!("{}/login/platform", self.base_url))
            .json(&SelectPlatformRequest {
                config_name: config.oas_config_name.clone(),
                platform,
            })
            .send()
            .await
            .map_err(|error| LoginExecutorError::Message(error.to_string()))?;

        if !response.status().is_success() {
            return Err(LoginExecutorError::Message(format!(
                "OAS platform selection failed with HTTP {}",
                response.status()
            )));
        }

        let result = response
            .json::<DetectLoginResponse>()
            .await
            .map_err(|error| LoginExecutorError::Message(error.to_string()))?;
        if !result.ready {
            return Err(LoginExecutorError::Message(result.message));
        }

        self.explicit_identity_sessions
            .lock()
            .await
            .insert(command.session_no.clone());

        Ok(())
    }

    async fn select_identity(
        &self,
        command: &SelectLoginIdentityCommand,
    ) -> Result<LoginIdentity, LoginExecutorError> {
        let config = self
            .driver
            .instance_config(&command.emulator_code)
            .map_err(|error| LoginExecutorError::Message(error.to_string()))?;
        let response = self
            .client
            .post(format!("{}/login/select-identity", self.base_url))
            .json(&SelectIdentityRequest {
                config_name: config.oas_config_name.clone(),
                server_name: command.server_name.clone(),
                character_name: command.character_name.clone(),
            })
            .send()
            .await
            .map_err(|error| LoginExecutorError::Message(error.to_string()))?;

        if !response.status().is_success() {
            return Err(LoginExecutorError::Message(format!(
                "OAS identity selection failed with HTTP {}",
                response.status()
            )));
        }

        let detected = response
            .json::<DetectLoginResponse>()
            .await
            .map_err(|error| LoginExecutorError::Message(error.to_string()))?;
        if !detected.ready {
            return Err(LoginExecutorError::Message(detected.message));
        }

        let identity = LoginIdentity {
            masked_account: detected.masked_account,
            character_name: detected.character_name,
            server_name: detected.server_name,
            game_uid: None,
        };
        self.explicit_identity_sessions
            .lock()
            .await
            .remove(&command.session_no);
        Ok(identity)
    }

    async fn cancel(&self, session_no: &str) -> Result<(), LoginExecutorError> {
        self.cancelled.lock().await.insert(session_no.to_string());
        self.explicit_identity_sessions.lock().await.remove(session_no);
        Ok(())
    }
}
