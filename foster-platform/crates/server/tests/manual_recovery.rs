use std::time::Duration;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::AUTHORIZATION},
};
use chrono::{TimeZone, Utc};
use foster_server::{
    agent_gateway::registry::AgentRegistry,
    app::{AppState, build_app_with_admin_token},
    config::AgentGatewayConfig,
    recovery::{RecoveryAction, RecoveryError, RecoveryService, ResolveRecoveryRequest},
};
use sqlx::MySqlPool;
use tower::ServiceExt;

struct Fixture {
    job_id: i64,
    subscription_id: i64,
    cycle_id: Option<i64>,
    allocation_id: Option<i64>,
}

fn at() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap()
}

async fn seed_recovery(pool: &MySqlPool, platform: bool) -> anyhow::Result<Fixture> {
    let host = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('recovery-host', 'recovery-host', 'ONLINE')",
    )
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let emulator = sqlx::query(
        "INSERT INTO emulator_instance(host_id, emulator_code, driver_type, status)
         VALUES (?, 'recovery-emu', 'FAKE', 'IDLE')",
    )
    .bind(host)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let account = sqlx::query(
        "INSERT INTO game_account(customer_id, login_status, verify_status, active_emulator_id)
         VALUES (12345, 'LOGGED_IN', 'VERIFIED', ?)",
    )
    .bind(emulator)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let mode = if platform { "PLATFORM" } else { "USER_FRIEND" };
    let resource = platform.then_some("FISH");
    let plan = sqlx::query(
        "INSERT INTO foster_plan(
            plan_code, plan_name, daily_target_runs, interval_minutes,
            resource_mode, resource_type, status
         ) VALUES ('PLAN-RECOVERY', 'Recovery Plan', 4, 360, ?, ?, 'ACTIVE')",
    )
    .bind(mode)
    .bind(resource)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let subscription_id = sqlx::query(
        "INSERT INTO foster_subscription(
            subscription_no, game_account_id, plan_id, resource_mode,
            resource_type, daily_target_runs, interval_minutes, status, start_at, end_at
         ) VALUES ('SUB-RECOVERY', ?, ?, ?, ?, 4, 360, 'ACTIVE', ?, ?)",
    )
    .bind(account)
    .bind(plan)
    .bind(mode)
    .bind(resource)
    .bind((at() - chrono::Duration::days(1)).naive_utc())
    .bind((at() + chrono::Duration::days(30)).naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let job_id = sqlx::query(
        "INSERT INTO foster_job(
            job_no, subscription_id, game_account_id, emulator_id,
            status, retry_count, scheduled_at, started_at,
            error_code, result_message
         ) VALUES ('JOB-RECOVERY', ?, ?, ?, 'RECOVERY_REQUIRED', 2, ?, ?,
                   'AGENT_RECOVERY_REQUIRED', 'interrupted')",
    )
    .bind(subscription_id)
    .bind(account)
    .bind(emulator)
    .bind((at() - chrono::Duration::hours(1)).naive_utc())
    .bind((at() - chrono::Duration::minutes(10)).naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    if !platform {
        return Ok(Fixture {
            job_id,
            subscription_id,
            cycle_id: None,
            allocation_id: None,
        });
    }

    let provider = sqlx::query(
        "INSERT INTO provider_account(
            provider_code, nickname, provider_alias, status
         ) VALUES ('PROVIDER-RECOVERY', '资源号', '资源号A', 'ACTIVE')",
    )
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let cycle_id = sqlx::query(
        "INSERT INTO foster_resource_cycle(
            provider_account_id, resource_type, resource_level,
            start_at, end_at, slot_capacity, occupied_slots, status
         ) VALUES (?, 'FISH', 6, ?, ?, 1, 1, 'FULL')",
    )
    .bind(provider)
    .bind((at() - chrono::Duration::hours(1)).naive_utc())
    .bind((at() + chrono::Duration::hours(8)).naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let allocation_id = sqlx::query(
        "INSERT INTO foster_resource_allocation(
            job_id, resource_cycle_id, provider_account_id,
            status, reserved_at
         ) VALUES (?, ?, ?, 'RESERVED', ?)",
    )
    .bind(job_id)
    .bind(cycle_id)
    .bind(provider)
    .bind((at() - chrono::Duration::minutes(10)).naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    sqlx::query(
        "UPDATE foster_job
         SET provider_account_id = ?, resource_cycle_id = ?, resource_allocation_id = ?
         WHERE id = ?",
    )
    .bind(provider)
    .bind(cycle_id)
    .bind(allocation_id)
    .bind(job_id)
    .execute(pool)
    .await?;

    Ok(Fixture {
        job_id,
        subscription_id,
        cycle_id: Some(cycle_id),
        allocation_id: Some(allocation_id),
    })
}

fn request(action: RecoveryAction, remaining: Option<i32>) -> ResolveRecoveryRequest {
    ResolveRecoveryRequest {
        expected_attempt: 2,
        action,
        operator: "ops-01".into(),
        note: "verified against live game state".into(),
        observed_remaining_seconds: remaining,
        confirmed_stopped: true,
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn platform_success_keeps_slot_and_schedules_from_observation(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_recovery(&pool, true).await?;
    let result = RecoveryService::new(pool.clone())
        .resolve(
            fixture.job_id,
            request(RecoveryAction::ConfirmSucceeded, Some(1_800)),
            at(),
        )
        .await?;

    assert_eq!(result.status, "SUCCESS");
    assert_eq!(
        result.next_run_at,
        Some(at() + chrono::Duration::seconds(1_800))
    );
    assert_eq!(result.allocation_status.as_deref(), Some("CONFIRMED"));

    let job: (String, Option<i32>) =
        sqlx::query_as("SELECT status, remaining_seconds FROM foster_job WHERE id = ?")
            .bind(fixture.job_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(job, ("SUCCESS".into(), Some(1_800)));

    let allocation: (String, Option<chrono::NaiveDateTime>) = sqlx::query_as(
        "SELECT status, occupied_until FROM foster_resource_allocation WHERE id = ?",
    )
    .bind(fixture.allocation_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(allocation.0, "CONFIRMED");
    assert_eq!(
        allocation.1,
        Some((at() + chrono::Duration::seconds(1_800)).naive_utc())
    );

    let occupied: i32 =
        sqlx::query_scalar("SELECT occupied_slots FROM foster_resource_cycle WHERE id = ?")
            .bind(fixture.cycle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(occupied, 1);

    let next: Option<chrono::NaiveDateTime> =
        sqlx::query_scalar("SELECT next_run_at FROM foster_subscription WHERE id = ?")
            .bind(fixture.subscription_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        next,
        Some((at() + chrono::Duration::seconds(1_800)).naive_utc())
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn verified_no_execution_releases_exactly_once_and_advances_attempt(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_recovery(&pool, true).await?;
    let service = RecoveryService::new(pool.clone());
    let original = request(RecoveryAction::ConfirmNotExecuted, None);

    let first = service
        .resolve(fixture.job_id, original.clone(), at())
        .await?;
    assert_eq!(first.status, "RETRY");
    assert_eq!(first.retry_after, Some(at() + chrono::Duration::minutes(5)));

    let row: (String, i32, Option<i64>) = sqlx::query_as(
        "SELECT status, retry_count, resource_allocation_id FROM foster_job WHERE id = ?",
    )
    .bind(fixture.job_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(row, ("RETRY".into(), 3, None));

    let status: String =
        sqlx::query_scalar("SELECT status FROM foster_resource_allocation WHERE id = ?")
            .bind(fixture.allocation_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(status, "RELEASED");

    let occupied: i32 =
        sqlx::query_scalar("SELECT occupied_slots FROM foster_resource_cycle WHERE id = ?")
            .bind(fixture.cycle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(occupied, 0);

    assert!(matches!(
        service.resolve(fixture.job_id, original, at()).await,
        Err(RecoveryError::StateConflict)
    ));

    let audit_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM foster_recovery_audit WHERE job_id = ?")
            .bind(fixture.job_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(audit_count, 1);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn wrong_attempt_does_not_release_reservation(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_recovery(&pool, true).await?;
    let mut resolution = request(RecoveryAction::ConfirmNotExecuted, None);
    resolution.expected_attempt = 1;

    assert!(matches!(
        RecoveryService::new(pool.clone())
            .resolve(fixture.job_id, resolution, at())
            .await,
        Err(RecoveryError::AttemptMismatch)
    ));

    let status: String =
        sqlx::query_scalar("SELECT status FROM foster_resource_allocation WHERE id = ?")
            .bind(fixture.allocation_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(status, "RESERVED");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn uncertain_platform_success_without_remaining_duration_is_rejected(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_recovery(&pool, true).await?;
    let result = RecoveryService::new(pool.clone())
        .resolve(
            fixture.job_id,
            request(RecoveryAction::ConfirmSucceeded, None),
            at(),
        )
        .await;

    assert!(matches!(result, Err(RecoveryError::RemainingRequired)));
    let status: String =
        sqlx::query_scalar("SELECT status FROM foster_resource_allocation WHERE id = ?")
            .bind(fixture.allocation_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(status, "RESERVED");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn already_confirmed_resource_cannot_be_released_as_no_execution(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_recovery(&pool, true).await?;
    sqlx::query("UPDATE foster_resource_allocation SET status = 'CONFIRMED' WHERE id = ?")
        .bind(fixture.allocation_id)
        .execute(&pool)
        .await?;

    assert!(matches!(
        RecoveryService::new(pool.clone())
            .resolve(
                fixture.job_id,
                request(RecoveryAction::ConfirmNotExecuted, None),
                at(),
            )
            .await,
        Err(RecoveryError::AllocationUnavailable)
    ));

    let occupied: i32 =
        sqlx::query_scalar("SELECT occupied_slots FROM foster_resource_cycle WHERE id = ?")
            .bind(fixture.cycle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(occupied, 1);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn basic_success_uses_subscription_interval(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_recovery(&pool, false).await?;
    let result = RecoveryService::new(pool)
        .resolve(
            fixture.job_id,
            request(RecoveryAction::ConfirmSucceeded, None),
            at(),
        )
        .await?;
    assert_eq!(
        result.next_run_at,
        Some(at() + chrono::Duration::minutes(360))
    );
    assert!(result.allocation_status.is_none());
    Ok(())
}

fn app_state(pool: MySqlPool) -> AppState {
    AppState {
        pool,
        registry: AgentRegistry::default(),
        gateway_config: AgentGatewayConfig {
            agent_token: "agent-test".into(),
            heartbeat_timeout: Duration::from_secs(45),
            sweep_interval: Duration::from_secs(5),
        },
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn recovery_endpoints_require_admin_token_and_list_pending_jobs(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_recovery(&pool, true).await?;
    let app = build_app_with_admin_token(app_state(pool), Some("admin-secret".into()));

    let denied = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/recovery-jobs")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

    let denied_post = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/recovery-jobs/{}/resolve", fixture.job_id))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "expectedAttempt": 2,
                        "action": "CONFIRM_NOT_EXECUTED",
                        "operator": "ops-01",
                        "note": "verified not executed",
                        "confirmedStopped": true
                    })
                    .to_string(),
                ))?,
        )
        .await?;
    assert_eq!(denied_post.status(), StatusCode::UNAUTHORIZED);

    let listed = app
        .oneshot(
            Request::builder()
                .uri("/admin/recovery-jobs")
                .header(AUTHORIZATION, "Bearer admin-secret")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(listed.status(), StatusCode::OK);
    let body = to_bytes(listed.into_body(), 1024 * 1024).await?;
    let jobs: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(jobs.as_array().map(|values| values.len()), Some(1));
    assert_eq!(jobs[0]["jobId"].as_i64(), Some(fixture.job_id));
    assert_eq!(jobs[0]["allocationStatus"].as_str(), Some("RESERVED"));

    let resolved = build_app_with_admin_token(
        app_state(pool.clone()),
        Some("admin-secret".into()),
    )
    .oneshot(
        Request::builder()
            .method("POST")
            .uri(format!("/admin/recovery-jobs/{}/resolve", fixture.job_id))
            .header("content-type", "application/json")
            .header(AUTHORIZATION, "Bearer admin-secret")
            .body(Body::from(
                serde_json::json!({
                    "expectedAttempt": 2,
                    "action": "CONFIRM_NOT_EXECUTED",
                    "operator": "ops-01",
                    "note": "verified old OAS process stopped",
                    "confirmedStopped": true
                })
                .to_string(),
            ))?,
    )
    .await?;
    assert_eq!(resolved.status(), StatusCode::OK);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn operator_must_confirm_old_executor_stopped(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_recovery(&pool, true).await?;
    let mut resolution = request(RecoveryAction::ConfirmNotExecuted, None);
    resolution.confirmed_stopped = false;

    assert!(matches!(
        RecoveryService::new(pool.clone())
            .resolve(fixture.job_id, resolution, at())
            .await,
        Err(RecoveryError::MustConfirmStopped)
    ));

    let status: String =
        sqlx::query_scalar("SELECT status FROM foster_resource_allocation WHERE id = ?")
            .bind(fixture.allocation_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(status, "RESERVED");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn expired_platform_reservation_is_not_released_twice_during_manual_retry(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_recovery(&pool, true).await?;
    let later = at() + chrono::Duration::hours(9);
    let reap = foster_server::resource_pool::ResourcePoolService::new(pool.clone())
        .reap(later)
        .await?;
    assert_eq!(reap.expired_allocations, 1);

    let result = RecoveryService::new(pool.clone())
        .resolve(
            fixture.job_id,
            request(RecoveryAction::ConfirmNotExecuted, None),
            later,
        )
        .await?;

    assert_eq!(result.status, "RETRY");
    assert_eq!(result.allocation_status.as_deref(), Some("EXPIRED"));

    let occupied: i32 =
        sqlx::query_scalar("SELECT occupied_slots FROM foster_resource_cycle WHERE id = ?")
            .bind(fixture.cycle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(occupied, 0);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_operator_resolutions_only_apply_once(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_recovery(&pool, true).await?;
    let first = RecoveryService::new(pool.clone());
    let second = RecoveryService::new(pool.clone());
    let resolution = request(RecoveryAction::ConfirmNotExecuted, None);

    let (left, right) = tokio::join!(
        first.resolve(fixture.job_id, resolution.clone(), at()),
        second.resolve(fixture.job_id, resolution, at()),
    );

    assert_eq!(
        [left.is_ok(), right.is_ok()]
            .iter()
            .filter(|value| **value)
            .count(),
        1
    );

    let occupied: i32 =
        sqlx::query_scalar("SELECT occupied_slots FROM foster_resource_cycle WHERE id = ?")
            .bind(fixture.cycle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(occupied, 0);

    let audit_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM foster_recovery_audit WHERE job_id = ?")
            .bind(fixture.job_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(audit_count, 1);
    Ok(())
}
