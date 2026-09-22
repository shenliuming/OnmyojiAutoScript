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
