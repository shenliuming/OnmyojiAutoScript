use chrono::{DateTime, Utc};
use foster_domain::{EmulatorStatus, FosterErrorCode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentCommandKind {
    Foster,
    Login,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentCommandStatus {
    Running,
    Finished,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCommandState {
    pub command_id: Uuid,
    pub kind: AgentCommandKind,
    pub status: AgentCommandStatus,
    pub job_id: Option<i64>,
    pub attempt: Option<i32>,
    pub session_no: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHello {
    pub agent_id: String,
    pub host_id: i64,
    pub agent_version: String,
    pub hostname: String,
    pub os_version: String,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub command_states: Vec<AgentCommandState>,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FosterStage {
    SwitchingAccount,
    VerifyingAccount,
    Running,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FosterStageChanged {
    pub job_id: i64,
    pub attempt: i32,
    pub stage: FosterStage,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FosterDetectedIdentity {
    pub masked_account: Option<String>,
    pub character_name: Option<String>,
    pub server_name: Option<String>,
    pub game_uid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FosterSucceeded {
    pub job_id: i64,
    pub attempt: i32,
    pub completed_at: DateTime<Utc>,
    pub remaining_seconds: Option<i64>,
    pub screenshot_url: Option<String>,
    pub detected_identity: FosterDetectedIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FosterFailed {
    pub job_id: i64,
    pub attempt: i32,
    pub failed_at: DateTime<Utc>,
    pub error_code: FosterErrorCode,
    pub message: String,
    pub screenshot_url: Option<String>,
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
    FosterStageChanged(FosterStageChanged),
    FosterSucceeded(FosterSucceeded),
    FosterFailed(FosterFailed),
}
