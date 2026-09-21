use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingCommand {
    pub nonce: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RefreshEmulatorsCommand {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ServerCommand {
    Ping(PingCommand),
    RefreshEmulators(RefreshEmulatorsCommand),
}
