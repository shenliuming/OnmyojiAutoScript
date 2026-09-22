use std::{collections::HashMap, time::Duration};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use foster_domain::{FosterErrorCode, ResourceMode, ResourceType};
use foster_protocol::{
    ExecuteFosterCommand, FosterDetectedIdentity, FosterStage,
};
use serde::{Deserialize, Serialize};

use super::{
    FosterExecution, FosterExecutor, FosterExecutorError, FosterStageCheckpoint,
};

#[derive(Debug, Clone)]
pub struct HttpOasFosterExecutor {
    client: reqwest::Client,
    base_url: String,
    config_map: HashMap<String, String>,
    timeout: Duration,
}

impl HttpOasFosterExecutor {
    pub fn new(
        base_url: String,
        config_map: HashMap<String, String>,
        timeout: Duration,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            config_map,
            timeout,
        }
    }

    fn config_name(&self, emulator_code: &str) -> String {
        self.config_map
            .get(emulator_code)
            .cloned()
            .unwrap_or_else(|| emulator_code.to_string())
    }
}

#[derive(Debug, Serialize)]
struct OasFosterRequest {
    config_name: String,
    job_id: i64,
    resource_mode: &'static str,
    resource_type: Option<&'static str>,
    provider_alias: Option<String>,
    masked_account: Option<String>,
    account_aliases: Vec<String>,
    character_name: Option<String>,
    server_name: Option<String>,
    game_uid: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OasStageCheckpoint {
    stage: String,
    occurred_at: String,
}

#[derive(Debug, Default, Deserialize)]
struct OasDetectedIdentity {
    masked_account: Option<String>,
    character_name: Option<String>,
    server_name: Option<String>,
    game_uid: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OasFosterResponse {
    success: bool,
    code: String,
    message: String,
    remaining_seconds: Option<i64>,
    screenshot_url: Option<String>,
    stages: Vec<OasStageCheckpoint>,
    detected_identity: OasDetectedIdentity,
}

#[async_trait]
impl FosterExecutor for HttpOasFosterExecutor {
    async fn execute(
        &self,
        command: &ExecuteFosterCommand,
    ) -> Result<FosterExecution, FosterExecutorError> {
        let request = OasFosterRequest {
            config_name: self.config_name(&command.emulator_code),
            job_id: command.job_id,
            resource_mode: resource_mode_name(command.resource_mode),
            resource_type: command.resource_type.map(resource_type_name),
            provider_alias: command.provider_alias.clone(),
            masked_account: command.target_identity.masked_account.clone(),
            account_aliases: command.target_identity.account_aliases.clone(),
            character_name: command.target_identity.character_name.clone(),
            server_name: command.target_identity.server_name.clone(),
            game_uid: command.target_identity.game_uid.clone(),
        };

        let response = self
            .client
            .post(format!("{}/foster/execute", self.base_url))
            .timeout(self.timeout)
            .json(&request)
            .send()
            .await;

        let response = match response {
            Ok(response) => response,
            Err(error) => {
                return Ok(network_failure(format!(
                    "OAS bridge request failed: {error}"
                )));
            }
        };

        if !response.status().is_success() {
            return Ok(network_failure(format!(
                "OAS bridge returned HTTP {}",
                response.status()
            )));
        }

        let response = response
            .json::<OasFosterResponse>()
            .await
            .map_err(|error| FosterExecutorError::Message(error.to_string()))?;

        let stages = response
            .stages
            .into_iter()
            .filter_map(|checkpoint| {
                let stage = parse_stage(&checkpoint.stage)?;
                let occurred_at = DateTime::parse_from_rfc3339(&checkpoint.occurred_at)
                    .ok()?
                    .with_timezone(&Utc);
                Some(FosterStageCheckpoint { stage, occurred_at })
            })
            .collect();

        Ok(FosterExecution {
            stages,
            completed_at: Utc::now(),
            success: response.success,
            error_code: (!response.success).then(|| parse_error_code(&response.code)),
            message: response.message,
            remaining_seconds: response.remaining_seconds,
            screenshot_url: response.screenshot_url,
            detected_identity: FosterDetectedIdentity {
                masked_account: response.detected_identity.masked_account,
                character_name: response.detected_identity.character_name,
                server_name: response.detected_identity.server_name,
                game_uid: response.detected_identity.game_uid,
            },
        })
    }
}

fn network_failure(message: String) -> FosterExecution {
    FosterExecution {
        stages: Vec::new(),
        completed_at: Utc::now(),
        success: false,
        error_code: Some(FosterErrorCode::NetworkError),
        message,
        remaining_seconds: None,
        screenshot_url: None,
        detected_identity: FosterDetectedIdentity {
            masked_account: None,
            character_name: None,
            server_name: None,
            game_uid: None,
        },
    }
}

fn resource_mode_name(mode: ResourceMode) -> &'static str {
    match mode {
        ResourceMode::UserFriend => "USER_FRIEND",
        ResourceMode::Platform => "PLATFORM",
    }
}

fn resource_type_name(resource_type: ResourceType) -> &'static str {
    match resource_type {
        ResourceType::Fish => "FISH",
        ResourceType::TaikoJade => "TAIKO_JADE",
    }
}

fn parse_stage(value: &str) -> Option<FosterStage> {
    match value {
        "SWITCHING_ACCOUNT" => Some(FosterStage::SwitchingAccount),
        "VERIFYING_ACCOUNT" => Some(FosterStage::VerifyingAccount),
        "RUNNING" => Some(FosterStage::Running),
        _ => None,
    }
}

fn parse_error_code(value: &str) -> FosterErrorCode {
    match value {
        "NO_SLOT" => FosterErrorCode::NoSlot,
        "PROVIDER_NOT_FOUND" => FosterErrorCode::ProviderNotFound,
        "ACCOUNT_LOGIN_EXPIRED" => FosterErrorCode::AccountLoginExpired,
        "IDENTITY_MISMATCH" => FosterErrorCode::IdentityMismatch,
        "EMULATOR_OFFLINE" => FosterErrorCode::EmulatorOffline,
        "NETWORK_ERROR" => FosterErrorCode::NetworkError,
        "GAME_BUSY" => FosterErrorCode::GameBusy,
        _ => FosterErrorCode::Unknown,
    }
}
