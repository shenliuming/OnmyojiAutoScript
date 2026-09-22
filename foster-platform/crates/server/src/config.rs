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

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub database_url: String,
    pub agent_token: String,
    pub admin_token: Option<String>,
    pub bind_addr: std::net::SocketAddr,
}

#[derive(Debug, thiserror::Error)]
pub enum ServerConfigError {
    #[error("DATABASE_URL is required")]
    MissingDatabaseUrl,
    #[error("FOSTER_AGENT_TOKEN is required")]
    MissingAgentToken,
    #[error("invalid FOSTER_BIND_ADDR: {0}")]
    InvalidBindAddr(String),
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, ServerConfigError> {
        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| ServerConfigError::MissingDatabaseUrl)?;
        let agent_token = std::env::var("FOSTER_AGENT_TOKEN")
            .map_err(|_| ServerConfigError::MissingAgentToken)?;
        let admin_token = std::env::var("FOSTER_ADMIN_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let bind_addr = std::env::var("FOSTER_BIND_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
            .parse()
            .map_err(|_| {
                ServerConfigError::InvalidBindAddr(
                    std::env::var("FOSTER_BIND_ADDR")
                        .unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
                )
            })?;

        Ok(Self {
            database_url,
            agent_token,
            admin_token,
            bind_addr,
        })
    }
}
