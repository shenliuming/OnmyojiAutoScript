use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use foster_domain::{FosterErrorCode, ResourceMode};
use foster_protocol::{
    AgentEvent, FosterDetectedIdentity, FosterFailed, FosterStage, FosterStageChanged,
    FosterSucceeded, ServerCommand,
};
use foster_server::{
    agent_gateway::registry::{AgentPresence, AgentRegistry},
    foster_dispatch::{DispatchFosterResult, FosterDispatchService},
};
use sqlx::MySqlPool;
use tokio::sync::mpsc;
use uuid::Uuid;

struct Fixture {
    host_id: i64,
    account_id: i64,
    subscription_id: i64,
    job_id: i64,
}

async fn seed_fixture(pool: &MySqlPool, resource_mode: &str) -> anyhow::Result<Fixture> {
    let host_id = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('host-foster-dispatch', 'host-foster-dispatch', 'ONLINE')",
    )
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let emulator_id = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type, max_account_count, status
         )
         VALUES (?, 'emu-foster-dispatch', 'FAKE', 5, 'IDLE')",
    )
    .bind(host_id)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let account_id = sqlx::query(
        "INSERT INTO game_account(
            customer_id, character_name, server_name, game_uid,
            login_status, verify_status, active_emulator_id
         )
         VALUES (91001, '角色A', '春之樱', 'uid-1',
                 'LOGGED_IN', 'VERIFIED', ?)",
    )
    .bind(emulator_id)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    sqlx::query(
        "INSERT INTO emulator_account_binding(
            emulator_id, game_account_id, slot_no, status, bound_at
         )
         VALUES (?, ?, 1, 'ACTIVE', NOW(3))",
    )
    .bind(emulator_id)
    .bind(account_id)
    .execute(pool)
    .await?;

    for (kind, value, normalized) in [
        ("MASKED_ACCOUNT", "12****34", "12****34"),
        ("OCR_ALIAS", "12****S4", "12****s4"),
        ("CHARACTER_NAME", "角色A", "角色A"),
        ("SERVER_NAME", "春之樱", "春之樱"),
        ("GAME_UID", "uid-1", "uid-1"),
    ] {
        sqlx::query(
            "INSERT INTO game_account_identity(
                game_account_id, identity_type, identity_value,
                normalized_value, source, confidence, enabled
             )
             VALUES (?, ?, ?, ?, 'TEST', 100, 1)",
        )
        .bind(account_id)
        .bind(kind)
        .bind(value)
        .bind(normalized)
        .execute(pool)
        .await?;
    }

    let plan_id = sqlx::query(
        "INSERT INTO foster_plan(
            plan_code, plan_name, daily_target_runs,
            interval_minutes, resource_mode, status
         )
         VALUES ('PLAN-DISPATCH', 'Plan Dispatch', 4, 360, ?, 'ACTIVE')",
    )
    .bind(resource_mode)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let base = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let subscription_id = sqlx::query(
        "INSERT INTO foster_subscription(
            subscription_no, game_account_id, plan_id,
            resource_mode, daily_target_runs, interval_minutes,
            status, start_at, end_at, next_run_at
         )
         VALUES ('SUB-DISPATCH', ?, ?, ?, 4, 360,
                 'ACTIVE', ?, ?, NULL)",
    )
    .bind(account_id)
    .bind(plan_id)
    .bind(resource_mode)
    .bind((base - chrono::Duration::days(1)).naive_utc())
    .bind((base + chrono::Duration::days(30)).naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let job_id = sqlx::query(
        "INSERT INTO foster_job(
            job_no, subscription_id, game_account_id, emulator_id,
            status, scheduled_at, started_at
         )
         VALUES ('JOB-DISPATCH', ?, ?, ?,
                 'SWITCHING_ACCOUNT', ?, ?)",
    )
    .bind(subscription_id)
    .bind(account_id)
    .bind(emulator_id)
    .bind((base - chrono::Duration::minutes(1)).naive_utc())
    .bind(base.naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    Ok(Fixture {
        host_id,
        account_id,
        subscription_id,
        job_id,
    })
}

fn online_registry(host_id: i64) -> (AgentRegistry, mpsc::UnboundedReceiver<foster_server::agent_gateway::registry::OutboundMessage>) {
    let registry = AgentRegistry::default();
    let (sender, receiver) = mpsc::unbounded_channel();

    registry.register_with_sender(
        AgentPresence {
            connection_id: Uuid::new_v4(),
            agent_id: "agent-test".into(),
            host_id,
            connected_at: Instant::now(),
            last_heartbeat_at: Instant::now(),
        },
        sender,
    );

    (registry, receiver)
}

#[sqlx::test(migrations = "../../migrations")]
async fn dispatch_builds_passwordless_identity_command(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "USER_FRIEND").await?;
    let service = FosterDispatchService::new(pool.clone());
    let (registry, mut receiver) = online_registry(fixture.host_id);

    let delivery = tokio::spawn(async move {
        let outbound = receiver.recv().await.expect("command");
        let command = outbound.envelope.payload.clone();
        let _ = outbound.delivered.send(Ok(()));
        command
    });

    let result = service.dispatch_job(fixture.job_id, &registry).await?;
    assert_eq!(result, DispatchFosterResult::Dispatched);

    let command = delivery.await?;
    let ServerCommand::ExecuteFoster(command) = command else {
        panic!("expected ExecuteFoster");
    };

    assert_eq!(command.job_id, fixture.job_id);
    assert_eq!(command.attempt, 0);
    assert_eq!(command.game_account_id, fixture.account_id);
    assert_eq!(command.resource_mode, ResourceMode::UserFriend);
    assert_eq!(
        command.target_identity.masked_account.as_deref(),
        Some("12****34")
    );
    assert!(command
        .target_identity
        .account_aliases
        .contains(&"12****S4".to_string()));
    assert_eq!(
        command.target_identity.character_name.as_deref(),
        Some("角色A")
    );
    assert_eq!(command.target_identity.server_name.as_deref(), Some("春之樱"));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn offline_agent_moves_job_to_waiting_emulator(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "USER_FRIEND").await?;
    let service = FosterDispatchService::new(pool.clone());

    let result = service
        .dispatch_job(fixture.job_id, &AgentRegistry::default())
        .await?;

    assert_eq!(result, DispatchFosterResult::WaitingEmulator);

    let status: String = sqlx::query_scalar("SELECT status FROM foster_job WHERE id = ?")
        .bind(fixture.job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "WAITING_EMULATOR");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn success_event_verifies_identity_and_schedules_next_run(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "USER_FRIEND").await?;
    let service = FosterDispatchService::new(pool.clone());
    let at = Utc.with_ymd_and_hms(2026, 9, 22, 12, 5, 0).unwrap();

    service
        .process_agent_event(
            fixture.host_id,
            &AgentEvent::FosterStageChanged(FosterStageChanged {
                job_id: fixture.job_id,
                attempt: 0,
                stage: FosterStage::VerifyingAccount,
                occurred_at: at,
            }),
        )
        .await?;
    service
        .process_agent_event(
            fixture.host_id,
            &AgentEvent::FosterStageChanged(FosterStageChanged {
                job_id: fixture.job_id,
                attempt: 0,
                stage: FosterStage::Running,
                occurred_at: at + chrono::Duration::seconds(1),
            }),
        )
        .await?;
    service
        .process_agent_event(
            fixture.host_id,
            &AgentEvent::FosterSucceeded(FosterSucceeded {
                job_id: fixture.job_id,
                attempt: 0,
                completed_at: at + chrono::Duration::seconds(10),
                remaining_seconds: Some(1_800),
                screenshot_url: Some("file:///success.png".into()),
                detected_identity: FosterDetectedIdentity {
                    masked_account: Some("12****34".into()),
                    character_name: Some("角色A".into()),
                    server_name: Some("春之樱".into()),
                    game_uid: Some("uid-1".into()),
                },
            }),
        )
        .await?;

    let job: (String, Option<String>) =
        sqlx::query_as("SELECT status, screenshot_url FROM foster_job WHERE id = ?")
            .bind(fixture.job_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(job.0, "SUCCESS");
    assert_eq!(job.1.as_deref(), Some("file:///success.png"));

    let next_run_at: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
        "SELECT next_run_at FROM foster_subscription WHERE id = ?",
    )
    .bind(fixture.subscription_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        next_run_at,
        Some((at + chrono::Duration::seconds(1_810)).naive_utc())
    );

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn identity_mismatch_success_event_suspends_account(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "USER_FRIEND").await?;
    let service = FosterDispatchService::new(pool.clone());
    let at = Utc.with_ymd_and_hms(2026, 9, 22, 12, 5, 0).unwrap();

    service
        .process_agent_event(
            fixture.host_id,
            &AgentEvent::FosterSucceeded(FosterSucceeded {
                job_id: fixture.job_id,
                attempt: 0,
                completed_at: at,
                remaining_seconds: Some(1_800),
                screenshot_url: None,
                detected_identity: FosterDetectedIdentity {
                    masked_account: Some("99****99".into()),
                    character_name: Some("角色B".into()),
                    server_name: Some("春之樱".into()),
                    game_uid: None,
                },
            }),
        )
        .await?;

    let status: String = sqlx::query_scalar("SELECT status FROM foster_job WHERE id = ?")
        .bind(fixture.job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "IDENTITY_MISMATCH");

    let subscription_status: String =
        sqlx::query_scalar("SELECT status FROM foster_subscription WHERE id = ?")
            .bind(fixture.subscription_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(subscription_status, "SUSPENDED");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn network_failure_from_switching_stage_retries(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "USER_FRIEND").await?;
    let service = FosterDispatchService::new(pool.clone());
    let at = Utc.with_ymd_and_hms(2026, 9, 22, 12, 5, 0).unwrap();

    service
        .process_agent_event(
            fixture.host_id,
            &AgentEvent::FosterFailed(FosterFailed {
                job_id: fixture.job_id,
                attempt: 0,
                failed_at: at,
                error_code: FosterErrorCode::NetworkError,
                message: "connection refused".into(),
                screenshot_url: None,
            }),
        )
        .await?;

    let row: (String, i32, Option<chrono::NaiveDateTime>) =
        sqlx::query_as("SELECT status, retry_count, retry_after FROM foster_job WHERE id = ?")
            .bind(fixture.job_id)
            .fetch_one(&pool)
            .await?;

    assert_eq!(row.0, "RETRY");
    assert_eq!(row.1, 1);
    assert_eq!(
        row.2,
        Some((at + chrono::Duration::minutes(5)).naive_utc())
    );

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn duplicate_terminal_success_event_is_ignored(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "USER_FRIEND").await?;
    let service = FosterDispatchService::new(pool.clone());
    let at = Utc.with_ymd_and_hms(2026, 9, 22, 12, 5, 0).unwrap();
    let event = AgentEvent::FosterSucceeded(FosterSucceeded {
        job_id: fixture.job_id,
        attempt: 0,
        completed_at: at,
        remaining_seconds: Some(1_800),
        screenshot_url: None,
        detected_identity: FosterDetectedIdentity {
            masked_account: Some("12****34".into()),
            character_name: Some("角色A".into()),
            server_name: Some("春之樱".into()),
            game_uid: Some("uid-1".into()),
        },
    });

    service
        .process_agent_event(fixture.host_id, &event)
        .await?;
    service
        .process_agent_event(fixture.host_id, &event)
        .await?;

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM foster_job WHERE id = ? AND status = 'SUCCESS'",
    )
    .bind(fixture.job_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(count, 1);

    Ok(())
}


#[sqlx::test(migrations = "../../migrations")]
async fn platform_job_waits_for_resource_phase(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "PLATFORM").await?;
    let service = FosterDispatchService::new(pool.clone());

    let result = service
        .dispatch_job(fixture.job_id, &AgentRegistry::default())
        .await?;

    assert_eq!(result, DispatchFosterResult::UnsupportedPlatform);

    let row: (String, Option<String>) =
        sqlx::query_as("SELECT status, error_code FROM foster_job WHERE id = ?")
            .bind(fixture.job_id)
            .fetch_one(&pool)
            .await?;

    assert_eq!(row.0, "WAITING_RESOURCE");
    assert_eq!(row.1.as_deref(), Some("PROVIDER_NOT_FOUND"));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn stale_attempt_success_is_ignored(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "USER_FRIEND").await?;
    let service = FosterDispatchService::new(pool.clone());
    let at = Utc.with_ymd_and_hms(2026, 9, 22, 12, 5, 0).unwrap();

    sqlx::query(
        "UPDATE foster_job
         SET retry_count = 1
         WHERE id = ?",
    )
    .bind(fixture.job_id)
    .execute(&pool)
    .await?;

    service
        .process_agent_event(
            fixture.host_id,
            &AgentEvent::FosterSucceeded(FosterSucceeded {
                job_id: fixture.job_id,
                attempt: 0,
                completed_at: at,
                remaining_seconds: Some(1_800),
                screenshot_url: None,
                detected_identity: FosterDetectedIdentity {
                    masked_account: Some("12****34".into()),
                    character_name: Some("角色A".into()),
                    server_name: Some("春之樱".into()),
                    game_uid: Some("uid-1".into()),
                },
            }),
        )
        .await?;

    let status: String = sqlx::query_scalar("SELECT status FROM foster_job WHERE id = ?")
        .bind(fixture.job_id)
        .fetch_one(&pool)
        .await?;

    assert_eq!(status, "SWITCHING_ACCOUNT");
    Ok(())
}
