use foster_agent::{
    config::AgentConfig,
    emulator::FakeEmulatorDriver,
    runtime::AgentRuntime,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let server_ws_url = std::env::var("FOSTER_SERVER_WS_URL")?;
    let agent_token = std::env::var("FOSTER_AGENT_TOKEN")?;
    let agent_id = std::env::var("FOSTER_AGENT_ID")?;
    let host_id = std::env::var("FOSTER_HOST_ID")?.parse::<i64>()?;

    tracing::info!(
        "starting foster agent with Phase 2 fake emulator driver"
    );

    let config =
        AgentConfig::production(server_ws_url, agent_token, agent_id, host_id);
    let runtime = AgentRuntime::new(config, FakeEmulatorDriver::new(Vec::new()));

    runtime.run().await?;
    Ok(())
}
