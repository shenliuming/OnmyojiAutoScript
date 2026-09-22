use foster_server::{
    agent_gateway::registry::AgentRegistry,
    app::{AppState, build_app_with_admin_token},
    config::{AgentGatewayConfig, ServerConfig},
};
use sqlx::mysql::MySqlPoolOptions;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = ServerConfig::from_env()?;
    let pool = MySqlPoolOptions::new()
        .max_connections(20)
        .connect(&config.database_url)
        .await?;

    sqlx::migrate!("../../migrations").run(&pool).await?;

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    tracing::info!(bind_addr = %config.bind_addr, "foster server listening");

    let state = AppState {
        pool,
        registry: AgentRegistry::default(),
        gateway_config: AgentGatewayConfig::production(config.agent_token),
    };
    let app = build_app_with_admin_token(state, config.admin_token);

    axum::serve(listener, app).await?;
    Ok(())
}
