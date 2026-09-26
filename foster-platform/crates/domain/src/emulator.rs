use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EmulatorStatus {
    Offline,
    Idle,
    SwitchingAccount,
    Running,
    LoginSession,
    Maintenance,
    Error,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EmulatorLifecycleStatus {
    #[default]
    Offline,
    Starting,
    Booting,
    Ready,
    Stopping,
    Maintenance,
    Error,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EmulatorOccupancyStatus {
    #[default]
    Idle,
    Busy,
    Recovery,
    Maintenance,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EmulatorActivity {
    #[default]
    None,
    Login,
    Foster,
    ManualControl,
}
