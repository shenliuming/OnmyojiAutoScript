use chrono::{Duration, TimeZone, Utc};
use foster_server::scheduler::{JobGateResult, SchedulerService};
use sqlx::MySqlPool;

struct GateFixture {
    account_id: i64,
    subscription_id: i64,
    job_id: i64,
}

async fn seed_gate_fixture(
    pool: &MySqlPool,
    suffix: &str,
    now: chrono::DateTime<Utc>,
    manual_pause_until: Option<chrono::DateTime<Utc>>,
    job_status: &str,
    deferred_until: Option<chrono::DateTime<Utc>>,
) -> anyhow::Result<GateFixture> {
    let host_id = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES (?, ?, 'ONLINE')",
    )
    .bind(format!("host-gate-{suffix}"))
    .bind(format!("host-gate-{suffix}"))
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
    .bind(format!("emu-gate-{suffix}"))
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let account_id = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status, active_emulator_id
         )
         VALUES (?, 'LOGGED_IN', 'VERIFIED', ?)",
    )
    .bind(40_000 + host_id)
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
         VALUES (?, ?, 4, 360, 'USER_FRIEND', 'ACTIVE')",
    )
    .bind(format!("PLAN-GATE-{suffix}"))
    .bind(format!("Plan Gate {suffix}"))
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
    .bind(format!("SUB-GATE-{suffix}"))
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
            status, scheduled_at, deferred_until
         )
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(format!("JOB-GATE-{suffix}"))
    .bind(subscription_id)
    .bind(account_id)
    .bind(job_status)
    .bind(now.naive_utc())
    .bind(deferred_until.map(|value| value.naive_utc()))
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    Ok(GateFixture {
        account_id,
        subscription_id,
        job_id,
    })
}

async fn add_quiet_period(
    pool: &MySqlPool,
    account_id: i64,
    weekday_mask: u8,
    start: &str,
    end: &str,
    before_buffer_minutes: i32,
    after_buffer_minutes: i32,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO foster_quiet_period(
            game_account_id, weekday_mask,
            start_time, end_time, timezone,
            before_buffer_minutes, after_buffer_minutes, enabled
         )
         VALUES (?, ?, ?, ?, 'Asia/Shanghai', ?, ?, 1)",
    )
    .bind(account_id)
    .bind(weekday_mask)
    .bind(start)
    .bind(end)
    .bind(before_buffer_minutes)
    .bind(after_buffer_minutes)
    .execute(pool)
    .await?;

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn manual_pause_outranks_quiet_period(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 12, 30, 0).unwrap();
    let pause_until = now + Duration::minutes(90);
    let fixture = seed_gate_fixture(
        &pool,
        "manual-first",
        now,
        Some(pause_until),
        "PENDING",
        None,
    )
    .await?;

    add_quiet_period(&pool, fixture.account_id, 1, "20:00:00", "23:00:00", 0, 0).await?;

    let scheduler = SchedulerService::new(pool.clone());
    assert_eq!(
        scheduler.gate_pending_job(fixture.job_id, now).await?,
        JobGateResult::DeferredManual(pause_until)
    );

    let row: (String, Option<chrono::NaiveDateTime>) = sqlx::query_as(
        "SELECT status, deferred_until
         FROM foster_job
         WHERE id = ?",
    )
    .bind(fixture.job_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.0, "DEFERRED_MANUAL");
    assert_eq!(row.1, Some(pause_until.naive_utc()));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn quiet_deferral_records_exact_end(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 12, 30, 0).unwrap();
    let expected = Utc.with_ymd_and_hms(2026, 9, 21, 15, 5, 0).unwrap();
    let fixture = seed_gate_fixture(&pool, "quiet", now, None, "PENDING", None).await?;

    add_quiet_period(&pool, fixture.account_id, 1, "20:00:00", "23:00:00", 0, 5).await?;

    let scheduler = SchedulerService::new(pool.clone());
    assert_eq!(
        scheduler.gate_pending_job(fixture.job_id, now).await?,
        JobGateResult::DeferredQuiet(expected)
    );

    let row: (String, Option<chrono::NaiveDateTime>) = sqlx::query_as(
        "SELECT status, deferred_until
         FROM foster_job
         WHERE id = ?",
    )
    .bind(fixture.job_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.0, "DEFERRED_QUIET");
    assert_eq!(row.1, Some(expected.naive_utc()));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn cross_midnight_quiet_period_loaded_from_database(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 16, 30, 0).unwrap();
    let expected = Utc.with_ymd_and_hms(2026, 9, 21, 17, 0, 0).unwrap();
    let fixture = seed_gate_fixture(&pool, "cross-midnight", now, None, "PENDING", None).await?;

    add_quiet_period(&pool, fixture.account_id, 1, "23:00:00", "01:00:00", 0, 0).await?;

    let scheduler = SchedulerService::new(pool);
    assert_eq!(
        scheduler.gate_pending_job(fixture.job_id, now).await?,
        JobGateResult::DeferredQuiet(expected)
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn expired_manual_pause_resumes_same_job(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 10, 0, 0).unwrap();
    let fixture = seed_gate_fixture(
        &pool,
        "resume",
        now,
        Some(now - Duration::minutes(1)),
        "DEFERRED_MANUAL",
        Some(now - Duration::minutes(1)),
    )
    .await?;

    let scheduler = SchedulerService::new(pool.clone());
    assert_eq!(
        scheduler.gate_pending_job(fixture.job_id, now).await?,
        JobGateResult::Executable
    );

    let row: (String, Option<chrono::NaiveDateTime>) = sqlx::query_as(
        "SELECT status, deferred_until
         FROM foster_job
         WHERE id = ?",
    )
    .bind(fixture.job_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.0, "PENDING");
    assert!(row.1.is_none());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn repeated_gate_is_idempotent_and_does_not_create_catch_up_job(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 12, 30, 0).unwrap();
    let expected = Utc.with_ymd_and_hms(2026, 9, 21, 15, 0, 0).unwrap();
    let fixture = seed_gate_fixture(&pool, "repeat", now, None, "PENDING", None).await?;

    add_quiet_period(&pool, fixture.account_id, 1, "20:00:00", "23:00:00", 0, 0).await?;

    let scheduler = SchedulerService::new(pool.clone());
    assert_eq!(
        scheduler.gate_pending_job(fixture.job_id, now).await?,
        JobGateResult::DeferredQuiet(expected)
    );
    assert_eq!(
        scheduler.gate_pending_job(fixture.job_id, now).await?,
        JobGateResult::DeferredQuiet(expected)
    );

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM foster_job
         WHERE subscription_id = ?",
    )
    .bind(fixture.subscription_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(count, 1);
    Ok(())
}
