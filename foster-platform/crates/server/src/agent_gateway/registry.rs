use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use dashmap::{DashMap, mapref::entry::Entry};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AgentPresence {
    pub connection_id: Uuid,
    pub agent_id: String,
    pub host_id: i64,
    pub connected_at: Instant,
    pub last_heartbeat_at: Instant,
}

#[derive(Clone, Default)]
pub struct AgentRegistry {
    inner: Arc<DashMap<i64, AgentPresence>>,
}

impl AgentRegistry {
    pub fn register(&self, presence: AgentPresence) {
        self.inner.insert(presence.host_id, presence);
    }

    pub fn heartbeat(&self, host_id: i64, connection_id: Uuid) -> bool {
        let Some(mut presence) = self.inner.get_mut(&host_id) else {
            return false;
        };

        if presence.connection_id != connection_id {
            return false;
        }

        presence.last_heartbeat_at = Instant::now();
        true
    }

    pub fn remove_if_current(&self, host_id: i64, connection_id: Uuid) -> bool {
        match self.inner.entry(host_id) {
            Entry::Occupied(entry) if entry.get().connection_id == connection_id => {
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

    pub fn remove_stale(&self, timeout: Duration) -> Vec<i64> {
        let stale: Vec<(i64, Uuid)> = self
            .inner
            .iter()
            .filter_map(|entry| {
                if entry.last_heartbeat_at.elapsed() >= timeout {
                    Some((*entry.key(), entry.connection_id))
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
