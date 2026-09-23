use std::time::Duration;

use chrono::Utc;
use foster_protocol::{
    AgentCommandKind, AgentCommandState, AgentCommandStatus,
};
use foster_server::{
    agent_reconciliation::AgentReconciliationService,
    enrollment::{EnrollmentService, login_command_id},
    foster_dispatch::foster_command_id,
};
use sqlx::MySqlPool;
use uuid::Uuid;

struct Fixture {
    host_id: i64,
    job_id: i64,
    attempt: i32,
}

async fn seed_job(
    pool: &MySqlPool,
    suffix: &str,
    status: &str,
    attempt: i32,
) -> anyhow::Result<Fixture> {
    let host_id = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES (?, ?, 'ONLINE')",
    )
    .bind(format!("host-reconcile-{suffix}"))
    .bind(format!("host-reconcile-{suffix}"))
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let emulator_id = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status
         )
         VALUES (?, ?, 'FAKE', 5, 'IDLE')",
    )
    .bind(host_id)
    .bind(format!("emu-reconcile-{suffix}"))
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let account_id = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status, active_emulator_id
         )
         VALUES (?, 'LOGGED_IN', 'VERIFIED', ?)",
    )
    .bind(100_000 + host_id)
    .bind(emulator_id)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let plan_id = sqlx::query(
        "INSERT INTO foster_plan(
            plan_code, plan_name, daily_target_runs,
            interval_minutes, resource_mode, status
         )
         VALUES (?, ?, 4, 360, 'USER_FRIEND', 'ACTIVE')",
    )
    .bind(format!("PLAN-RECONCILE-{suffix}"))
    .bind(format!("Plan Reconcile {suffix}"))
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let subscription_id = sqlx::query(
        "INSERT INTO foster_subscription(
            subscription_no, game_account_id, plan_id,
            resource_mode, daily_target_runs, interval_minutes,
            status, start_at, end_at, next_run_at
         )
         VALUES (?, ?, ?, 'USER_FRIEND', 4, 360,
                 'ACTIVE', NOW(3), DATE_ADD(NOW(3), INTERVAL 30 DAY), NULL)",
    )
    .bind(format!("SUB-RECONCILE-{suffix}"))
    .bind(account_id)
    .bind(plan_id)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let job_id = sqlx::query(
        "INSERT INTO foster_job(
            job_no, subscription_id, game_account_id,
            emulator_id, status, retry_count, scheduled_at, started_at
         )
         VALUES (?, ?, ?, ?, ?, ?, NOW(3), NOW(3))",
    )
    .bind(format!("JOB-RECONCILE-{suffix}"))
    .bind(subscription_id)
    .bind(account_id)
    .bind(emulator_id)
    .bind(status)
    .bind(attempt)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    Ok(Fixture {
        host_id,
        job_id,
        attempt,
    })
}

struct LoginFixture {
    host_id: i64,
    session_no: String,
    binding_id: i64,
}

async fn seed_login(
    pool: &MySqlPool,
    suffix: &str,
    status: &str,
) -> anyhow::Result<LoginFixture> {
    let host_id = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES (?, ?, 'ONLINE')",
    )
    .bind(format!("host-login-reconcile-{suffix}"))
    .bind(format!("host-login-reconcile-{suffix}"))
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status
         )
         VALUES (?, ?, 'FAKE', 5, 'IDLE')",
    )
    .bind(host_id)
    .bind(format!("emu-login-reconcile-{suffix}"))
    .execute(pool)
    .await?;

    let account_id = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status
         )
         VALUES (?, 'PENDING', 'PENDING')",
    )
    .bind(200_000 + host_id)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let created = EnrollmentService::new(pool.clone())
        .create_login_session(account_id, Duration::from_secs(900))
        .await?;

    sqlx::query(
        "UPDATE login_session
         SET status = ?
         WHERE session_no = ?",
    )
    .bind(status)
    .bind(&created.session_no)
    .execute(pool)
    .await?;

    Ok(LoginFixture {
        host_id,
        session_no: created.session_no,
        binding_id: created.binding_id,
    })
}

fn login_command_state(
    fixture: &LoginFixture,
    status: AgentCommandStatus,
) -> AgentCommandState {
    AgentCommandState {
        command_id: login_command_id(&fixture.session_no),
        kind: AgentCommandKind::Login,
        status,
        job_id: None,
        attempt: None,
        session_no: Some(fixture.session_no.clone()),
        updated_at: Utc::now(),
    }
}

async fn login_status(pool: &MySqlPool, session_no: &str) -> anyhow::Result<String> {
    Ok(sqlx::query_scalar(
        "SELECT status
         FROM login_session
         WHERE session_no = ?",
    )
    .bind(session_no)
    .fetch_one(pool)
    .await?)
}

async fn binding_status(pool: &MySqlPool, binding_id: i64) -> anyhow::Result<String> {
    Ok(sqlx::query_scalar(
        "SELECT status
         FROM emulator_account_binding
         WHERE id = ?",
    )
    .bind(binding_id)
    .fetch_one(pool)
    .await?)
}

fn command_state(
    fixture: &Fixture,
    status: AgentCommandStatus,
) -> AgentCommandState {
    AgentCommandState {
        command_id: foster_command_id(fixture.job_id, fixture.attempt),
        kind: AgentCommandKind::Foster,
        status,
        job_id: Some(fixture.job_id),
        attempt: Some(fixture.attempt),
        session_no: None,
        updated_at: Utc::now(),
    }
}

async fn job_status(pool: &MySqlPool, job_id: i64) -> anyhow::Result<(String, Option<String>)> {
    Ok(sqlx::query_as(
        "SELECT status, error_code
         FROM foster_job
         WHERE id = ?",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await?)
}

#[test]
fn foster_command_id_is_stable_per_attempt() {
    assert_eq!(foster_command_id(7, 2), foster_command_id(7, 2));
    assert_ne!(foster_command_id(7, 2), foster_command_id(7, 3));
    assert_ne!(foster_command_id(7, 2), foster_command_id(8, 2));
}

#[sqlx::test(migrations = "../../migrations")]
async fn matching_running_command_preserves_server_job(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_job(&pool, "running", "RUNNING", 2).await?;
    let service = AgentReconciliationService::new(pool.clone());

    let report = service
        .reconcile(
            fixture.host_id,
            &[command_state(&fixture, AgentCommandStatus::Running)],
        )
        .await?;

    assert_eq!(report.preserved, 1);
    assert_eq!(report.recovery_required, 0);
    assert_eq!(job_status(&pool, fixture.job_id).await?.0, "RUNNING");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn finished_command_waits_for_terminal_event_replay(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_job(&pool, "finished", "VERIFYING_ACCOUNT", 1).await?;
    let service = AgentReconciliationService::new(pool.clone());

    let report = service
        .reconcile(
            fixture.host_id,
            &[command_state(&fixture, AgentCommandStatus::Finished)],
        )
        .await?;

    assert_eq!(report.preserved, 1);
    assert_eq!(report.recovery_required, 0);
    assert_eq!(
        job_status(&pool, fixture.job_id).await?.0,
        "VERIFYING_ACCOUNT"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn interrupted_command_moves_job_to_recovery_required(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_job(&pool, "interrupted", "RUNNING", 4).await?;
    let service = AgentReconciliationService::new(pool.clone());

    let report = service
        .reconcile(
            fixture.host_id,
            &[command_state(&fixture, AgentCommandStatus::Interrupted)],
        )
        .await?;

    assert_eq!(report.recovery_required, 1);

    let state = job_status(&pool, fixture.job_id).await?;
    assert_eq!(state.0, "RECOVERY_REQUIRED");
    assert_eq!(state.1.as_deref(), Some("AGENT_RECOVERY_REQUIRED"));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn missing_agent_command_moves_running_job_to_recovery_required(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_job(&pool, "missing", "RUNNING", 0).await?;
    let service = AgentReconciliationService::new(pool.clone());

    let report = service.reconcile(fixture.host_id, &[]).await?;

    assert_eq!(report.recovery_required, 1);
    assert_eq!(
        job_status(&pool, fixture.job_id).await?.0,
        "RECOVERY_REQUIRED"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn untrusted_command_id_does_not_preserve_job(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_job(&pool, "wrong-id", "VERIFYING_ACCOUNT", 5).await?;
    let service = AgentReconciliationService::new(pool.clone());

    let mut state = command_state(&fixture, AgentCommandStatus::Running);
    state.command_id = Uuid::new_v4();

    let report = service.reconcile(fixture.host_id, &[state]).await?;

    assert_eq!(report.recovery_required, 1);
    assert_eq!(
        job_status(&pool, fixture.job_id).await?.0,
        "RECOVERY_REQUIRED"
    );
    Ok(())
}


#[sqlx::test(migrations = "../../migrations")]
async fn interrupted_login_fails_session_and_releases_pending_slot(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_login(&pool, "interrupted", "PREPARING").await?;
    let service = AgentReconciliationService::new(pool.clone());

    let report = service
        .reconcile(
            fixture.host_id,
            &[login_command_state(
                &fixture,
                AgentCommandStatus::Interrupted,
            )],
        )
        .await?;

    assert_eq!(report.login_failed, 1);
    assert_eq!(login_status(&pool, &fixture.session_no).await?, "FAILED");
    assert_eq!(binding_status(&pool, fixture.binding_id).await?, "UNBOUND");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn missing_login_command_fails_active_login_and_releases_slot(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_login(&pool, "missing", "QR_READY").await?;
    let service = AgentReconciliationService::new(pool.clone());

    let report = service.reconcile(fixture.host_id, &[]).await?;

    assert_eq!(report.login_failed, 1);
    assert_eq!(login_status(&pool, &fixture.session_no).await?, "FAILED");
    assert_eq!(binding_status(&pool, fixture.binding_id).await?, "UNBOUND");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn finished_login_command_is_preserved_for_cached_event_replay(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_login(&pool, "finished", "DETECTING_LOGIN").await?;
    let service = AgentReconciliationService::new(pool.clone());

    let report = service
        .reconcile(
            fixture.host_id,
            &[login_command_state(
                &fixture,
                AgentCommandStatus::Finished,
            )],
        )
        .await?;

    assert_eq!(report.login_failed, 0);
    assert_eq!(report.preserved, 1);
    assert_eq!(
        login_status(&pool, &fixture.session_no).await?,
        "DETECTING_LOGIN"
    );
    assert_eq!(binding_status(&pool, fixture.binding_id).await?, "PENDING");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn verifying_account_is_not_failed_when_agent_work_is_already_done(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_login(&pool, "verify", "VERIFYING_ACCOUNT").await?;
    let service = AgentReconciliationService::new(pool.clone());

    let report = service.reconcile(fixture.host_id, &[]).await?;

    assert_eq!(report.login_failed, 0);
    assert_eq!(
        login_status(&pool, &fixture.session_no).await?,
        "VERIFYING_ACCOUNT"
    );
    assert_eq!(binding_status(&pool, fixture.binding_id).await?, "PENDING");
    Ok(())
}


#[sqlx::test(migrations = "../../migrations")]
async fn switching_job_without_agent_state_is_safely_redispatched(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_job(&pool, "redispatch", "SWITCHING_ACCOUNT", 2).await?;
    let service = AgentReconciliationService::new(pool.clone());

    let report = service.reconcile(fixture.host_id, &[]).await?;

    assert_eq!(report.recovery_required, 0);
    assert_eq!(report.redispatch_job_ids, vec![fixture.job_id]);
    assert_eq!(
        job_status(&pool, fixture.job_id).await?.0,
        "SWITCHING_ACCOUNT"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn interrupted_switching_job_is_not_redispatched(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_job(&pool, "redispatch-interrupted", "SWITCHING_ACCOUNT", 3).await?;
    let service = AgentReconciliationService::new(pool.clone());

    let report = service
        .reconcile(
            fixture.host_id,
            &[command_state(&fixture, AgentCommandStatus::Interrupted)],
        )
        .await?;

    assert_eq!(report.recovery_required, 1);
    assert!(report.redispatch_job_ids.is_empty());
    assert_eq!(
        job_status(&pool, fixture.job_id).await?.0,
        "RECOVERY_REQUIRED"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn matching_running_switching_command_is_not_redispatched(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_job(&pool, "redispatch-running", "SWITCHING_ACCOUNT", 4).await?;
    let service = AgentReconciliationService::new(pool.clone());

    let report = service
        .reconcile(
            fixture.host_id,
            &[command_state(&fixture, AgentCommandStatus::Running)],
        )
        .await?;

    assert!(report.redispatch_job_ids.is_empty());
    assert_eq!(
        job_status(&pool, fixture.job_id).await?.0,
        "SWITCHING_ACCOUNT"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn finished_switching_command_is_redispatched_for_cached_result(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_job(&pool, "redispatch-finished", "SWITCHING_ACCOUNT", 5).await?;
    let service = AgentReconciliationService::new(pool.clone());

    let report = service
        .reconcile(
            fixture.host_id,
            &[command_state(&fixture, AgentCommandStatus::Finished)],
        )
        .await?;

    assert_eq!(report.redispatch_job_ids, vec![fixture.job_id]);
    Ok(())
}
