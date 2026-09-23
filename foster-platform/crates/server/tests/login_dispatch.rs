use std::time::{Duration, Instant};

use foster_protocol::ServerCommand;
use foster_server::{
    agent_gateway::registry::{AgentPresence, AgentRegistry},
    enrollment::{DispatchLoginResult, EnrollmentService, login_command_id},
};
use sqlx::MySqlPool;
use uuid::Uuid;

struct Fixture {
    host_id: i64,
    account_id: i64,
    session_no: String,
}

async fn seed_fixture(pool: &MySqlPool, suffix: &str) -> anyhow::Result<Fixture> {
    let host = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES (?, ?, 'ONLINE')",
    )
    .bind(format!("host-dispatch-{suffix}"))
    .bind(format!("host-dispatch-{suffix}"))
    .execute(pool)
    .await?;
    let host_id = host.last_insert_id() as i64;

    sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status
         )
         VALUES (?, ?, 'FAKE', 5, 'IDLE')",
    )
    .bind(host_id)
    .bind(format!("emu-dispatch-{suffix}"))
    .execute(pool)
    .await?;

    let account = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status
         )
         VALUES (?, 'PENDING', 'PENDING')",
    )
    .bind(10000_i64 + host_id)
    .execute(pool)
    .await?;
    let account_id = account.last_insert_id() as i64;

    let created = EnrollmentService::new(pool.clone())
        .create_login_session(account_id, Duration::from_secs(900))
        .await?;

    Ok(Fixture {
        host_id,
        account_id,
        session_no: created.session_no,
    })
}

fn presence(host_id: i64, connection_id: Uuid) -> AgentPresence {
    let now = Instant::now();
    AgentPresence {
        connection_id,
        agent_id: format!("agent-{host_id}"),
        host_id,
        connected_at: now,
        last_heartbeat_at: now,
    }
}

#[test]
fn login_command_id_is_stable_per_session() {
    assert_eq!(login_command_id("LOGIN-A"), login_command_id("LOGIN-A"));
    assert_ne!(login_command_id("LOGIN-A"), login_command_id("LOGIN-B"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn online_host_receives_start_login(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "online").await?;
    let registry = AgentRegistry::default();
    let connection_id = Uuid::new_v4();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    registry.register_with_sender(presence(fixture.host_id, connection_id), tx);

    let service = EnrollmentService::new(pool.clone());
    let session_no = fixture.session_no.clone();
    let registry_for_dispatch = registry.clone();
    let dispatch = tokio::spawn(async move {
        service
            .dispatch_login_session(&session_no, &registry_for_dispatch)
            .await
    });

    let outbound = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await?
        .expect("current agent should receive START_LOGIN");

    assert_eq!(
        outbound.envelope.command_id,
        login_command_id(&fixture.session_no)
    );

    match &outbound.envelope.payload {
        ServerCommand::StartLogin(command) => {
            assert_eq!(command.session_no, fixture.session_no);
            assert_eq!(command.game_account_id, fixture.account_id);
            assert!(command.emulator_code.starts_with("emu-dispatch-online"));
        }
        other => panic!("expected START_LOGIN, got {other:?}"),
    }

    outbound
        .delivered
        .send(Ok(()))
        .expect("dispatch waiter should still be alive");

    assert_eq!(dispatch.await??, DispatchLoginResult::Dispatched);

    let status: String = sqlx::query_scalar(
        "SELECT status
         FROM login_session
         WHERE session_no = ?",
    )
    .bind(&fixture.session_no)
    .fetch_one(&pool)
    .await?;
    assert_eq!(status, "PREPARING");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn offline_host_leaves_session_waiting_emulator(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "offline").await?;
    let registry = AgentRegistry::default();

    let result = EnrollmentService::new(pool.clone())
        .dispatch_login_session(&fixture.session_no, &registry)
        .await?;

    assert_eq!(result, DispatchLoginResult::WaitingEmulator);

    let status: String = sqlx::query_scalar(
        "SELECT status
         FROM login_session
         WHERE session_no = ?",
    )
    .bind(&fixture.session_no)
    .fetch_one(&pool)
    .await?;
    assert_eq!(status, "WAITING_EMULATOR");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn replacement_connection_receives_new_login_command(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "replacement").await?;
    let registry = AgentRegistry::default();

    let (old_tx, mut old_rx) = tokio::sync::mpsc::unbounded_channel();
    registry.register_with_sender(presence(fixture.host_id, Uuid::new_v4()), old_tx);

    let (new_tx, mut new_rx) = tokio::sync::mpsc::unbounded_channel();
    registry.register_with_sender(presence(fixture.host_id, Uuid::new_v4()), new_tx);

    let service = EnrollmentService::new(pool);
    let session_no = fixture.session_no.clone();
    let registry_for_dispatch = registry.clone();
    let dispatch = tokio::spawn(async move {
        service
            .dispatch_login_session(&session_no, &registry_for_dispatch)
            .await
    });

    assert!(old_rx.try_recv().is_err());

    let outbound = tokio::time::timeout(Duration::from_secs(1), new_rx.recv())
        .await?
        .expect("replacement connection should receive command");
    assert!(matches!(
        outbound.envelope.payload,
        ServerCommand::StartLogin(_)
    ));
    outbound.delivered.send(Ok(())).unwrap();

    assert_eq!(dispatch.await??, DispatchLoginResult::Dispatched);
    assert!(old_rx.try_recv().is_err());

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn duplicate_dispatch_is_idempotent(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "duplicate").await?;
    let registry = AgentRegistry::default();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    registry.register_with_sender(presence(fixture.host_id, Uuid::new_v4()), tx);

    let service = EnrollmentService::new(pool);
    let first_service = service.clone();
    let first_registry = registry.clone();
    let first_session_no = fixture.session_no.clone();
    let first = tokio::spawn(async move {
        first_service
            .dispatch_login_session(&first_session_no, &first_registry)
            .await
    });

    let outbound = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await?
        .expect("first dispatch should send");
    outbound.delivered.send(Ok(())).unwrap();
    assert_eq!(first.await??, DispatchLoginResult::Dispatched);

    let second = service
        .dispatch_login_session(&fixture.session_no, &registry)
        .await?;
    assert_eq!(second, DispatchLoginResult::AlreadyDispatched);
    assert!(rx.try_recv().is_err());

    Ok(())
}
