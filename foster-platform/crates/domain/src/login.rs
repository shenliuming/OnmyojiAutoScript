use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LoginSessionStatus {
    Created,
    WaitingEmulator,
    Preparing,
    WaitingQr,
    QrReady,
    WaitingScan,
    DetectingLogin,
    VerifyingAccount,
    Success,
    QrExpired,
    Failed,
    Cancelled,
}
