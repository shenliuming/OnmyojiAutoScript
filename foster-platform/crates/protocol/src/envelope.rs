#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use uuid::Uuid;

    use crate::{
        AgentEnvelope, AgentEvent, AgentHello, PROTOCOL_VERSION, ProtocolVersionError,
        validate_protocol_version,
    };

    #[test]
    fn hello_serializes_with_stable_type_name() {
        let envelope = AgentEnvelope {
            protocol_version: PROTOCOL_VERSION,
            event_id: Uuid::nil(),
            sent_at: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            payload: AgentEvent::Hello(AgentHello {
                agent_id: "agent-01".into(),
                host_id: 7,
                agent_version: "0.1.0".into(),
                hostname: "win-host".into(),
                os_version: "windows".into(),
                capabilities: vec!["EMULATOR_DISCOVERY".into()],
            }),
        };

        let value = serde_json::to_value(envelope).unwrap();

        assert_eq!(value["protocol_version"], 1);
        assert_eq!(value["payload"]["type"], "HELLO");
        assert_eq!(value["payload"]["data"]["host_id"], 7);
    }

    #[test]
    fn unsupported_protocol_version_is_rejected() {
        assert_eq!(
            validate_protocol_version(PROTOCOL_VERSION + 1),
            Err(ProtocolVersionError {
                received: PROTOCOL_VERSION + 1,
                expected: PROTOCOL_VERSION,
            })
        );
    }
}
