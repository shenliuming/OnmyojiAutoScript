use std::time::Instant;

use chrono::{TimeZone, Utc};
use foster_domain::{FosterErrorCode, ResourceMode, ResourceType};
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

    let resource_type = (resource_mode == "PLATFORM").then_some("FISH");

    let plan_id = sqlx::query(
        "INSERT INTO foster_plan(
            plan_code, plan_name, daily_target_runs,
            interval_minutes, resource_mode, resource_type, status
         )
         VALUES ('PLAN-DISPATCH', 'Plan Dispatch', 4, 360, ?, ?, 'ACTIVE')",
    )
    .bind(resource_mode)
    .bind(resource_type)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let base = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let subscription_id = sqlx::query(
        "INSERT INTO foster_subscription(
            subscription_no, game_account_id, plan_id,
            resource_mode, resource_type, daily_target_runs, interval_minutes,
            status, start_at, end_at, next_run_at
         )
         VALUES ('SUB-DISPATCH', ?, ?, ?, ?, 4, 360,
                 'ACTIVE', ?, ?, NULL)",
    )
    .bind(account_id)
    .bind(plan_id)
    .bind(resource_mode)
    .bind(resource_type)
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

async fn seed_platform_resource(
    pool: &MySqlPool,
    account_id: i64,
    base: chrono::DateTime<Utc>,
    alias: &str,
) -> anyhow::Result<(i64, i64)> {
    let provider_id = sqlx::query(
        "INSERT INTO provider_account(
            provider_code, nickname, provider_alias, server_name, status
         )
         VALUES (?, '资源号', ?, '春之樱', 'ACTIVE')",
    )
    .bind(format!("PROVIDER-{alias}"))
    .bind(alias)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    sqlx::query(
        "INSERT INTO foster_friend_binding(
            game_account_id, provider_account_id, status, verified_at
         )
         VALUES (?, ?, 'VERIFIED', ?)",
    )
    .bind(account_id)
    .bind(provider_id)
    .bind(base.naive_utc())
    .execute(pool)
    .await?;

    let cycle_id = sqlx::query(
        "INSERT INTO foster_resource_cycle(
            provider_account_id, resource_type, resource_level,
            start_at, end_at, slot_capacity, occupied_slots, status
         )
         VALUES (?, 'FISH', 6, ?, ?, 1, 0, 'AVAILABLE')",
    )
    .bind(provider_id)
    .bind((base - chrono::Duration::hours(1)).naive_utc())
    .bind((base + chrono::Duration::hours(8)).naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    Ok((provider_id, cycle_id))
}

fn online_registry(
    host_id: i64,
) -> (
    AgentRegistry,
    mpsc::UnboundedReceiver<foster_server::agent_gateway::registry::OutboundMessage>,
) {
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
    assert!(
        command
            .target_identity
            .account_aliases
            .contains(&"12****S4".to_string())
    );
    assert_eq!(
        command.target_identity.character_name.as_deref(),
        Some("角色A")
    );
    assert_eq!(
        command.target_identity.server_name.as_deref(),
        Some("春之樱")
    );

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

    let next_run_at: Option<chrono::NaiveDateTime> =
        sqlx::query_scalar("SELECT next_run_at FROM foster_subscription WHERE id = ?")
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
async fn identity_mismatch_success_event_suspends_account(pool: MySqlPool) -> anyhow::Result<()> {
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
    assert_eq!(row.2, Some((at + chrono::Duration::minutes(5)).naive_utc()));

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

    service.process_agent_event(fixture.host_id, &event).await?;
    service.process_agent_event(fixture.host_id, &event).await?;

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM foster_job WHERE id = ? AND status = 'SUCCESS'")
            .bind(fixture.job_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(count, 1);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn platform_job_waits_when_no_resource_is_available(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "PLATFORM").await?;
    let service = FosterDispatchService::new(pool.clone());

    let result = service
        .dispatch_job_at(
            fixture.job_id,
            &AgentRegistry::default(),
            Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap(),
        )
        .await?;

    assert_eq!(result, DispatchFosterResult::WaitingResource);

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

#[sqlx::test(migrations = "../../migrations")]
async fn platform_dispatch_sends_exact_provider_and_confirms_on_success(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "PLATFORM").await?;
    let base = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let (_provider_id, cycle_id) =
        seed_platform_resource(&pool, fixture.account_id, base, "资源A01").await?;

    let service = FosterDispatchService::new(pool.clone());
    let (registry, mut receiver) = online_registry(fixture.host_id);

    let delivery = tokio::spawn(async move {
        let outbound = receiver.recv().await.expect("command");
        let command = outbound.envelope.payload.clone();
        let _ = outbound.delivered.send(Ok(()));
        command
    });

    let result = service
        .dispatch_job_at(fixture.job_id, &registry, base)
        .await?;
    assert_eq!(result, DispatchFosterResult::Dispatched);

    let command = delivery.await?;
    let ServerCommand::ExecuteFoster(command) = command else {
        panic!("expected ExecuteFoster");
    };
    assert_eq!(command.resource_mode, ResourceMode::Platform);
    assert_eq!(command.resource_type, Some(ResourceType::Fish));
    assert_eq!(command.provider_alias.as_deref(), Some("资源A01"));

    let occupied: i32 =
        sqlx::query_scalar("SELECT occupied_slots FROM foster_resource_cycle WHERE id = ?")
            .bind(cycle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(occupied, 1);

    let completed_at = base + chrono::Duration::minutes(2);
    service
        .process_agent_event(
            fixture.host_id,
            &AgentEvent::FosterStageChanged(FosterStageChanged {
                job_id: fixture.job_id,
                attempt: 0,
                stage: FosterStage::VerifyingAccount,
                occurred_at: completed_at - chrono::Duration::seconds(2),
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
                occurred_at: completed_at - chrono::Duration::seconds(1),
            }),
        )
        .await?;
    service
        .process_agent_event(
            fixture.host_id,
            &AgentEvent::FosterSucceeded(FosterSucceeded {
                job_id: fixture.job_id,
                attempt: 0,
                completed_at,
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

    let allocation: (String, Option<chrono::NaiveDateTime>) = sqlx::query_as(
        "SELECT status, occupied_until
         FROM foster_resource_allocation
         WHERE job_id = ?",
    )
    .bind(fixture.job_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(allocation.0, "CONFIRMED");
    assert_eq!(
        allocation.1,
        Some((completed_at + chrono::Duration::seconds(1_800)).naive_utc())
    );

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn provider_not_found_releases_slot_and_marks_binding_suspect(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "PLATFORM").await?;
    let base = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let (provider_id, cycle_id) =
        seed_platform_resource(&pool, fixture.account_id, base, "资源A02").await?;

    let service = FosterDispatchService::new(pool.clone());
    let (registry, mut receiver) = online_registry(fixture.host_id);
    let delivery = tokio::spawn(async move {
        let outbound = receiver.recv().await.expect("command");
        let _ = outbound.delivered.send(Ok(()));
    });

    assert_eq!(
        service
            .dispatch_job_at(fixture.job_id, &registry, base)
            .await?,
        DispatchFosterResult::Dispatched
    );
    delivery.await?;

    service
        .process_agent_event(
            fixture.host_id,
            &AgentEvent::FosterFailed(FosterFailed {
                job_id: fixture.job_id,
                attempt: 0,
                failed_at: base + chrono::Duration::minutes(1),
                error_code: FosterErrorCode::ProviderNotFound,
                message: "provider alias not found".into(),
                screenshot_url: None,
            }),
        )
        .await?;

    let allocation_status: String =
        sqlx::query_scalar("SELECT status FROM foster_resource_allocation WHERE job_id = ?")
            .bind(fixture.job_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(allocation_status, "RELEASED");

    let occupied: i32 =
        sqlx::query_scalar("SELECT occupied_slots FROM foster_resource_cycle WHERE id = ?")
            .bind(cycle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(occupied, 0);

    let binding: (String, Option<String>) = sqlx::query_as(
        "SELECT status, last_failure_code
         FROM foster_friend_binding
         WHERE game_account_id = ? AND provider_account_id = ?",
    )
    .bind(fixture.account_id)
    .bind(provider_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(binding.0, "SUSPECT");
    assert_eq!(binding.1.as_deref(), Some("PROVIDER_NOT_FOUND"));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn no_slot_quarantines_cycle_and_next_attempt_uses_other_provider(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "PLATFORM").await?;
    let base = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let (_provider_a, cycle_a) =
        seed_platform_resource(&pool, fixture.account_id, base, "资源A03").await?;
    let (_provider_b, cycle_b) =
        seed_platform_resource(&pool, fixture.account_id, base, "资源B03").await?;

    let service = FosterDispatchService::new(pool.clone());
    let (registry, mut receiver) = online_registry(fixture.host_id);
    let first_delivery = tokio::spawn(async move {
        let outbound = receiver.recv().await.expect("first command");
        let command = outbound.envelope.payload.clone();
        let _ = outbound.delivered.send(Ok(()));
        command
    });

    assert_eq!(
        service
            .dispatch_job_at(fixture.job_id, &registry, base)
            .await?,
        DispatchFosterResult::Dispatched
    );
    let first = first_delivery.await?;
    let ServerCommand::ExecuteFoster(first) = first else {
        panic!("expected first ExecuteFoster");
    };
    assert_eq!(first.provider_alias.as_deref(), Some("资源A03"));

    service
        .process_agent_event(
            fixture.host_id,
            &AgentEvent::FosterFailed(FosterFailed {
                job_id: fixture.job_id,
                attempt: 0,
                failed_at: base + chrono::Duration::minutes(1),
                error_code: FosterErrorCode::NoSlot,
                message: "friend realm has no available foster slot".into(),
                screenshot_url: None,
            }),
        )
        .await?;

    let cycle_a_state: (i32, String) = sqlx::query_as(
        "SELECT occupied_slots, status
         FROM foster_resource_cycle
         WHERE id = ?",
    )
    .bind(cycle_a)
    .fetch_one(&pool)
    .await?;
    assert_eq!(cycle_a_state.0, 0);
    assert_eq!(cycle_a_state.1, "FULL");

    sqlx::query(
        "UPDATE foster_job
         SET status = 'SWITCHING_ACCOUNT',
             retry_after = NULL
         WHERE id = ?",
    )
    .bind(fixture.job_id)
    .execute(&pool)
    .await?;

    let (registry, mut receiver) = online_registry(fixture.host_id);
    let second_delivery = tokio::spawn(async move {
        let outbound = receiver.recv().await.expect("second command");
        let command = outbound.envelope.payload.clone();
        let _ = outbound.delivered.send(Ok(()));
        command
    });

    assert_eq!(
        service
            .dispatch_job_at(fixture.job_id, &registry, base)
            .await?,
        DispatchFosterResult::Dispatched
    );

    let second = second_delivery.await?;
    let ServerCommand::ExecuteFoster(second) = second else {
        panic!("expected second ExecuteFoster");
    };
    assert_eq!(second.attempt, 1);
    assert_eq!(second.provider_alias.as_deref(), Some("资源B03"));

    let occupied_b: i32 =
        sqlx::query_scalar("SELECT occupied_slots FROM foster_resource_cycle WHERE id = ?")
            .bind(cycle_b)
            .fetch_one(&pool)
            .await?;
    assert_eq!(occupied_b, 1);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn insufficient_identity_does_not_reserve_platform_slot(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "PLATFORM").await?;
    let base = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let (_provider_id, cycle_id) =
        seed_platform_resource(&pool, fixture.account_id, base, "资源SAFE01").await?;

    sqlx::query(
        "DELETE FROM game_account_identity
         WHERE game_account_id = ?",
    )
    .bind(fixture.account_id)
    .execute(&pool)
    .await?;

    sqlx::query(
        "UPDATE game_account
         SET character_name = NULL,
             server_name = NULL,
             game_uid = NULL
         WHERE id = ?",
    )
    .bind(fixture.account_id)
    .execute(&pool)
    .await?;

    let service = FosterDispatchService::new(pool.clone());
    let result = service
        .dispatch_job_at(fixture.job_id, &AgentRegistry::default(), base)
        .await?;

    assert_eq!(result, DispatchFosterResult::RejectedIdentity);

    let occupied_slots: i32 =
        sqlx::query_scalar("SELECT occupied_slots FROM foster_resource_cycle WHERE id = ?")
            .bind(cycle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(occupied_slots, 0);

    let allocation_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM foster_resource_allocation
         WHERE job_id = ?",
    )
    .bind(fixture.job_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(allocation_count, 0);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn uncertain_delivery_requires_recovery_instead_of_retry(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "USER_FRIEND").await?;
    let service = FosterDispatchService::new(pool.clone());
    let (registry, mut receiver) = online_registry(fixture.host_id);

    let uncertain = tokio::spawn(async move {
        let outbound = receiver.recv().await.expect("foster command");
        assert!(matches!(
            outbound.envelope.payload,
            ServerCommand::ExecuteFoster(_)
        ));
        drop(outbound.delivered);
    });

    let result = service.dispatch_job(fixture.job_id, &registry).await?;
    uncertain.await?;
    assert_eq!(result, DispatchFosterResult::RecoveryRequired);

    let job: (String, Option<String>) =
        sqlx::query_as("SELECT status, error_code FROM foster_job WHERE id = ?")
            .bind(fixture.job_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(job.0, "RECOVERY_REQUIRED");
    assert_eq!(job.1.as_deref(), Some("AGENT_DELIVERY_UNCERTAIN"));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn uncertain_platform_delivery_does_not_release_reserved_slot(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "PLATFORM").await?;
    let base = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let (_provider_id, cycle_id) =
        seed_platform_resource(&pool, fixture.account_id, base, "uncertain-provider").await?;
    let service = FosterDispatchService::new(pool.clone());
    let (registry, mut receiver) = online_registry(fixture.host_id);

    let uncertain = tokio::spawn(async move {
        let outbound = receiver.recv().await.expect("foster command");
        drop(outbound.delivered);
    });
    let result = service
        .dispatch_job_at(fixture.job_id, &registry, base)
        .await?;
    uncertain.await?;

    assert_eq!(result, DispatchFosterResult::RecoveryRequired);
    let status: String = sqlx::query_scalar("SELECT status FROM foster_job WHERE id = ?")
        .bind(fixture.job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "RECOVERY_REQUIRED");

    let allocation_status: String =
        sqlx::query_scalar("SELECT status FROM foster_resource_allocation WHERE job_id = ?")
            .bind(fixture.job_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(allocation_status, "RESERVED");
    let slots: i32 =
        sqlx::query_scalar("SELECT occupied_slots FROM foster_resource_cycle WHERE id = ?")
            .bind(cycle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(slots, 1);
    Ok(())
}
