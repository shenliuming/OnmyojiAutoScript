use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AgentGatewayConfig {
    pub agent_token: String,
    pub heartbeat_timeout: Duration,
    pub sweep_interval: Duration,
}

impl AgentGatewayConfig {
    pub fn production(agent_token: String) -> Self {
        Self {
            agent_token,
            heartbeat_timeout: Duration::from_secs(45),
            sweep_interval: Duration::from_secs(5),
        }
    }
}
