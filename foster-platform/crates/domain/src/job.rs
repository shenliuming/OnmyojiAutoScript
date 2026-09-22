use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FosterJobStatus {
    Pending,
    DeferredQuiet,
    DeferredManual,
    WaitingEmulator,
    WaitingResource,
    SwitchingAccount,
    VerifyingAccount,
    Running,
    Success,
    Retry,
    Failed,
    IdentityMismatch,
    Cancelled,
    RecoveryRequired,
}

impl FosterJobStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Success | Self::Failed | Self::IdentityMismatch | Self::Cancelled
        )
    }

    pub fn is_executing(self) -> bool {
        matches!(
            self,
            Self::SwitchingAccount | Self::VerifyingAccount | Self::Running
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FosterErrorCode {
    NoSlot,
    ProviderNotFound,
    AccountLoginExpired,
    IdentityMismatch,
    EmulatorOffline,
    NetworkError,
    GameBusy,
    Unknown,
}

impl FosterErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoSlot => "NO_SLOT",
            Self::ProviderNotFound => "PROVIDER_NOT_FOUND",
            Self::AccountLoginExpired => "ACCOUNT_LOGIN_EXPIRED",
            Self::IdentityMismatch => "IDENTITY_MISMATCH",
            Self::EmulatorOffline => "EMULATOR_OFFLINE",
            Self::NetworkError => "NETWORK_ERROR",
            Self::GameBusy => "GAME_BUSY",
            Self::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryDecision {
    RetryAt(DateTime<Utc>),
    WaitForEmulator,
    FailTerminal,
    SuspendAccount,
}
