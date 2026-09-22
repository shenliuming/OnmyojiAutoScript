use std::time::Duration;

use foster_domain::EmulatorStatus;
use foster_protocol::{
    AgentEnvelope, AgentEvent, AgentHello, EmulatorDescriptor, EmulatorHeartbeat,
    EmulatorSnapshot, Heartbeat, PROTOCOL_VERSION,
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

async fn spawn_app(pool: MySqlPool) -> anyhow::Result<std::net::SocketAddr> {
    let app = build_app(AppState {
        pool,
        registry: AgentRegistry::default(),
        gateway_config: gateway_config(),
    });

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("test server failed");
    });

    Ok(address)
}

async fn connect(
    address: std::net::SocketAddr,
) -> anyhow::Result<WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>> {
    let mut request = format!("ws://{address}/agent/ws").into_client_request()?;
    request.headers_mut().insert(
        http::header::AUTHORIZATION,
        "Bearer test-token".parse().unwrap(),
    );

    let (socket, _) = connect_async(request).await?;
    Ok(socket)
}

fn envelope(payload: AgentEvent) -> AgentEnvelope {
    AgentEnvelope {
        protocol_version: PROTOCOL_VERSION,
        event_id: Uuid::new_v4(),
        sent_at: chrono::Utc::now(),
        payload,
    }
}

async fn send_event<S>(socket: &mut WebSocketStream<S>, payload: AgentEvent) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    socket
        .send(Message::Text(
            serde_json::to_string(&envelope(payload))?.into(),
        ))
        .await?;
    Ok(())
}

async fn send_hello<S>(socket: &mut WebSocketStream<S>, host_id: i64) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    send_event(
        socket,
        AgentEvent::Hello(AgentHello {
            agent_id: "agent-01".into(),
            host_id,
            agent_version: "0.2.0".into(),
            hostname: "win-host".into(),
            os_version: "windows".into(),
            capabilities: vec!["EMULATOR_DISCOVERY".into()],
        }),
    )
    .await
}

async fn wait_until(timeout: Duration, mut predicate: impl AsyncFnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if predicate().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn hello_updates_existing_host_online(pool: MySqlPool) -> anyhow::Result<()> {
    seed_host(&pool, 7).await?;
    let address = spawn_app(pool.clone()).await?;

    let mut socket = connect(address).await?;
    send_hello(&mut socket, 7).await?;

    assert!(
        wait_until(Duration::from_secs(1), async || {
            let row: Option<(String, Option<String>)> =
                sqlx::query_as("SELECT status, agent_version FROM host WHERE id = 7")
                    .fetch_optional(&pool)
                    .await
                    .ok()
                    .flatten();

            matches!(
                row,
                Some((status, Some(version)))
                    if status == "ONLINE" && version == "0.2.0"
            )
        })
        .await
    );

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn hello_for_unknown_host_is_rejected_without_insert(pool: MySqlPool) -> anyhow::Result<()> {
    let address = spawn_app(pool.clone()).await?;

    let mut socket = connect(address).await?;
    send_hello(&mut socket, 999).await?;

    tokio::time::sleep(Duration::from_millis(75)).await;

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM host WHERE id = 999")
        .fetch_one(&pool)
        .await?;

    assert_eq!(count, 0);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn heartbeat_updates_last_heartbeat_at(pool: MySqlPool) -> anyhow::Result<()> {
    seed_host(&pool, 7).await?;
    let address = spawn_app(pool.clone()).await?;

    let mut socket = connect(address).await?;
    send_hello(&mut socket, 7).await?;

    assert!(
        wait_until(Duration::from_secs(1), async || {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM host
                 WHERE id = 7 AND last_heartbeat_at IS NOT NULL",
            )
            .fetch_one(&pool)
            .await
            .is_ok_and(|count| count == 1)
        })
        .await
    );

    let before: chrono::NaiveDateTime =
        sqlx::query_scalar("SELECT last_heartbeat_at FROM host WHERE id = 7")
            .fetch_one(&pool)
            .await?;

    tokio::time::sleep(Duration::from_millis(20)).await;

    send_event(
        &mut socket,
        AgentEvent::Heartbeat(Heartbeat {
            host_id: 7,
            emulators: Vec::new(),
        }),
    )
    .await?;

    assert!(
        wait_until(Duration::from_secs(1), async || {
            sqlx::query_scalar::<_, chrono::NaiveDateTime>(
                "SELECT last_heartbeat_at FROM host WHERE id = 7",
            )
            .fetch_one(&pool)
            .await
            .is_ok_and(|value| value > before)
        })
        .await
    );

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn emulator_snapshot_inserts_new_emulator(pool: MySqlPool) -> anyhow::Result<()> {
    seed_host(&pool, 7).await?;
    let address = spawn_app(pool.clone()).await?;

    let mut socket = connect(address).await?;
    send_hello(&mut socket, 7).await?;

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

    assert!(
        wait_until(Duration::from_secs(1), async || {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM emulator_instance
                 WHERE emulator_code = 'emu-01'
                   AND host_id = 7
                   AND driver_type = 'FAKE'",
            )
            .fetch_one(&pool)
            .await
            .is_ok_and(|count| count == 1)
        })
        .await
    );

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

    let address = spawn_app(pool.clone()).await?;
    let mut socket = connect(address).await?;
    send_hello(&mut socket, 7).await?;

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

    assert!(
        wait_until(Duration::from_secs(1), async || {
            let row: Option<(String, Option<String>, i32, Option<String>)> = sqlx::query_as(
                "SELECT driver_type, adb_serial,
                         max_account_count, current_job_id
                  FROM emulator_instance
                  WHERE emulator_code = 'emu-01'",
            )
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten();

            matches!(
                row,
                Some((driver, Some(adb), 9, Some(job)))
                    if driver == "FAKE"
                        && adb == "new-adb"
                        && job == "job-1"
            )
        })
        .await
    );

    Ok(())
}


#[sqlx::test(migrations = "../../migrations")]
async fn heartbeat_updates_emulator_status(pool: MySqlPool) -> anyhow::Result<()> {
    seed_host(&pool, 7).await?;
    sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type, max_account_count,
            status, adb_serial
         )
         VALUES (7, 'emu-status', 'ADB', 5, 'IDLE', '127.0.0.1:5555')",
    )
    .execute(&pool)
    .await?;

    let address = spawn_app(pool.clone()).await?;
    let mut socket = connect(address).await?;
    send_hello(&mut socket, 7).await?;

    send_event(
        &mut socket,
        AgentEvent::Heartbeat(Heartbeat {
            host_id: 7,
            emulators: vec![EmulatorHeartbeat {
                emulator_code: "emu-status".into(),
                status: EmulatorStatus::Offline,
            }],
        }),
    )
    .await?;

    assert!(
        wait_until(Duration::from_secs(1), async || {
            sqlx::query_scalar::<_, String>(
                "SELECT status FROM emulator_instance
                 WHERE emulator_code = 'emu-status'",
            )
            .fetch_one(&pool)
            .await
            .is_ok_and(|status| status == "OFFLINE")
        })
        .await
    );

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn closing_host_marks_emulators_offline(pool: MySqlPool) -> anyhow::Result<()> {
    seed_host(&pool, 7).await?;
    sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type, max_account_count,
            status, adb_serial
         )
         VALUES (7, 'emu-close', 'ADB', 5, 'IDLE', '127.0.0.1:5556')",
    )
    .execute(&pool)
    .await?;

    let address = spawn_app(pool.clone()).await?;
    let mut socket = connect(address).await?;
    send_hello(&mut socket, 7).await?;
    socket.close(None).await?;

    assert!(
        wait_until(Duration::from_secs(1), async || {
            let row: Option<(String, String)> = sqlx::query_as(
                "SELECT h.status, e.status
                 FROM host h
                 JOIN emulator_instance e ON e.host_id = h.id
                 WHERE h.id = 7
                   AND e.emulator_code = 'emu-close'",
            )
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten();

            matches!(
                row,
                Some((host_status, emulator_status))
                    if host_status == "OFFLINE"
                        && emulator_status == "OFFLINE"
            )
        })
        .await
    );

    Ok(())
}
