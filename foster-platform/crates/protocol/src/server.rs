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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ServerCommand {
    Ping(PingCommand),
    RefreshEmulators(RefreshEmulatorsCommand),
    StartLogin(StartLoginCommand),
    CancelLogin(CancelLoginCommand),
}
