use std::time::Duration;

use async_trait::async_trait;
use foster_agent::{
    config::AgentConfig,
    emulator::FakeEmulatorDriver,
    foster::{
        FosterExecution, FosterExecutor, FosterExecutorError,
    },
    runtime::AgentRuntime,
};
use foster_domain::ResourceMode;
use foster_protocol::{
    AgentEnvelope, AgentEvent, ExecuteFosterCommand, FosterDetectedIdentity,
    FosterTargetIdentity, PROTOCOL_VERSION, ServerCommand, ServerEnvelope,
};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        Message,
        handshake::server::{Request, Response},
    },
};
use uuid::Uuid;

#[derive(Clone)]
struct SlowFosterExecutor;

#[async_trait]
impl FosterExecutor for SlowFosterExecutor {
    async fn execute(
        &self,
        _command: &ExecuteFosterCommand,
    ) -> Result<FosterExecution, FosterExecutorError> {
        tokio::time::sleep(Duration::from_millis(180)).await;

        Ok(FosterExecution {
            stages: Vec::new(),
            completed_at: chrono::Utc::now(),
            success: true,
            error_code: None,
            message: "ok".into(),
            remaining_seconds: Some(21_600),
            screenshot_url: None,
            detected_identity: FosterDetectedIdentity {
                masked_account: Some("12****34".into()),
                character_name: Some("角色A".into()),
                server_name: Some("春之樱".into()),
                game_uid: None,
            },
        })
    }
}

fn test_config(server_ws_url: String) -> AgentConfig {
    AgentConfig {
        server_ws_url,
        agent_token: "test-token".into(),
        agent_id: "agent-foster".into(),
        host_id: 7,
        heartbeat_interval: Duration::from_millis(40),
        reconnect_delays: vec![Duration::from_millis(10)],
    }
}

#[allow(clippy::result_large_err)]
async fn accept_authenticated(
    listener: &TcpListener,
) -> anyhow::Result<tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>> {
    let (stream, _) = listener.accept().await?;
    Ok(
        accept_hdr_async(stream, |request: &Request, response: Response| {
            assert_eq!(
                request
                    .headers()
                    .get(http::header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer test-token")
            );
            Ok(response)
        })
        .await?,
    )
}

async fn read_agent_event(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) -> anyhow::Result<AgentEnvelope> {
    let message = tokio::time::timeout(Duration::from_secs(1), socket.next())
        .await?
        .ok_or_else(|| anyhow::anyhow!("agent socket closed"))??;

    let Message::Text(text) = message else {
        anyhow::bail!("expected text frame");
    };

    Ok(serde_json::from_str(text.as_str())?)
}

#[tokio::test]
async fn heartbeat_continues_while_foster_executes() -> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("ws://{}/agent/ws", listener.local_addr()?);

    let runtime = AgentRuntime::new(test_config(url), FakeEmulatorDriver::new(Vec::new()))
        .with_foster_executor(SlowFosterExecutor);
    let task = tokio::spawn(runtime.run());

    let mut socket = accept_authenticated(&listener).await?;
    let _hello = read_agent_event(&mut socket).await?;
    let _snapshot = read_agent_event(&mut socket).await?;

    let command = ServerEnvelope {
        protocol_version: PROTOCOL_VERSION,
        command_id: Uuid::new_v4(),
        sent_at: chrono::Utc::now(),
        payload: ServerCommand::ExecuteFoster(ExecuteFosterCommand {
            job_id: 42,
            attempt: 0,
            game_account_id: 1001,
            emulator_code: "emu-01".into(),
            resource_mode: ResourceMode::UserFriend,
            resource_type: None,
            provider_alias: None,
            target_identity: FosterTargetIdentity {
                masked_account: Some("12****34".into()),
                account_aliases: Vec::new(),
                character_name: Some("角色A".into()),
                server_name: Some("春之樱".into()),
                game_uid: None,
            },
        }),
    };

    socket
        .send(Message::Text(serde_json::to_string(&command)?.into()))
        .await?;

    let mut saw_heartbeat = false;
    let mut saw_success = false;

    for _ in 0..10 {
        let event = read_agent_event(&mut socket).await?;
        match event.payload {
            AgentEvent::Heartbeat(_) => saw_heartbeat = true,
            AgentEvent::FosterSucceeded(value) if value.job_id == 42 => {
                saw_success = true;
                break;
            }
            _ => {}
        }
    }

    assert!(saw_heartbeat, "heartbeat stopped while foster executor was running");
    assert!(saw_success, "foster success event was not emitted");

    task.abort();
    Ok(())
}
