use chrono::{TimeZone, Utc};
use foster_server::scheduler::SchedulerService;
use sqlx::MySqlPool;

struct AccountFixture {
    account_id: i64,
    subscription_id: i64,
}

async fn seed_host_emulator(pool: &MySqlPool) -> anyhow::Result<(i64, i64)> {
    let host_id = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('host-run-once', 'host-run-once', 'ONLINE')",
    )
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let emulator_id = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status, lifecycle_status
         )
         VALUES (?, 'emu-run-once', 'FAKE', 5, 'IDLE', 'READY')",
    )
    .bind(host_id)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    Ok((host_id, emulator_id))
}

async fn seed_plan(pool: &MySqlPool) -> anyhow::Result<i64> {
    Ok(sqlx::query(
        "INSERT INTO foster_plan(
            plan_code, plan_name, daily_target_runs,
            interval_minutes, resource_mode, status
         )
         VALUES ('PLAN-RUN-ONCE', 'Plan Run Once', 4, 360,
                 'USER_FRIEND', 'ACTIVE')",
    )
    .execute(pool)
    .await?
    .last_insert_id() as i64)
}

async fn seed_account_subscription(
    pool: &MySqlPool,
    emulator_id: i64,
    plan_id: i64,
    customer_id: i64,
    slot_no: i32,
    subscription_no: &str,
    next_run_at: Option<chrono::DateTime<Utc>>,
) -> anyhow::Result<AccountFixture> {
    let account_id = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status, active_emulator_id
         )
         VALUES (?, 'LOGGED_IN', 'VERIFIED', ?)",
    )
    .bind(customer_id)
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

    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();

    let subscription_id = sqlx::query(
        "INSERT INTO foster_subscription(
            subscription_no, game_account_id, plan_id,
            resource_mode, daily_target_runs, interval_minutes,
            status, start_at, end_at, next_run_at
         )
         VALUES (?, ?, ?, 'USER_FRIEND', 4, 360,
                 'ACTIVE', ?, ?, ?)",
    )
    .bind(subscription_no)
    .bind(account_id)
    .bind(plan_id)
    .bind((now - chrono::Duration::days(1)).naive_utc())
    .bind((now + chrono::Duration::days(30)).naive_utc())
    .bind(next_run_at.map(|value| value.naive_utc()))
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    Ok(AccountFixture {
        account_id,
        subscription_id,
    })
}

async fn insert_job(
    pool: &MySqlPool,
    job_no: &str,
    fixture: &AccountFixture,
    status: &str,
    scheduled_at: chrono::DateTime<Utc>,
    deferred_until: Option<chrono::DateTime<Utc>>,
    retry_after: Option<chrono::DateTime<Utc>>,
) -> anyhow::Result<i64> {
    Ok(sqlx::query(
        "INSERT INTO foster_job(
            job_no, subscription_id, game_account_id, status,
            scheduled_at, deferred_until, retry_after
         )
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(job_no)
    .bind(fixture.subscription_id)
    .bind(fixture.account_id)
    .bind(status)
    .bind(scheduled_at.naive_utc())
    .bind(deferred_until.map(|value| value.naive_utc()))
    .bind(retry_after.map(|value| value.naive_utc()))
    .execute(pool)
    .await?
    .last_insert_id() as i64)
}

#[sqlx::test(migrations = "../../migrations")]
async fn one_tick_creates_and_claims_due_job(pool: MySqlPool) -> anyhow::Result<()> {
    let (_, emulator_id) = seed_host_emulator(&pool).await?;
    let plan_id = seed_plan(&pool).await?;
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();

    let fixture = seed_account_subscription(
        &pool,
        emulator_id,
        plan_id,
        90001,
        1,
        "SUB-RUN-ONCE-1",
        Some(now - chrono::Duration::minutes(1)),
    )
    .await?;

    let scheduler = SchedulerService::new(pool.clone());
    let report = scheduler.run_once(now).await?;

    assert_eq!(report.created_job_ids.len(), 1);
    assert_eq!(report.claimed_job_ids, report.created_job_ids);

    let status: String =
        sqlx::query_scalar("SELECT status FROM foster_job WHERE subscription_id = ?")
            .bind(fixture.subscription_id)
            .fetch_one(&pool)
            .await?;

    assert_eq!(status, "SWITCHING_ACCOUNT");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn repeated_tick_is_idempotent(pool: MySqlPool) -> anyhow::Result<()> {
    let (_, emulator_id) = seed_host_emulator(&pool).await?;
    let plan_id = seed_plan(&pool).await?;
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();

    seed_account_subscription(
        &pool,
        emulator_id,
        plan_id,
        90001,
        1,
        "SUB-RUN-ONCE-2",
        Some(now - chrono::Duration::minutes(1)),
    )
    .await?;

    let scheduler = SchedulerService::new(pool.clone());

    let first = scheduler.run_once(now).await?;
    let second = scheduler.run_once(now).await?;

    assert_eq!(first.created_job_ids.len(), 1);
    assert_eq!(first.claimed_job_ids.len(), 1);
    assert!(second.created_job_ids.is_empty());
    assert!(second.claimed_job_ids.is_empty());
    assert!(second.waiting_job_ids.is_empty());

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM foster_job")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn expired_manual_deferral_resumes_same_job(pool: MySqlPool) -> anyhow::Result<()> {
    let (_, emulator_id) = seed_host_emulator(&pool).await?;
    let plan_id = seed_plan(&pool).await?;
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();

    let fixture = seed_account_subscription(
        &pool,
        emulator_id,
        plan_id,
        90001,
        1,
        "SUB-RUN-ONCE-3",
        None,
    )
    .await?;

    sqlx::query(
        "UPDATE foster_subscription
         SET manual_pause_until = ?
         WHERE id = ?",
    )
    .bind((now - chrono::Duration::minutes(1)).naive_utc())
    .bind(fixture.subscription_id)
    .execute(&pool)
    .await?;

    let job_id = insert_job(
        &pool,
        "JOB-DEFERRED-RUN-ONCE",
        &fixture,
        "DEFERRED_MANUAL",
        now - chrono::Duration::hours(1),
        Some(now - chrono::Duration::minutes(1)),
        None,
    )
    .await?;

    let scheduler = SchedulerService::new(pool.clone());
    let report = scheduler.run_once(now).await?;

    assert_eq!(report.claimed_job_ids, vec![job_id]);

    let status: String = sqlx::query_scalar("SELECT status FROM foster_job WHERE id = ?")
        .bind(job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "SWITCHING_ACCOUNT");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn retry_job_waits_until_retry_after(pool: MySqlPool) -> anyhow::Result<()> {
    let (_, emulator_id) = seed_host_emulator(&pool).await?;
    let plan_id = seed_plan(&pool).await?;
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();

    let fixture = seed_account_subscription(
        &pool,
        emulator_id,
        plan_id,
        90001,
        1,
        "SUB-RUN-ONCE-4",
        None,
    )
    .await?;

    let retry_after = now + chrono::Duration::minutes(5);
    let job_id = insert_job(
        &pool,
        "JOB-RETRY-RUN-ONCE",
        &fixture,
        "RETRY",
        now - chrono::Duration::minutes(1),
        None,
        Some(retry_after),
    )
    .await?;

    let scheduler = SchedulerService::new(pool.clone());

    let early = scheduler.run_once(now).await?;
    assert!(early.claimed_job_ids.is_empty());

    let due = scheduler.run_once(retry_after).await?;
    assert_eq!(due.claimed_job_ids, vec![job_id]);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn emulator_contention_leaves_second_job_waiting(pool: MySqlPool) -> anyhow::Result<()> {
    let (_, emulator_id) = seed_host_emulator(&pool).await?;
    let plan_id = seed_plan(&pool).await?;
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();

    let first = seed_account_subscription(
        &pool,
        emulator_id,
        plan_id,
        90001,
        1,
        "SUB-RUN-ONCE-5A",
        None,
    )
    .await?;
    let second = seed_account_subscription(
        &pool,
        emulator_id,
        plan_id,
        90002,
        2,
        "SUB-RUN-ONCE-5B",
        None,
    )
    .await?;

    let first_job = insert_job(
        &pool,
        "JOB-CONTENTION-A",
        &first,
        "PENDING",
        now - chrono::Duration::minutes(2),
        None,
        None,
    )
    .await?;
    let second_job = insert_job(
        &pool,
        "JOB-CONTENTION-B",
        &second,
        "PENDING",
        now - chrono::Duration::minutes(1),
        None,
        None,
    )
    .await?;

    let scheduler = SchedulerService::new(pool.clone());
    let report = scheduler.run_once(now).await?;

    assert_eq!(report.claimed_job_ids, vec![first_job]);
    assert_eq!(report.waiting_job_ids, vec![second_job]);

    let statuses: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, status
         FROM foster_job
         WHERE id IN (?, ?)
         ORDER BY id",
    )
    .bind(first_job)
    .bind(second_job)
    .fetch_all(&pool)
    .await?;

    assert_eq!(statuses[0].1, "SWITCHING_ACCOUNT");
    assert_eq!(statuses[1].1, "WAITING_EMULATOR");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn waiting_resource_job_is_only_reclaimed_after_retry_after(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let (_, emulator_id) = seed_host_emulator(&pool).await?;
    let plan_id = seed_plan(&pool).await?;
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();

    let fixture = seed_account_subscription(
        &pool,
        emulator_id,
        plan_id,
        90006,
        1,
        "SUB-RUN-ONCE-RESOURCE",
        None,
    )
    .await?;

    let retry_after = now + chrono::Duration::seconds(60);
    let job_id = insert_job(
        &pool,
        "JOB-WAITING-RESOURCE",
        &fixture,
        "WAITING_RESOURCE",
        now - chrono::Duration::minutes(10),
        None,
        Some(retry_after),
    )
    .await?;

    let scheduler = SchedulerService::new(pool.clone());

    let early = scheduler.run_once(now).await?;
    assert!(early.claimed_job_ids.is_empty());

    let due = scheduler.run_once(retry_after).await?;
    assert_eq!(due.claimed_job_ids, vec![job_id]);

    let status: String = sqlx::query_scalar("SELECT status FROM foster_job WHERE id = ?")
        .bind(job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "SWITCHING_ACCOUNT");

    Ok(())
}
