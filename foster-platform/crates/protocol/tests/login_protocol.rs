use chrono::{TimeZone, Utc};
use foster_protocol::{
    AgentEnvelope, AgentEvent, LoginIdentityDetected, LoginQrReady,
    PROTOCOL_VERSION, ServerCommand, ServerEnvelope, StartLoginCommand,
};
use uuid::Uuid;

#[test]
fn start_login_serializes_with_stable_tag() {
    let envelope = ServerEnvelope {
        protocol_version: PROTOCOL_VERSION,
        command_id: Uuid::nil(),
        sent_at: Utc.timestamp_opt(0, 0).single().unwrap(),
        payload: ServerCommand::StartLogin(StartLoginCommand {
            session_no: "LOGIN-001".into(),
            game_account_id: 42,
            emulator_code: "emu-01".into(),
        }),
    };

    let value = serde_json::to_value(envelope).unwrap();

    assert_eq!(value["payload"]["type"], "START_LOGIN");
    assert_eq!(value["payload"]["data"]["session_no"], "LOGIN-001");
    assert_eq!(value["payload"]["data"]["emulator_code"], "emu-01");
}

#[test]
fn login_qr_ready_roundtrips() {
    let envelope = AgentEnvelope {
        protocol_version: PROTOCOL_VERSION,
        event_id: Uuid::nil(),
        sent_at: Utc.timestamp_opt(0, 0).single().unwrap(),
        payload: AgentEvent::LoginQrReady(LoginQrReady {
            session_no: "LOGIN-001".into(),
            qr_payload: "data:image/png;base64,abc".into(),
            expires_at: Utc.timestamp_opt(600, 0).single().unwrap(),
        }),
    };

    let json = serde_json::to_string(&envelope).unwrap();
    let decoded: AgentEnvelope = serde_json::from_str(&json).unwrap();

    assert!(matches!(
        decoded.payload,
        AgentEvent::LoginQrReady(value)
            if value.session_no == "LOGIN-001"
                && value.qr_payload == "data:image/png;base64,abc"
    ));
}

#[test]
fn login_identity_detected_roundtrips_optional_fields() {
    let envelope = AgentEnvelope {
        protocol_version: PROTOCOL_VERSION,
        event_id: Uuid::nil(),
        sent_at: Utc.timestamp_opt(0, 0).single().unwrap(),
        payload: AgentEvent::LoginIdentityDetected(LoginIdentityDetected {
            session_no: "LOGIN-001".into(),
            masked_account: Some("138****5678".into()),
            character_name: Some("角色A".into()),
            server_name: Some("春之樱".into()),
            game_uid: None,
        }),
    };

    let json = serde_json::to_string(&envelope).unwrap();
    let decoded: AgentEnvelope = serde_json::from_str(&json).unwrap();

    assert!(matches!(
        decoded.payload,
        AgentEvent::LoginIdentityDetected(value)
            if value.masked_account.as_deref() == Some("138****5678")
                && value.character_name.as_deref() == Some("角色A")
                && value.server_name.as_deref() == Some("春之樱")
                && value.game_uid.is_none()
    ));
}
