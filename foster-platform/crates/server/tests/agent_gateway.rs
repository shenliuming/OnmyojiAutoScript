use std::time::Duration;

use axum::body::Body;
use foster_protocol::{
    AgentEnvelope, AgentEvent, AgentHello, PROTOCOL_VERSION,
};
use foster_server::{
    agent_gateway::registry::AgentRegistry,
    app::{AppState, build_app},
    config::AgentGatewayConfig,
};
use futures_util::{SinkExt, StreamExt};
use http::{Request, StatusCode};
use sqlx::MySqlPool;
use tokio::net::TcpListener;
use tokio_tungstenite::{
    WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use tower::ServiceExt;
use uuid::Uuid;

async fn seed_host(pool: &MySqlPool, host_id: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO host(id, host_code, hostname, status)
         VALUES (?, ?, ?, 'OFFLINE')",
    )
    .bind(host_id)
    .bind(format!("HOST-{host_id}"))
    .bind(format!("host-{host_id}"))
    .execute(pool)
    .await?;

    Ok(())
}

fn gateway_config() -> AgentGatewayConfig {
    AgentGatewayConfig {
        agent_token: "test-token".into(),
        heartbeat_timeout: Duration::from_millis(150),
        sweep_interval: Duration::from_millis(25),
    }
}

async fn spawn_app(
    pool: MySqlPool,
) -> anyhow::Result<(std::net::SocketAddr, AgentRegistry)> {
    let registry = AgentRegistry::default();
    let app = build_app(AppState {
        pool,
        registry: registry.clone(),
        gateway_config: gateway_config(),
    });

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("test server failed");
    });

    Ok((address, registry))
}

async fn connect(
    address: std::net::SocketAddr,
    token: Option<&str>,
) -> Result<
    WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::Error,
> {
    let mut request = format!("ws://{address}/agent/ws").into_client_request()?;
    if let Some(token) = token {
        request.headers_mut().insert(
            http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
    }

    let (socket, _) = connect_async(request).await?;
    Ok(socket)
}

async fn send_hello<S>(
    socket: &mut WebSocketStream<S>,
    host_id: i64,
    protocol_version: u16,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let envelope = AgentEnvelope {
        protocol_version,
        event_id: Uuid::new_v4(),
        sent_at: chrono::Utc::now(),
        payload: AgentEvent::Hello(AgentHello {
            agent_id: "agent-01".into(),
            host_id,
            agent_version: "0.1.0".into(),
            hostname: "win-host".into(),
            os_version: "windows".into(),
            capabilities: vec!["EMULATOR_DISCOVERY".into()],
        }),
    };

    socket
        .send(Message::Text(serde_json::to_string(&envelope)?.into()))
        .await?;

    Ok(())
}

async fn wait_until(
    timeout: Duration,
    mut predicate: impl FnMut() -> bool,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if predicate() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn healthz_returns_ok(pool: MySqlPool) -> anyhow::Result<()> {
    let registry = AgentRegistry::default();
    let app = build_app(AppState {
        pool,
        registry,
        gateway_config: gateway_config(),
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn missing_agent_token_is_rejected(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let (address, _) = spawn_app(pool).await?;

    let result = connect(address, None).await;

    assert!(result.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn invalid_agent_token_is_rejected(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let (address, _) = spawn_app(pool).await?;

    let result = connect(address, Some("wrong-token")).await;

    assert!(result.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn valid_hello_registers_known_host(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    seed_host(&pool, 7).await?;
    let (address, registry) = spawn_app(pool).await?;

    let mut socket = connect(address, Some("test-token")).await?;
    send_hello(&mut socket, 7, PROTOCOL_VERSION).await?;

    assert!(
        wait_until(Duration::from_secs(1), || registry.is_online(7)).await
    );

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn unsupported_protocol_version_never_registers_host(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    seed_host(&pool, 7).await?;
    let (address, registry) = spawn_app(pool).await?;

    let mut socket = connect(address, Some("test-token")).await?;
    send_hello(&mut socket, 7, PROTOCOL_VERSION + 1).await?;

    tokio::time::sleep(Duration::from_millis(75)).await;
    assert!(!registry.is_online(7));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn replacement_connection_survives_old_socket_close(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    seed_host(&pool, 7).await?;
    let (address, registry) = spawn_app(pool).await?;

    let mut first = connect(address, Some("test-token")).await?;
    send_hello(&mut first, 7, PROTOCOL_VERSION).await?;
    assert!(
        wait_until(Duration::from_secs(1), || registry.is_online(7)).await
    );

    let mut second = connect(address, Some("test-token")).await?;
    send_hello(&mut second, 7, PROTOCOL_VERSION).await?;

    assert!(
        wait_until(Duration::from_secs(1), || {
            registry.is_online(7) && registry.connection_count() == 1
        })
        .await
    );

    first.close(None).await?;

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(registry.is_online(7));
    assert_eq!(registry.connection_count(), 1);

    second.close(None).await?;
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn host_becomes_offline_after_heartbeat_timeout(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    seed_host(&pool, 7).await?;
    let (address, registry) = spawn_app(pool).await?;

    let mut socket = connect(address, Some("test-token")).await?;
    send_hello(&mut socket, 7, PROTOCOL_VERSION).await?;

    assert!(
        wait_until(Duration::from_secs(1), || registry.is_online(7)).await
    );

    assert!(
        wait_until(Duration::from_secs(1), || !registry.is_online(7)).await
    );

    let _ = socket.next().await;
    Ok(())
}
