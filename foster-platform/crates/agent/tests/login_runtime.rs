use std::time::Duration;

use foster_agent::{
    config::AgentConfig,
    emulator::FakeEmulatorDriver,
    login::{FakeLoginExecutor, FakeLoginScenario},
    runtime::AgentRuntime,
};
use foster_protocol::{
    AgentEnvelope, AgentEvent, CancelLoginCommand, PROTOCOL_VERSION, ServerCommand, ServerEnvelope,
    StartLoginCommand,
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

fn test_config(server_ws_url: String) -> AgentConfig {
    AgentConfig {
        server_ws_url,
        agent_token: "test-token".into(),
        agent_id: "agent-login".into(),
        host_id: 7,
        heartbeat_interval: Duration::from_secs(30),
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

async fn read_non_heartbeat_event(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) -> anyhow::Result<AgentEnvelope> {
    loop {
        let event = read_agent_event(socket).await?;
        if !matches!(event.payload, AgentEvent::Heartbeat(_)) {
            return Ok(event);
        }
    }
}

async fn send_command(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    payload: ServerCommand,
) -> anyhow::Result<()> {
    let envelope = ServerEnvelope {
        protocol_version: PROTOCOL_VERSION,
        command_id: Uuid::new_v4(),
        sent_at: chrono::Utc::now(),
        payload,
    };

    socket
        .send(Message::Text(serde_json::to_string(&envelope)?.into()))
        .await?;

    Ok(())
}

async fn send_command_with_id(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    command_id: Uuid,
    payload: ServerCommand,
) -> anyhow::Result<()> {
    let envelope = ServerEnvelope {
        protocol_version: PROTOCOL_VERSION,
        command_id,
        sent_at: chrono::Utc::now(),
        payload,
    };

    socket
        .send(Message::Text(serde_json::to_string(&envelope)?.into()))
        .await?;

    Ok(())
}

fn scenario() -> FakeLoginScenario {
    FakeLoginScenario {
        qr_payload: "fake-qr://login".into(),
        qr_ttl: Duration::from_secs(120),
        identity_delay: Duration::ZERO,
        masked_account: Some("138****5678".into()),
        character_name: Some("角色A".into()),
        server_name: Some("春之樱".into()),
        game_uid: Some("10001".into()),
    }
}

#[tokio::test]
async fn start_login_emits_ordered_login_events() -> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("ws://{}/agent/ws", listener.local_addr()?);
    let executor = FakeLoginExecutor::new(scenario());

    let runtime = AgentRuntime::new(test_config(url), FakeEmulatorDriver::new(Vec::new()))
        .with_login_executor(executor);
    let task = tokio::spawn(runtime.run());

    let mut socket = accept_authenticated(&listener).await?;
    let _hello = read_agent_event(&mut socket).await?;
    let _snapshot = read_agent_event(&mut socket).await?;

    send_command(
        &mut socket,
        ServerCommand::StartLogin(StartLoginCommand {
            session_no: "LOGIN-001".into(),
            game_account_id: 1001,
            emulator_code: "emu-01".into(),
        }),
    )
    .await?;

    let preparing = read_non_heartbeat_event(&mut socket).await?;
    let qr = read_non_heartbeat_event(&mut socket).await?;
    let identity = read_non_heartbeat_event(&mut socket).await?;

    assert!(matches!(
        preparing.payload,
        AgentEvent::LoginPreparing(ref event) if event.session_no == "LOGIN-001"
    ));
    assert!(matches!(
        qr.payload,
        AgentEvent::LoginQrReady(ref event)
            if event.session_no == "LOGIN-001"
                && event.qr_payload == "fake-qr://login"
    ));
    assert!(matches!(
        identity.payload,
        AgentEvent::LoginIdentityDetected(ref event)
            if event.session_no == "LOGIN-001"
                && event.character_name.as_deref() == Some("角色A")
                && event.server_name.as_deref() == Some("春之樱")
                && event.game_uid.as_deref() == Some("10001")
    ));

    task.abort();
    Ok(())
}

#[tokio::test]
async fn cancel_login_invokes_executor_without_emitting_stale_login_events() -> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("ws://{}/agent/ws", listener.local_addr()?);
    let executor = FakeLoginExecutor::new(scenario());
    let observer = executor.clone();

    let runtime = AgentRuntime::new(test_config(url), FakeEmulatorDriver::new(Vec::new()))
        .with_login_executor(executor);
    let task = tokio::spawn(runtime.run());

    let mut socket = accept_authenticated(&listener).await?;
    let _hello = read_agent_event(&mut socket).await?;
    let _snapshot = read_agent_event(&mut socket).await?;

    send_command(
        &mut socket,
        ServerCommand::CancelLogin(CancelLoginCommand {
            session_no: "LOGIN-CANCEL".into(),
        }),
    )
    .await?;

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if observer.was_cancelled("LOGIN-CANCEL").await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;

    let unexpected = tokio::time::timeout(Duration::from_millis(100), async {
        loop {
            let event = read_agent_event(&mut socket).await?;
            if !matches!(event.payload, AgentEvent::Heartbeat(_)) {
                return Ok::<_, anyhow::Error>(event);
            }
        }
    })
    .await;

    assert!(unexpected.is_err());

    task.abort();
    Ok(())
}

#[tokio::test]
async fn heartbeat_continues_while_waiting_for_login_identity() -> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("ws://{}/agent/ws", listener.local_addr()?);

    let mut config = test_config(url);
    config.heartbeat_interval = Duration::from_millis(40);

    let mut delayed = scenario();
    delayed.identity_delay = Duration::from_millis(180);

    let runtime = AgentRuntime::new(config, FakeEmulatorDriver::new(Vec::new()))
        .with_login_executor(FakeLoginExecutor::new(delayed));
    let task = tokio::spawn(runtime.run());

    let mut socket = accept_authenticated(&listener).await?;
    let _hello = read_agent_event(&mut socket).await?;
    let _snapshot = read_agent_event(&mut socket).await?;

    send_command(
        &mut socket,
        ServerCommand::StartLogin(StartLoginCommand {
            session_no: "LOGIN-SLOW".into(),
            game_account_id: 1001,
            emulator_code: "emu-01".into(),
        }),
    )
    .await?;

    let mut saw_qr = false;
    let mut saw_heartbeat = false;
    let mut saw_identity = false;

    for _ in 0..12 {
        let event = read_agent_event(&mut socket).await?;
        match event.payload {
            AgentEvent::LoginQrReady(value) if value.session_no == "LOGIN-SLOW" => {
                saw_qr = true;
            }
            AgentEvent::Heartbeat(_) => saw_heartbeat = true,
            AgentEvent::LoginIdentityDetected(value) if value.session_no == "LOGIN-SLOW" => {
                saw_identity = true;
                break;
            }
            _ => {}
        }
    }

    assert!(saw_qr);
    assert!(saw_heartbeat);
    assert!(saw_identity);

    task.abort();
    Ok(())
}

#[tokio::test]
async fn duplicate_start_login_executes_once_and_replays_terminal_identity() -> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("ws://{}/agent/ws", listener.local_addr()?);
    let executor = FakeLoginExecutor::new(scenario());
    let observer = executor.clone();

    let runtime = AgentRuntime::new(test_config(url), FakeEmulatorDriver::new(Vec::new()))
        .with_login_executor(executor);
    let task = tokio::spawn(runtime.run());

    let mut socket = accept_authenticated(&listener).await?;
    let _hello = read_agent_event(&mut socket).await?;
    let _snapshot = read_agent_event(&mut socket).await?;

    let command_id = Uuid::new_v4();
    let command = ServerCommand::StartLogin(StartLoginCommand {
        session_no: "LOGIN-DEDUPE".into(),
        game_account_id: 1001,
        emulator_code: "emu-01".into(),
    });

    send_command_with_id(&mut socket, command_id, command.clone()).await?;
    send_command_with_id(&mut socket, command_id, command.clone()).await?;

    let mut saw_identity = false;
    for _ in 0..6 {
        let event = read_non_heartbeat_event(&mut socket).await?;
        if matches!(
            event.payload,
            AgentEvent::LoginIdentityDetected(ref value)
                if value.session_no == "LOGIN-DEDUPE"
        ) {
            saw_identity = true;
            break;
        }
    }

    assert!(saw_identity);
    assert_eq!(observer.prepare_count(), 1);

    send_command_with_id(&mut socket, command_id, command).await?;
    let replay = read_non_heartbeat_event(&mut socket).await?;

    assert!(matches!(
        replay.payload,
        AgentEvent::LoginIdentityDetected(ref value)
            if value.session_no == "LOGIN-DEDUPE"
    ));
    assert_eq!(observer.prepare_count(), 1);

    task.abort();
    Ok(())
}
