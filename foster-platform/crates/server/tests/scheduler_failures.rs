use chrono::{TimeZone, Utc};
use foster_domain::{FosterErrorCode, RetryDecision};
use foster_server::scheduler::SchedulerService;
use sqlx::MySqlPool;

struct Fixture {
    subscription_id: i64,
    job_id: i64,
}

async fn seed_running_job(pool: &MySqlPool) -> anyhow::Result<Fixture> {
    let host_id = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('host-failure', 'host-failure', 'ONLINE')",
    )
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let emulator_id = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status, lifecycle_status
         )
         VALUES (?, 'emu-failure', 'FAKE', 5, 'IDLE', 'READY')",
    )
    .bind(host_id)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let account_id = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status, active_emulator_id
         )
         VALUES (80001, 'LOGGED_IN', 'VERIFIED', ?)",
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

    let plan_id = sqlx::query(
        "INSERT INTO foster_plan(
            plan_code, plan_name, daily_target_runs,
            interval_minutes, resource_mode, status
         )
         VALUES ('PLAN-FAILURE', 'Plan Failure', 4, 360,
                 'USER_FRIEND', 'ACTIVE')",
    )
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();

    let subscription_id = sqlx::query(
        "INSERT INTO foster_subscription(
            subscription_no, game_account_id, plan_id,
            resource_mode, daily_target_runs, interval_minutes,
            status, start_at, end_at, next_run_at
         )
         VALUES ('SUB-FAILURE', ?, ?, 'USER_FRIEND', 4, 360,
                 'ACTIVE', ?, ?, NULL)",
    )
    .bind(account_id)
    .bind(plan_id)
    .bind((now - chrono::Duration::days(1)).naive_utc())
    .bind((now + chrono::Duration::days(30)).naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let job_id = sqlx::query(
        "INSERT INTO foster_job(
            job_no, subscription_id, game_account_id,
            emulator_id, status, scheduled_at, started_at
         )
         VALUES ('JOB-FAILURE', ?, ?, ?, 'RUNNING', ?, ?)",
    )
    .bind(subscription_id)
    .bind(account_id)
    .bind(emulator_id)
    .bind((now - chrono::Duration::minutes(5)).naive_utc())
    .bind((now - chrono::Duration::minutes(2)).naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    Ok(Fixture {
        subscription_id,
        job_id,
    })
}

async fn job_state(
    pool: &MySqlPool,
    job_id: i64,
) -> anyhow::Result<(String, i32, Option<chrono::NaiveDateTime>, Option<String>)> {
    Ok(sqlx::query_as(
        "SELECT status, retry_count, retry_after, error_code
         FROM foster_job
         WHERE id = ?",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrations = "../../migrations")]
async fn network_error_retries_in_five_minutes(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_running_job(&pool).await?;
    let scheduler = SchedulerService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();

    let decision = scheduler
        .handle_failure(
            fixture.job_id,
            FosterErrorCode::NetworkError,
            "temporary network issue",
            now,
        )
        .await?;

    assert_eq!(
        decision,
        RetryDecision::RetryAt(now + chrono::Duration::minutes(5))
    );

    let state = job_state(&pool, fixture.job_id).await?;
    assert_eq!(state.0, "RETRY");
    assert_eq!(state.1, 1);
    assert_eq!(
        state.2,
        Some((now + chrono::Duration::minutes(5)).naive_utc())
    );
    assert_eq!(state.3.as_deref(), Some("NETWORK_ERROR"));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn game_busy_retries_in_five_minutes(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_running_job(&pool).await?;
    let scheduler = SchedulerService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();

    let decision = scheduler
        .handle_failure(fixture.job_id, FosterErrorCode::GameBusy, "busy", now)
        .await?;

    assert_eq!(
        decision,
        RetryDecision::RetryAt(now + chrono::Duration::minutes(5))
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn no_slot_uses_short_retry(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_running_job(&pool).await?;
    let scheduler = SchedulerService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();

    let decision = scheduler
        .handle_failure(fixture.job_id, FosterErrorCode::NoSlot, "no slot", now)
        .await?;

    assert_eq!(
        decision,
        RetryDecision::RetryAt(now + chrono::Duration::minutes(1))
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn provider_not_found_uses_short_retry(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_running_job(&pool).await?;
    let scheduler = SchedulerService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();

    let decision = scheduler
        .handle_failure(
            fixture.job_id,
            FosterErrorCode::ProviderNotFound,
            "provider missing",
            now,
        )
        .await?;

    assert_eq!(
        decision,
        RetryDecision::RetryAt(now + chrono::Duration::minutes(1))
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn emulator_offline_waits_without_terminal_failure(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_running_job(&pool).await?;
    let scheduler = SchedulerService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();

    let decision = scheduler
        .handle_failure(
            fixture.job_id,
            FosterErrorCode::EmulatorOffline,
            "offline",
            now,
        )
        .await?;

    assert_eq!(decision, RetryDecision::WaitForEmulator);

    let state = job_state(&pool, fixture.job_id).await?;
    assert_eq!(state.0, "WAITING_EMULATOR");
    assert!(state.2.is_none());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_expired_suspends_subscription(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_running_job(&pool).await?;
    let scheduler = SchedulerService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();

    let decision = scheduler
        .handle_failure(
            fixture.job_id,
            FosterErrorCode::AccountLoginExpired,
            "login expired",
            now,
        )
        .await?;

    assert_eq!(decision, RetryDecision::SuspendAccount);

    let job: (String, Option<chrono::NaiveDateTime>) =
        sqlx::query_as("SELECT status, finished_at FROM foster_job WHERE id = ?")
            .bind(fixture.job_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(job.0, "FAILED");
    assert!(job.1.is_some());

    let subscription: (String, Option<chrono::NaiveDateTime>) = sqlx::query_as(
        "SELECT status, next_run_at
         FROM foster_subscription
         WHERE id = ?",
    )
    .bind(fixture.subscription_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(subscription.0, "SUSPENDED");
    assert!(subscription.1.is_none());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn identity_mismatch_suspends_subscription(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_running_job(&pool).await?;
    let scheduler = SchedulerService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();

    let decision = scheduler
        .handle_failure(
            fixture.job_id,
            FosterErrorCode::IdentityMismatch,
            "identity mismatch",
            now,
        )
        .await?;

    assert_eq!(decision, RetryDecision::SuspendAccount);

    let status: String = sqlx::query_scalar("SELECT status FROM foster_job WHERE id = ?")
        .bind(fixture.job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "IDENTITY_MISMATCH");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn unknown_error_retries_then_fails_and_restores_schedule(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_running_job(&pool).await?;
    let scheduler = SchedulerService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();

    for attempt in 0..2 {
        let at = now + chrono::Duration::minutes(i64::from(attempt) * 6);
        let decision = scheduler
            .handle_failure(fixture.job_id, FosterErrorCode::Unknown, "unknown", at)
            .await?;

        assert_eq!(
            decision,
            RetryDecision::RetryAt(at + chrono::Duration::minutes(5))
        );

        sqlx::query(
            "UPDATE foster_job
             SET status = 'RUNNING',
                 retry_after = NULL
             WHERE id = ?",
        )
        .bind(fixture.job_id)
        .execute(&pool)
        .await?;
    }

    let final_at = now + chrono::Duration::minutes(12);
    let decision = scheduler
        .handle_failure(
            fixture.job_id,
            FosterErrorCode::Unknown,
            "unknown final",
            final_at,
        )
        .await?;

    assert_eq!(decision, RetryDecision::FailTerminal);

    let state = job_state(&pool, fixture.job_id).await?;
    assert_eq!(state.0, "FAILED");
    assert_eq!(state.1, 3);

    let next_run_at: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
        "SELECT next_run_at
         FROM foster_subscription
         WHERE id = ?",
    )
    .bind(fixture.subscription_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(
        next_run_at,
        Some((final_at + chrono::Duration::minutes(360)).naive_utc())
    );
    Ok(())
}
