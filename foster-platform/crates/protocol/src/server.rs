use foster_domain::{ResourceMode, ResourceType};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingCommand {
    pub nonce: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RefreshEmulatorsCommand {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartLoginCommand {
    pub session_no: String,
    pub game_account_id: i64,
    pub emulator_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelLoginCommand {
    pub session_no: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FosterTargetIdentity {
    pub masked_account: Option<String>,
    pub account_aliases: Vec<String>,
    pub character_name: Option<String>,
    pub server_name: Option<String>,
    pub game_uid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteFosterCommand {
    pub job_id: i64,
    pub game_account_id: i64,
    pub emulator_code: String,
    pub resource_mode: ResourceMode,
    pub resource_type: Option<ResourceType>,
    pub provider_alias: Option<String>,
    pub target_identity: FosterTargetIdentity,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ServerCommand {
    Ping(PingCommand),
    RefreshEmulators(RefreshEmulatorsCommand),
    StartLogin(StartLoginCommand),
    CancelLogin(CancelLoginCommand),
    ExecuteFoster(ExecuteFosterCommand),
}
