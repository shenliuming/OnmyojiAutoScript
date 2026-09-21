use std::time::Duration;

use foster_protocol::{
    AgentEnvelope, AgentEvent, AgentHello, EmulatorDescriptor, EmulatorSnapshot,
    Heartbeat, PROTOCOL_VERSION,
};
use foster_server::{
    agent_gateway::registry::AgentRegistry,
    app::{AppState, build_app},
    config::AgentGatewayConfig,
};
use futures_util::SinkExt;
use sqlx::MySqlPool;
use tokio::net::TcpListener;
use tokio_tungstenite::{
    WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use uuid::Uuid;

fn gateway_config() -> AgentGatewayConfig {
    AgentGatewayConfig {
        agent_token: "test-token".into(),
        heartbeat_timeout: Duration::from_secs(2),
        sweep_interval: Duration::from_millis(100),
    }
}

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
) -> anyhow::Result<
    WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
> {
    let mut request = format!("ws://{address}/agent/ws").into_client_request()?;
    request.headers_mut().insert(
        http::header::AUTHORIZATION,
        "Bearer test-token".parse().unwrap(),
    );

    let (socket, _) = connect_async(request).await?;
    Ok(socket)
}

async fn send_event<S>(
    socket: &mut WebSocketStream<S>,
    event: AgentEvent,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let envelope = AgentEnvelope {
        protocol_version: PROTOCOL_VERSION,
        event_id: Uuid::new_v4(),
        sent_at: chrono::Utc::now(),
        payload: event,
    };

    socket
        .send(Message::Text(serde_json::to_string(&envelope)?.into()))
        .await?;

    Ok(())
}

async fn send_hello<S>(
    socket: &mut WebSocketStream<S>,
    host_id: i64,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    send_event(
        socket,
        AgentEvent::Hello(AgentHello {
            agent_id: "agent-01".into(),
            host_id,
            agent_version: "0.1.0".into(),
            hostname: "win-host".into(),
            os_version: "windows".into(),
            capabilities: vec!["EMULATOR_DISCOVERY".into()],
        }),
    )
    .await
}

async fn wait_online(registry: &AgentRegistry, host_id: i64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);

    while !registry.is_online(host_id) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "host did not become online"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn hello_updates_existing_host_online(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    seed_host(&pool, 7).await?;
    let (address, registry) = spawn_app(pool.clone()).await?;
    let mut socket = connect(address).await?;

    send_hello(&mut socket, 7).await?;
    wait_online(&registry, 7).await;

    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT status, agent_version
         FROM host
         WHERE id = 7",
    )
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.0, "ONLINE");
    assert_eq!(row.1.as_deref(), Some("0.1.0"));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn hello_for_unknown_host_is_rejected_without_insert(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let (address, registry) = spawn_app(pool.clone()).await?;
    let mut socket = connect(address).await?;

    send_hello(&mut socket, 999).await?;
    tokio::time::sleep(Duration::from_millis(75)).await;

    assert!(!registry.is_online(999));

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM host WHERE id = 999")
            .fetch_one(&pool)
            .await?;
    assert_eq!(count, 0);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn heartbeat_updates_last_heartbeat_at(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    seed_host(&pool, 7).await?;
    let (address, registry) = spawn_app(pool.clone()).await?;
    let mut socket = connect(address).await?;

    send_hello(&mut socket, 7).await?;
    wait_online(&registry, 7).await;

    sqlx::query(
        "UPDATE host
         SET last_heartbeat_at = '2000-01-01 00:00:00.000'
         WHERE id = 7",
    )
    .execute(&pool)
    .await?;

    send_event(
        &mut socket,
        AgentEvent::Heartbeat(Heartbeat {
            host_id: 7,
            emulators: Vec::new(),
        }),
    )
    .await?;

    tokio::time::sleep(Duration::from_millis(75)).await;

    let updated: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM host
         WHERE id = 7
           AND last_heartbeat_at > '2001-01-01 00:00:00.000'",
    )
    .fetch_one(&pool)
    .await?;

    assert_eq!(updated, 1);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn emulator_snapshot_inserts_new_emulator(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    seed_host(&pool, 7).await?;
    let (address, registry) = spawn_app(pool.clone()).await?;
    let mut socket = connect(address).await?;

    send_hello(&mut socket, 7).await?;
    wait_online(&registry, 7).await;

    send_event(
        &mut socket,
        AgentEvent::EmulatorSnapshot(EmulatorSnapshot {
            host_id: 7,
            emulators: vec![EmulatorDescriptor {
                emulator_code: "emu-01".into(),
                driver_type: "FAKE".into(),
                adb_serial: Some("127.0.0.1:5555".into()),
            }],
        }),
    )
    .await?;

    tokio::time::sleep(Duration::from_millis(75)).await;

    let row: (i64, String, Option<String>) = sqlx::query_as(
        "SELECT host_id, driver_type, adb_serial
         FROM emulator_instance
         WHERE emulator_code = 'emu-01'",
    )
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.0, 7);
    assert_eq!(row.1, "FAKE");
    assert_eq!(row.2.as_deref(), Some("127.0.0.1:5555"));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn emulator_snapshot_preserves_server_controlled_fields(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    seed_host(&pool, 7).await?;

    sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type, max_account_count,
            status, adb_serial, current_job_id
         )
         VALUES (
            7, 'emu-01', 'OLD', 9,
            'IDLE', 'old-adb', 'job-1'
         )",
    )
    .execute(&pool)
    .await?;

    let (address, registry) = spawn_app(pool.clone()).await?;
    let mut socket = connect(address).await?;

    send_hello(&mut socket, 7).await?;
    wait_online(&registry, 7).await;

    send_event(
        &mut socket,
        AgentEvent::EmulatorSnapshot(EmulatorSnapshot {
            host_id: 7,
            emulators: vec![EmulatorDescriptor {
                emulator_code: "emu-01".into(),
                driver_type: "FAKE".into(),
                adb_serial: Some("new-adb".into()),
            }],
        }),
    )
    .await?;

    tokio::time::sleep(Duration::from_millis(75)).await;

    let row: (String, Option<String>, i32, Option<String>) =
        sqlx::query_as(
            "SELECT driver_type, adb_serial, max_account_count, current_job_id
             FROM emulator_instance
             WHERE emulator_code = 'emu-01'",
        )
        .fetch_one(&pool)
        .await?;

    assert_eq!(row.0, "FAKE");
    assert_eq!(row.1.as_deref(), Some("new-adb"));
    assert_eq!(row.2, 9);
    assert_eq!(row.3.as_deref(), Some("job-1"));

    Ok(())
}
