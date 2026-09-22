#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedLoginSession {
    pub session_id: i64,
    pub session_no: String,
    pub public_token: String,
    pub control_token: String,
    pub emulator_id: i64,
    pub binding_id: i64,
}
