use std::{collections::HashMap, time::Duration};

use foster_agent::{
    config::AgentConfig,
    emulator::FakeEmulatorDriver,
    foster::HttpOasFosterExecutor,
    runtime::AgentRuntime,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let server_ws_url = std::env::var("FOSTER_SERVER_WS_URL")?;
    let agent_token = std::env::var("FOSTER_AGENT_TOKEN")?;
    let agent_id = std::env::var("FOSTER_AGENT_ID")?;
    let host_id = std::env::var("FOSTER_HOST_ID")?.parse::<i64>()?;

    let oas_base_url = std::env::var("FOSTER_OAS_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:22270".to_string());
    let oas_config_map: HashMap<String, String> = std::env::var("FOSTER_OAS_CONFIG_MAP")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| serde_json::from_str(&value))
        .transpose()?
        .unwrap_or_default();
    let foster_timeout = std::env::var("FOSTER_OAS_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(180));

    tracing::info!(
        oas_base_url = %oas_base_url,
        "starting foster agent; emulator vendor driver remains fake until host integration phase"
    );

    let config = AgentConfig::production(server_ws_url, agent_token, agent_id, host_id);
    let foster_executor =
        HttpOasFosterExecutor::new(oas_base_url, oas_config_map, foster_timeout);
    let runtime = AgentRuntime::new(config, FakeEmulatorDriver::new(Vec::new()))
        .with_foster_executor(foster_executor);

    runtime.run().await?;
    Ok(())
}
