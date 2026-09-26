use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use foster_agent::{
    config::AgentConfig,
    emulator::FakeEmulatorDriver,
    foster::{FosterExecution, FosterExecutor, FosterExecutorError},
    runtime::AgentRuntime,
};
use foster_domain::{EmulatorActivity, EmulatorOccupancyStatus, FosterErrorCode, ResourceMode};
use foster_protocol::{
    AgentEnvelope, AgentEvent, EmulatorDescriptor, ExecuteFosterCommand, FosterDetectedIdentity,
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

#[derive(Clone)]
struct CountingFosterExecutor {
    count: Arc<AtomicUsize>,
    delay: Duration,
}

#[async_trait]
impl FosterExecutor for CountingFosterExecutor {
    async fn execute(
        &self,
        _command: &ExecuteFosterCommand,
    ) -> Result<FosterExecution, FosterExecutorError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;

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

fn foster_envelope(command_id: Uuid, job_id: i64, attempt: i32) -> ServerEnvelope {
    ServerEnvelope {
        protocol_version: PROTOCOL_VERSION,
        command_id,
        sent_at: chrono::Utc::now(),
        payload: ServerCommand::ExecuteFoster(ExecuteFosterCommand {
            job_id,
            attempt,
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
    }
}

async fn send_server_envelope(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    envelope: &ServerEnvelope,
) -> anyhow::Result<()> {
    socket
        .send(Message::Text(serde_json::to_string(envelope)?.into()))
        .await?;
    Ok(())
}

async fn wait_for_foster_success(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    job_id: i64,
) -> anyhow::Result<()> {
    for _ in 0..20 {
        let event = read_agent_event(socket).await?;
        if matches!(
            event.payload,
            AgentEvent::FosterSucceeded(ref value) if value.job_id == job_id
        ) {
            return Ok(());
        }
    }

    anyhow::bail!("foster success event not observed")
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

    assert!(
        saw_heartbeat,
        "heartbeat stopped while foster executor was running"
    );
    assert!(saw_success, "foster success event was not emitted");

    task.abort();
    Ok(())
}

#[tokio::test]
async fn duplicate_foster_command_executes_once_and_replays_finished_result() -> anyhow::Result<()>
{
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("ws://{}/agent/ws", listener.local_addr()?);
    let count = Arc::new(AtomicUsize::new(0));

    let runtime = AgentRuntime::new(test_config(url), FakeEmulatorDriver::new(Vec::new()))
        .with_foster_executor(CountingFosterExecutor {
            count: count.clone(),
            delay: Duration::from_millis(120),
        });
    let task = tokio::spawn(runtime.run());

    let mut socket = accept_authenticated(&listener).await?;
    let _hello = read_agent_event(&mut socket).await?;
    let _snapshot = read_agent_event(&mut socket).await?;

    let command_id = Uuid::new_v4();
    let command = foster_envelope(command_id, 77, 0);

    send_server_envelope(&mut socket, &command).await?;
    send_server_envelope(&mut socket, &command).await?;

    wait_for_foster_success(&mut socket, 77).await?;
    assert_eq!(count.load(Ordering::SeqCst), 1);

    send_server_envelope(&mut socket, &command).await?;
    wait_for_foster_success(&mut socket, 77).await?;
    assert_eq!(count.load(Ordering::SeqCst), 1);

    task.abort();
    Ok(())
}

#[tokio::test]
async fn foster_result_survives_websocket_reconnect() -> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("ws://{}/agent/ws", listener.local_addr()?);
    let count = Arc::new(AtomicUsize::new(0));

    let runtime = AgentRuntime::new(test_config(url), FakeEmulatorDriver::new(Vec::new()))
        .with_foster_executor(CountingFosterExecutor {
            count: count.clone(),
            delay: Duration::from_millis(140),
        });
    let task = tokio::spawn(runtime.run());

    let mut first_socket = accept_authenticated(&listener).await?;
    let _hello = read_agent_event(&mut first_socket).await?;
    let _snapshot = read_agent_event(&mut first_socket).await?;

    let command = foster_envelope(Uuid::new_v4(), 88, 0);
    send_server_envelope(&mut first_socket, &command).await?;
    first_socket.close(None).await?;

    let mut second_socket = accept_authenticated(&listener).await?;
    let hello = read_agent_event(&mut second_socket).await?;
    let AgentEvent::Hello(hello) = hello.payload else {
        anyhow::bail!("expected reconnect hello");
    };
    assert!(
        hello
            .command_states
            .iter()
            .any(|state| state.job_id == Some(88))
    );

    let _snapshot = read_agent_event(&mut second_socket).await?;
    wait_for_foster_success(&mut second_socket, 88).await?;

    assert_eq!(count.load(Ordering::SeqCst), 1);

    task.abort();
    Ok(())
}

#[tokio::test]
async fn runtime_marks_emulator_busy_and_rejects_competing_command() -> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("ws://{}/agent/ws", listener.local_addr()?);
    let count = Arc::new(AtomicUsize::new(0));
    let driver = FakeEmulatorDriver::new(vec![EmulatorDescriptor {
        emulator_code: "emu-01".into(),
        driver_type: "FAKE".into(),
        adb_serial: Some("127.0.0.1:5555".into()),
    }]);

    let runtime =
        AgentRuntime::new(test_config(url), driver).with_foster_executor(CountingFosterExecutor {
            count: count.clone(),
            delay: Duration::from_millis(250),
        });
    let task = tokio::spawn(runtime.run());

    let mut socket = accept_authenticated(&listener).await?;
    let _hello = read_agent_event(&mut socket).await?;
    let _snapshot = read_agent_event(&mut socket).await?;

    let first = foster_envelope(Uuid::new_v4(), 501, 0);
    send_server_envelope(&mut socket, &first).await?;

    let mut saw_busy = false;
    for _ in 0..10 {
        let event = read_agent_event(&mut socket).await?;
        if let AgentEvent::Heartbeat(value) = event.payload {
            if value.emulators.iter().any(|emulator| {
                emulator.emulator_code == "emu-01"
                    && emulator.occupancy == EmulatorOccupancyStatus::Busy
                    && emulator.activity == EmulatorActivity::Foster
                    && emulator.current_job_id == Some(501)
            }) {
                saw_busy = true;
                break;
            }
        }
    }
    assert!(saw_busy, "heartbeat never exposed busy foster runtime");

    let second = foster_envelope(Uuid::new_v4(), 502, 0);
    send_server_envelope(&mut socket, &second).await?;

    let mut rejected = false;
    for _ in 0..10 {
        let event = read_agent_event(&mut socket).await?;
        if matches!(
            event.payload,
            AgentEvent::FosterFailed(ref value)
                if value.job_id == 502
                    && value.error_code == FosterErrorCode::GameBusy
        ) {
            rejected = true;
            break;
        }
    }

    assert!(rejected, "competing emulator command was not rejected");
    wait_for_foster_success(&mut socket, 501).await?;
    assert_eq!(count.load(Ordering::SeqCst), 1);

    task.abort();
    Ok(())
}
