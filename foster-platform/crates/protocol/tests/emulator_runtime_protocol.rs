use chrono::Utc;
use foster_domain::{EmulatorActivity, EmulatorLifecycleStatus, EmulatorOccupancyStatus};
use foster_protocol::{AgentEnvelope, AgentEvent, EmulatorHeartbeat, Heartbeat, PROTOCOL_VERSION};
use uuid::Uuid;

#[test]
fn heartbeat_serializes_rich_emulator_runtime_state() {
    let command_id = Uuid::new_v4();
    let envelope = AgentEnvelope {
        protocol_version: PROTOCOL_VERSION,
        event_id: Uuid::new_v4(),
        sent_at: Utc::now(),
        payload: AgentEvent::Heartbeat(Heartbeat {
            host_id: 7,
            emulators: vec![EmulatorHeartbeat {
                emulator_code: "emu-01".into(),
                lifecycle: EmulatorLifecycleStatus::Ready,
                occupancy: EmulatorOccupancyStatus::Busy,
                activity: EmulatorActivity::Foster,
                activity_stage: Some("EXECUTING".into()),
                current_command_id: Some(command_id),
                current_job_id: Some(42),
                current_login_session_no: None,
                current_game_account_id: Some(1001),
                activity_started_at: Some(Utc::now()),
            }],
        }),
    };

    let value = serde_json::to_value(&envelope).unwrap();
    let emulator = &value["payload"]["data"]["emulators"][0];

    assert_eq!(emulator["lifecycle"], "READY");
    assert_eq!(emulator["occupancy"], "BUSY");
    assert_eq!(emulator["activity"], "FOSTER");
    assert_eq!(emulator["activity_stage"], "EXECUTING");
    assert_eq!(emulator["current_command_id"], command_id.to_string());
    assert_eq!(emulator["current_job_id"], 42);
    assert_eq!(emulator["current_game_account_id"], 1001);
}
