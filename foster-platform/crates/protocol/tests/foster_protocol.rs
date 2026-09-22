use chrono::{TimeZone, Utc};
use foster_domain::{FosterErrorCode, ResourceMode, ResourceType};
use foster_protocol::{
    AgentEnvelope, AgentEvent, ExecuteFosterCommand, FosterDetectedIdentity, FosterFailed,
    FosterStage, FosterStageChanged, FosterSucceeded, FosterTargetIdentity, PROTOCOL_VERSION,
    ServerCommand, ServerEnvelope,
};
use uuid::Uuid;

#[test]
fn execute_foster_serializes_with_stable_type_name() {
    let envelope = ServerEnvelope {
        protocol_version: PROTOCOL_VERSION,
        command_id: Uuid::nil(),
        sent_at: Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap(),
        payload: ServerCommand::ExecuteFoster(ExecuteFosterCommand {
            job_id: 99,
            attempt: 2,
            game_account_id: 7,
            emulator_code: "emu-01".into(),
            resource_mode: ResourceMode::UserFriend,
            resource_type: Some(ResourceType::Fish),
            provider_alias: None,
            target_identity: FosterTargetIdentity {
                masked_account: Some("12****34".into()),
                account_aliases: vec!["12".into()],
                character_name: Some("角色A".into()),
                server_name: Some("春之樱".into()),
                game_uid: None,
            },
        }),
    };

    let value = serde_json::to_value(envelope).unwrap();
    assert_eq!(value["payload"]["type"], "EXECUTE_FOSTER");
    assert_eq!(value["payload"]["data"]["resource_mode"], "USER_FRIEND");
    assert_eq!(value["payload"]["data"]["resource_type"], "FISH");
    assert_eq!(value["payload"]["data"]["job_id"], 99);
}

#[test]
fn foster_events_serialize_with_stable_tags() {
    let at = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let events = vec![
        AgentEvent::FosterStageChanged(FosterStageChanged {
            job_id: 1,
            attempt: 0,
            stage: FosterStage::VerifyingAccount,
            occurred_at: at,
        }),
        AgentEvent::FosterSucceeded(FosterSucceeded {
            job_id: 1,
            attempt: 0,
            completed_at: at,
            remaining_seconds: Some(21_600),
            screenshot_url: None,
            detected_identity: FosterDetectedIdentity {
                masked_account: Some("12****34".into()),
                character_name: Some("角色A".into()),
                server_name: Some("春之樱".into()),
                game_uid: None,
            },
        }),
        AgentEvent::FosterFailed(FosterFailed {
            job_id: 2,
            attempt: 0,
            failed_at: at,
            error_code: FosterErrorCode::NoSlot,
            message: "no slot".into(),
            screenshot_url: None,
        }),
    ];

    let tags: Vec<String> = events
        .into_iter()
        .map(|payload| {
            let envelope = AgentEnvelope {
                protocol_version: PROTOCOL_VERSION,
                event_id: Uuid::nil(),
                sent_at: at,
                payload,
            };
            serde_json::to_value(envelope).unwrap()["payload"]["type"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();

    assert_eq!(
        tags,
        vec!["FOSTER_STAGE_CHANGED", "FOSTER_SUCCEEDED", "FOSTER_FAILED"]
    );
}
