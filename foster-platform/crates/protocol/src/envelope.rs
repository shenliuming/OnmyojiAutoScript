use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AgentEvent, ServerCommand};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerEnvelope {
    pub protocol_version: u16,
    pub command_id: Uuid,
    pub sent_at: DateTime<Utc>,
    pub payload: ServerCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEnvelope {
    pub protocol_version: u16,
    pub event_id: Uuid,
    pub sent_at: DateTime<Utc>,
    pub payload: AgentEvent,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("unsupported protocol version {received}; expected {expected}")]
pub struct ProtocolVersionError {
    pub received: u16,
    pub expected: u16,
}

pub fn validate_protocol_version(received: u16) -> Result<(), ProtocolVersionError> {
    if received == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(ProtocolVersionError {
            received,
            expected: PROTOCOL_VERSION,
        })
    }
}

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
