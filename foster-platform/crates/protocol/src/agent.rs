use chrono::{DateTime, Utc};
use foster_domain::EmulatorStatus;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHello {
    pub agent_id: String,
    pub host_id: i64,
    pub agent_version: String,
    pub hostname: String,
    pub os_version: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmulatorHeartbeat {
    pub emulator_code: String,
    pub status: EmulatorStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heartbeat {
    pub host_id: i64,
    pub emulators: Vec<EmulatorHeartbeat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmulatorDescriptor {
    pub emulator_code: String,
    pub driver_type: String,
    pub adb_serial: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmulatorSnapshot {
    pub host_id: i64,
    pub emulators: Vec<EmulatorDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pong {
    pub nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginPreparing {
    pub session_no: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginQrReady {
    pub session_no: String,
    pub qr_payload: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginQrExpired {
    pub session_no: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginIdentityDetected {
    pub session_no: String,
    pub masked_account: Option<String>,
    pub character_name: Option<String>,
    pub server_name: Option<String>,
    pub game_uid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginFailed {
    pub session_no: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentEvent {
    Hello(AgentHello),
    Heartbeat(Heartbeat),
    EmulatorSnapshot(EmulatorSnapshot),
    Pong(Pong),
    LoginPreparing(LoginPreparing),
    LoginQrReady(LoginQrReady),
    LoginQrExpired(LoginQrExpired),
    LoginIdentityDetected(LoginIdentityDetected),
    LoginFailed(LoginFailed),
}
