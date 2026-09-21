use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use foster_agent::{config::AgentConfig, emulator::FakeEmulatorDriver, runtime::AgentRuntime};
use foster_protocol::{
    AgentEnvelope, AgentEvent, EmulatorDescriptor, PROTOCOL_VERSION, RefreshEmulatorsCommand,
    ServerCommand, ServerEnvelope,
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
        agent_id: "agent-01".into(),
        host_id: 7,
        heartbeat_interval: Duration::from_millis(50),
        reconnect_delays: vec![
            Duration::from_millis(10),
            Duration::from_millis(20),
            Duration::from_millis(30),
        ],
    }
}

fn fake_driver() -> FakeEmulatorDriver {
    FakeEmulatorDriver::new(vec![EmulatorDescriptor {
        emulator_code: "emu-01".into(),
        driver_type: "FAKE".into(),
        adb_serial: Some("127.0.0.1:5555".into()),
    }])
}

async fn bind_server() -> anyhow::Result<(TcpListener, String)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    Ok((listener, format!("ws://{address}/agent/ws")))
}

async fn accept_authenticated(
    listener: &TcpListener,
) -> anyhow::Result<tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>> {
    let (stream, _) = listener.accept().await?;
    let socket = accept_hdr_async(stream, |request: &Request, response: Response| {
        assert_eq!(
            request
                .headers()
                .get(http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer test-token")
        );
        Ok(response)
    })
    .await?;
    Ok(socket)
}

async fn read_agent_event(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) -> anyhow::Result<AgentEnvelope> {
    let message = tokio::time::timeout(Duration::from_secs(1), socket.next())
        .await?
        .ok_or_else(|| anyhow::anyhow!("agent socket closed"))??;

    let Message::Text(text) = message else {
        anyhow::bail!("expected text websocket frame");
    };

    Ok(serde_json::from_str(text.as_str())?)
}

#[tokio::test]
async fn agent_sends_hello_after_connect() -> anyhow::Result<()> {
    let (listener, url) = bind_server().await?;
    let runtime = AgentRuntime::new(test_config(url), fake_driver());
    let task = tokio::spawn(runtime.run());

    let mut socket = accept_authenticated(&listener).await?;
    let envelope = read_agent_event(&mut socket).await?;

    assert_eq!(envelope.protocol_version, PROTOCOL_VERSION);
    assert!(matches!(
        envelope.payload,
        AgentEvent::Hello(ref hello)
            if hello.agent_id == "agent-01" && hello.host_id == 7
    ));

    task.abort();
    Ok(())
}

#[tokio::test]
async fn agent_sends_snapshot_after_hello() -> anyhow::Result<()> {
    let (listener, url) = bind_server().await?;
    let runtime = AgentRuntime::new(test_config(url), fake_driver());
    let task = tokio::spawn(runtime.run());

    let mut socket = accept_authenticated(&listener).await?;
    let _hello = read_agent_event(&mut socket).await?;
    let snapshot = read_agent_event(&mut socket).await?;

    assert!(matches!(
        snapshot.payload,
        AgentEvent::EmulatorSnapshot(ref value)
            if value.host_id == 7
                && value.emulators.len() == 1
                && value.emulators[0].emulator_code == "emu-01"
    ));

    task.abort();
    Ok(())
}

#[tokio::test]
async fn agent_sends_heartbeat_on_interval() -> anyhow::Result<()> {
    let (listener, url) = bind_server().await?;
    let runtime = AgentRuntime::new(test_config(url), fake_driver());
    let task = tokio::spawn(runtime.run());

    let mut socket = accept_authenticated(&listener).await?;
    let _hello = read_agent_event(&mut socket).await?;
    let _snapshot = read_agent_event(&mut socket).await?;
    let heartbeat = read_agent_event(&mut socket).await?;

    assert!(matches!(
        heartbeat.payload,
        AgentEvent::Heartbeat(ref value) if value.host_id == 7
    ));

    task.abort();
    Ok(())
}

#[tokio::test]
async fn refresh_emulators_sends_fresh_snapshot() -> anyhow::Result<()> {
    let (listener, url) = bind_server().await?;
    let runtime = AgentRuntime::new(test_config(url), fake_driver());
    let task = tokio::spawn(runtime.run());

    let mut socket = accept_authenticated(&listener).await?;
    let _hello = read_agent_event(&mut socket).await?;
    let _snapshot = read_agent_event(&mut socket).await?;

    let command = ServerEnvelope {
        protocol_version: PROTOCOL_VERSION,
        command_id: Uuid::new_v4(),
        sent_at: chrono::Utc::now(),
        payload: ServerCommand::RefreshEmulators(RefreshEmulatorsCommand::default()),
    };

    socket
        .send(Message::Text(serde_json::to_string(&command)?.into()))
        .await?;

    let refreshed = loop {
        let event = read_agent_event(&mut socket).await?;
        if matches!(event.payload, AgentEvent::EmulatorSnapshot(_)) {
            break event;
        }
    };

    assert!(matches!(
        refreshed.payload,
        AgentEvent::EmulatorSnapshot(ref value)
            if value.emulators.len() == 1
                && value.emulators[0].emulator_code == "emu-01"
    ));

    task.abort();
    Ok(())
}

#[tokio::test]
async fn agent_reconnects_and_sends_hello_again() -> anyhow::Result<()> {
    let (listener, url) = bind_server().await?;
    let runtime = AgentRuntime::new(test_config(url), fake_driver());
    let task = tokio::spawn(runtime.run());

    let connections = Arc::new(AtomicUsize::new(0));

    let mut first = accept_authenticated(&listener).await?;
    connections.fetch_add(1, Ordering::SeqCst);
    let first_hello = read_agent_event(&mut first).await?;
    first.close(None).await?;

    let mut second =
        tokio::time::timeout(Duration::from_secs(1), accept_authenticated(&listener)).await??;
    connections.fetch_add(1, Ordering::SeqCst);
    let second_hello = read_agent_event(&mut second).await?;

    let first_identity = match first_hello.payload {
        AgentEvent::Hello(value) => (value.agent_id, value.host_id),
        other => panic!("expected HELLO, got {other:?}"),
    };
    let second_identity = match second_hello.payload {
        AgentEvent::Hello(value) => (value.agent_id, value.host_id),
        other => panic!("expected HELLO, got {other:?}"),
    };

    assert_eq!(first_identity, second_identity);
    assert_eq!(connections.load(Ordering::SeqCst), 2);

    task.abort();
    Ok(())
}
