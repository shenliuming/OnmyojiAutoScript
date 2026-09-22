use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::Utc;
use dashmap::{DashMap, mapref::entry::Entry};
use foster_protocol::{PROTOCOL_VERSION, ServerCommand, ServerEnvelope};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AgentPresence {
    pub connection_id: Uuid,
    pub agent_id: String,
    pub host_id: i64,
    pub connected_at: Instant,
    pub last_heartbeat_at: Instant,
}

#[derive(Debug)]
pub struct OutboundMessage {
    pub envelope: ServerEnvelope,
    pub delivered: oneshot::Sender<Result<(), String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentSendError {
    #[error("agent host is offline")]
    Offline,
    #[error("agent outbound queue is closed")]
    QueueClosed,
    #[error("agent delivery acknowledgement channel closed")]
    DeliveryClosed,
    #[error("agent websocket delivery failed: {0}")]
    DeliveryFailed(String),
}

struct RegisteredAgent {
    presence: AgentPresence,
    outbound: Option<mpsc::UnboundedSender<OutboundMessage>>,
}

#[derive(Clone, Default)]
pub struct AgentRegistry {
    inner: Arc<DashMap<i64, RegisteredAgent>>,
}

impl AgentRegistry {
    pub fn register(&self, presence: AgentPresence) {
        self.inner.insert(
            presence.host_id,
            RegisteredAgent {
                presence,
                outbound: None,
            },
        );
    }

    pub fn register_with_sender(
        &self,
        presence: AgentPresence,
        outbound: mpsc::UnboundedSender<OutboundMessage>,
    ) {
        self.inner.insert(
            presence.host_id,
            RegisteredAgent {
                presence,
                outbound: Some(outbound),
            },
        );
    }

    pub fn heartbeat(&self, host_id: i64, connection_id: Uuid) -> bool {
        let Some(mut registered) = self.inner.get_mut(&host_id) else {
            return false;
        };

        if registered.presence.connection_id != connection_id {
            return false;
        }

        registered.presence.last_heartbeat_at = Instant::now();
        true
    }

    pub fn remove_if_current(&self, host_id: i64, connection_id: Uuid) -> bool {
        match self.inner.entry(host_id) {
            Entry::Occupied(entry)
                if entry.get().presence.connection_id == connection_id =>
            {
                entry.remove();
                true
            }
            _ => false,
        }
    }

    pub fn is_online(&self, host_id: i64) -> bool {
        self.inner.contains_key(&host_id)
    }

    pub fn connection_count(&self) -> usize {
        self.inner.len()
    }

    pub async fn send_command(
        &self,
        host_id: i64,
        command: ServerCommand,
    ) -> Result<(), AgentSendError> {
        let sender = self
            .inner
            .get(&host_id)
            .and_then(|registered| registered.outbound.clone())
            .ok_or(AgentSendError::Offline)?;

        let envelope = ServerEnvelope {
            protocol_version: PROTOCOL_VERSION,
            command_id: Uuid::new_v4(),
            sent_at: Utc::now(),
            payload: command,
        };
        let (delivered, delivery) = oneshot::channel();

        sender
            .send(OutboundMessage {
                envelope,
                delivered,
            })
            .map_err(|_| AgentSendError::QueueClosed)?;

        match delivery.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(AgentSendError::DeliveryFailed(message)),
            Err(_) => Err(AgentSendError::DeliveryClosed),
        }
    }

    pub fn remove_stale(&self, timeout: Duration) -> Vec<i64> {
        let stale: Vec<(i64, Uuid)> = self
            .inner
            .iter()
            .filter_map(|entry| {
                if entry.presence.last_heartbeat_at.elapsed() >= timeout {
                    Some((*entry.key(), entry.presence.connection_id))
                } else {
                    None
                }
            })
            .collect();

        stale
            .into_iter()
            .filter_map(|(host_id, connection_id)| {
                self.remove_if_current(host_id, connection_id)
                    .then_some(host_id)
            })
            .collect()
    }
}
