use std::time::Duration;

use foster_agent::{
    command_journal::CommandJournal, config::AgentConfig, emulator::GenericAdbEmulatorDriver,
    foster::HttpOasFosterExecutor, login::HttpOasLoginExecutor, mumu::MumuConfig,
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
    let journal_path = std::env::var("FOSTER_COMMAND_JOURNAL_PATH")
        .unwrap_or_else(|_| "command-journal.json".to_string());
    let command_journal = CommandJournal::open(&journal_path)?;
    let _mumu_config = MumuConfig::from_env()?;

    let emulators_json = std::env::var("FOSTER_EMULATORS_JSON")?;
    let adb_program = std::env::var("FOSTER_ADB_PATH").unwrap_or_else(|_| "adb".to_string());
    let driver = GenericAdbEmulatorDriver::from_json(&emulators_json, adb_program)?;

    let oas_base_url = std::env::var("FOSTER_OAS_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:22270".to_string());

    let foster_timeout = std::env::var("FOSTER_OAS_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(180));

    let login_qr_ttl = std::env::var("FOSTER_LOGIN_QR_TTL_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(120));
    let login_identity_timeout = std::env::var("FOSTER_LOGIN_IDENTITY_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(300));
    let login_poll_interval = std::env::var("FOSTER_LOGIN_POLL_INTERVAL_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(2));

    tracing::info!(
        oas_base_url = %oas_base_url,
        emulator_count = driver.oas_config_map().len(),
        command_journal = %journal_path,
        "starting foster agent with generic adb host runtime"
    );

    let config = AgentConfig::production(server_ws_url, agent_token, agent_id, host_id);

    let login_executor = HttpOasLoginExecutor::new(
        driver.clone(),
        oas_base_url.clone(),
        login_qr_ttl,
        login_identity_timeout,
        login_poll_interval,
    );

    let foster_executor =
        HttpOasFosterExecutor::new(oas_base_url, driver.oas_config_map(), foster_timeout);

    let runtime = AgentRuntime::new(config, driver)
        .with_login_executor(login_executor)
        .with_foster_executor(foster_executor)
        .with_command_journal(command_journal);

    runtime.run().await?;
    Ok(())
}
