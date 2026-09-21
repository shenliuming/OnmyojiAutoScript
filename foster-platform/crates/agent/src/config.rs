use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub server_ws_url: String,
    pub agent_token: String,
    pub agent_id: String,
    pub host_id: i64,
    pub heartbeat_interval: Duration,
    pub reconnect_delays: Vec<Duration>,
}

impl AgentConfig {
    pub fn production(
        server_ws_url: String,
        agent_token: String,
        agent_id: String,
        host_id: i64,
    ) -> Self {
        Self {
            server_ws_url,
            agent_token,
            agent_id,
            host_id,
            heartbeat_interval: Duration::from_secs(15),
            reconnect_delays: vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
                Duration::from_secs(16),
                Duration::from_secs(30),
            ],
        }
    }
}
