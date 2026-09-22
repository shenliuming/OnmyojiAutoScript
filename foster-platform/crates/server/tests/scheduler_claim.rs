use chrono::{Duration, TimeZone, Utc};
use foster_server::scheduler::{ClaimResult, SchedulerService};
use sqlx::MySqlPool;

struct ClaimFixture {
    emulator_id: i64,
    account_id: i64,
    subscription_id: i64,
    job_id: i64,
}

async fn seed_host(pool: &MySqlPool, suffix: &str, status: &str) -> anyhow::Result<i64> {
    Ok(sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES (?, ?, ?)",
    )
    .bind(format!("host-claim-{suffix}"))
    .bind(format!("host-claim-{suffix}"))
    .bind(status)
    .execute(pool)
    .await?
    .last_insert_id() as i64)
}

async fn seed_emulator(
    pool: &MySqlPool,
    host_id: i64,
    suffix: &str,
    status: &str,
) -> anyhow::Result<i64> {
    Ok(sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status
         )
         VALUES (?, ?, 'FAKE', 5, ?)",
    )
    .bind(host_id)
    .bind(format!("emu-claim-{suffix}"))
    .bind(status)
    .execute(pool)
    .await?
    .last_insert_id() as i64)
}

async fn seed_bound_job(
    pool: &MySqlPool,
    emulator_id: i64,
    suffix: &str,
    slot_no: i32,
    now: chrono::DateTime<Utc>,
    job_status: &str,
    manual_pause_until: Option<chrono::DateTime<Utc>>,
) -> anyhow::Result<ClaimFixture> {
    let account_id = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status, active_emulator_id
         )
         VALUES (?, 'LOGGED_IN', 'VERIFIED', ?)",
    )
    .bind(50_000 + i64::from(slot_no))
    .bind(emulator_id)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    sqlx::query(
        "INSERT INTO emulator_account_binding(
            emulator_id, game_account_id, slot_no, status, bound_at
         )
         VALUES (?, ?, ?, 'ACTIVE', NOW(3))",
    )
    .bind(emulator_id)
    .bind(account_id)
    .bind(slot_no)
    .execute(pool)
    .await?;

    let plan_id = sqlx::query(
        "INSERT INTO foster_plan(
            plan_code, plan_name, daily_target_runs,
            interval_minutes, resource_mode, status
         )
         VALUES (?, ?, 4, 360, 'USER_FRIEND', 'ACTIVE')",
    )
    .bind(format!("PLAN-CLAIM-{suffix}"))
    .bind(format!("Plan Claim {suffix}"))
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let subscription_id = sqlx::query(
        "INSERT INTO foster_subscription(
            subscription_no, game_account_id, plan_id,
            resource_mode, daily_target_runs, interval_minutes,
            status, manual_pause_until, start_at, end_at, next_run_at
         )
         VALUES (?, ?, ?, 'USER_FRIEND', 4, 360,
                 'ACTIVE', ?, ?, ?, NULL)",
    )
    .bind(format!("SUB-CLAIM-{suffix}"))
    .bind(account_id)
    .bind(plan_id)
    .bind(manual_pause_until.map(|value| value.naive_utc()))
    .bind((now - Duration::days(1)).naive_utc())
    .bind((now + Duration::days(30)).naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let job_id = sqlx::query(
        "INSERT INTO foster_job(
            job_no, subscription_id, game_account_id,
            status, scheduled_at
         )
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(format!("JOB-CLAIM-{suffix}"))
    .bind(subscription_id)
    .bind(account_id)
    .bind(job_status)
    .bind(now.naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    Ok(ClaimFixture {
        emulator_id,
        account_id,
        subscription_id,
        job_id,
    })
}

async fn add_quiet_period(pool: &MySqlPool, account_id: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO foster_quiet_period(
            game_account_id, weekday_mask,
            start_time, end_time, timezone,
            before_buffer_minutes, after_buffer_minutes, enabled
         )
         VALUES (?, 1, '20:00:00', '23:00:00',
                 'Asia/Shanghai', 0, 0, 1)",
    )
    .bind(account_id)
    .execute(pool)
    .await?;

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn two_accounts_on_same_emulator_only_one_claims(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 10, 0, 0).unwrap();
    let host_id = seed_host(&pool, "race", "ONLINE").await?;
    let emulator_id = seed_emulator(&pool, host_id, "race", "IDLE").await?;
    let first = seed_bound_job(&pool, emulator_id, "race-a", 1, now, "PENDING", None).await?;
    let second = seed_bound_job(&pool, emulator_id, "race-b", 2, now, "PENDING", None).await?;

    let left = SchedulerService::new(pool.clone());
    let right = SchedulerService::new(pool.clone());

    let (a, b) = tokio::join!(
        left.claim_for_execution(first.job_id, now),
        right.claim_for_execution(second.job_id, now)
    );

    let results = [a?, b?];
    let claimed = results
        .iter()
        .filter(|result| matches!(result, ClaimResult::Claimed { .. }))
        .count();
    let waiting = results
        .iter()
        .filter(|result| matches!(result, ClaimResult::WaitingEmulator))
        .count();

    assert_eq!(claimed, 1);
    assert_eq!(waiting, 1);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn offline_emulator_moves_job_to_waiting(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 10, 0, 0).unwrap();
    let host_id = seed_host(&pool, "offline", "OFFLINE").await?;
    let emulator_id = seed_emulator(&pool, host_id, "offline", "OFFLINE").await?;
    let fixture = seed_bound_job(&pool, emulator_id, "offline", 1, now, "PENDING", None).await?;

    let scheduler = SchedulerService::new(pool.clone());

    assert_eq!(
        scheduler.claim_for_execution(fixture.job_id, now).await?,
        ClaimResult::WaitingEmulator
    );

    let status: String = sqlx::query_scalar("SELECT status FROM foster_job WHERE id = ?")
        .bind(fixture.job_id)
        .fetch_one(&pool)
        .await?;

    assert_eq!(status, "WAITING_EMULATOR");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn manual_pause_starting_while_waiting_blocks_later_claim(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 10, 0, 0).unwrap();
    let pause_until = now + Duration::hours(2);
    let host_id = seed_host(&pool, "pause", "ONLINE").await?;
    let emulator_id = seed_emulator(&pool, host_id, "pause", "IDLE").await?;
    let fixture = seed_bound_job(
        &pool,
        emulator_id,
        "pause",
        1,
        now,
        "WAITING_EMULATOR",
        Some(pause_until),
    )
    .await?;

    let scheduler = SchedulerService::new(pool.clone());

    assert_eq!(
        scheduler.claim_for_execution(fixture.job_id, now).await?,
        ClaimResult::DeferredManual(pause_until)
    );

    let status: String = sqlx::query_scalar("SELECT status FROM foster_job WHERE id = ?")
        .bind(fixture.job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "DEFERRED_MANUAL");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn quiet_period_starting_while_waiting_blocks_later_claim(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 12, 30, 0).unwrap();
    let expected = Utc.with_ymd_and_hms(2026, 9, 21, 15, 0, 0).unwrap();
    let host_id = seed_host(&pool, "quiet", "ONLINE").await?;
    let emulator_id = seed_emulator(&pool, host_id, "quiet", "IDLE").await?;
    let fixture = seed_bound_job(
        &pool,
        emulator_id,
        "quiet",
        1,
        now,
        "WAITING_EMULATOR",
        None,
    )
    .await?;

    add_quiet_period(&pool, fixture.account_id).await?;

    let scheduler = SchedulerService::new(pool.clone());

    assert_eq!(
        scheduler.claim_for_execution(fixture.job_id, now).await?,
        ClaimResult::DeferredQuiet(expected)
    );

    let status: String = sqlx::query_scalar("SELECT status FROM foster_job WHERE id = ?")
        .bind(fixture.job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "DEFERRED_QUIET");

    Ok(())
}
