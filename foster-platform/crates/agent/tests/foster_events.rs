use chrono::{TimeZone, Utc};
use foster_agent::foster::{
    FosterExecution, FosterStageCheckpoint, events_for_execution,
};
use foster_domain::FosterErrorCode;
use foster_protocol::{
    AgentEvent, FosterDetectedIdentity, FosterStage,
};

fn identity() -> FosterDetectedIdentity {
    FosterDetectedIdentity {
        masked_account: Some("12****34".into()),
        character_name: Some("角色A".into()),
        server_name: Some("春之樱".into()),
        game_uid: None,
    }
}

#[test]
fn successful_execution_preserves_stage_order_before_success() {
    let base = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let execution = FosterExecution {
        stages: vec![
            FosterStageCheckpoint {
                stage: FosterStage::SwitchingAccount,
                occurred_at: base,
            },
            FosterStageCheckpoint {
                stage: FosterStage::VerifyingAccount,
                occurred_at: base + chrono::Duration::seconds(2),
            },
            FosterStageCheckpoint {
                stage: FosterStage::Running,
                occurred_at: base + chrono::Duration::seconds(4),
            },
        ],
        completed_at: base + chrono::Duration::seconds(10),
        success: true,
        error_code: None,
        message: "ok".into(),
        remaining_seconds: Some(21_600),
        screenshot_url: None,
        detected_identity: identity(),
    };

    let events = events_for_execution(42, 3, execution);

    assert!(matches!(
        &events[0],
        AgentEvent::FosterStageChanged(value)
            if value.stage == FosterStage::SwitchingAccount
    ));
    assert!(matches!(
        &events[1],
        AgentEvent::FosterStageChanged(value)
            if value.stage == FosterStage::VerifyingAccount
    ));
    assert!(matches!(
        &events[2],
        AgentEvent::FosterStageChanged(value)
            if value.stage == FosterStage::Running
    ));
    assert!(matches!(
        &events[3],
        AgentEvent::FosterSucceeded(value)
            if value.job_id == 42 && value.attempt == 3 && value.remaining_seconds == Some(21_600)
    ));
}

#[test]
fn failed_execution_emits_structured_failure() {
    let at = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let execution = FosterExecution {
        stages: Vec::new(),
        completed_at: at,
        success: false,
        error_code: Some(FosterErrorCode::NetworkError),
        message: "connection refused".into(),
        remaining_seconds: None,
        screenshot_url: None,
        detected_identity: identity(),
    };

    let events = events_for_execution(7, 1, execution);

    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        AgentEvent::FosterFailed(value)
            if value.job_id == 7
                && value.attempt == 1
                && value.error_code == FosterErrorCode::NetworkError
    ));
}
