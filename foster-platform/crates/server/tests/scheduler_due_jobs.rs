use chrono::{Duration, Utc};
use foster_server::scheduler::SchedulerService;
use sqlx::MySqlPool;

async fn seed_active_subscription(
    pool: &MySqlPool,
    suffix: &str,
    next_run_at: chrono::DateTime<Utc>,
    start_at: chrono::DateTime<Utc>,
    end_at: chrono::DateTime<Utc>,
    status: &str,
) -> anyhow::Result<i64> {
    let host = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES (?, ?, 'ONLINE')",
    )
    .bind(format!("host-due-{suffix}"))
    .bind(format!("host-due-{suffix}"))
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let emulator = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status, lifecycle_status
         )
         VALUES (?, ?, 'FAKE', 5, 'IDLE', 'READY')",
    )
    .bind(host)
    .bind(format!("emu-due-{suffix}"))
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let account = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status, active_emulator_id
         )
         VALUES (?, 'LOGGED_IN', 'VERIFIED', ?)",
    )
    .bind(30_000 + host)
    .bind(emulator)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    sqlx::query(
        "INSERT INTO emulator_account_binding(
            emulator_id, game_account_id, slot_no, status, bound_at
         )
         VALUES (?, ?, 1, 'ACTIVE', NOW(3))",
    )
    .bind(emulator)
    .bind(account)
    .execute(pool)
    .await?;

    let plan = sqlx::query(
        "INSERT INTO foster_plan(
            plan_code, plan_name, daily_target_runs,
            interval_minutes, resource_mode, status
         )
         VALUES (?, ?, 4, 360, 'USER_FRIEND', 'ACTIVE')",
    )
    .bind(format!("PLAN-DUE-{suffix}"))
    .bind(format!("Plan Due {suffix}"))
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let subscription = sqlx::query(
        "INSERT INTO foster_subscription(
            subscription_no, game_account_id, plan_id,
            resource_mode, daily_target_runs, interval_minutes,
            status, start_at, end_at, next_run_at
         )
         VALUES (?, ?, ?, 'USER_FRIEND', 4, 360, ?, ?, ?, ?)",
    )
    .bind(format!("SUB-DUE-{suffix}"))
    .bind(account)
    .bind(plan)
    .bind(status)
    .bind(start_at.naive_utc())
    .bind(end_at.naive_utc())
    .bind(next_run_at.naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    Ok(subscription)
}

#[sqlx::test(migrations = "../../migrations")]
async fn due_subscription_creates_one_job(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc::now();
    let subscription = seed_active_subscription(
        &pool,
        "due",
        now - Duration::minutes(1),
        now - Duration::days(1),
        now + Duration::days(30),
        "ACTIVE",
    )
    .await?;

    let scheduler = SchedulerService::new(pool.clone());
    let created = scheduler.create_due_jobs(now, 10).await?;

    assert_eq!(created.len(), 1);

    let job_subscription: i64 = sqlx::query_scalar(
        "SELECT subscription_id
         FROM foster_job
         WHERE id = ?",
    )
    .bind(created[0])
    .fetch_one(&pool)
    .await?;

    assert_eq!(job_subscription, subscription);

    let next_run_at: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
        "SELECT next_run_at
         FROM foster_subscription
         WHERE id = ?",
    )
    .bind(subscription)
    .fetch_one(&pool)
    .await?;

    assert!(next_run_at.is_none());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn not_yet_due_subscription_is_ignored(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc::now();
    seed_active_subscription(
        &pool,
        "future",
        now + Duration::hours(1),
        now - Duration::days(1),
        now + Duration::days(30),
        "ACTIVE",
    )
    .await?;

    let scheduler = SchedulerService::new(pool);
    assert!(scheduler.create_due_jobs(now, 10).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn expired_or_inactive_subscription_is_ignored(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc::now();

    seed_active_subscription(
        &pool,
        "expired",
        now - Duration::minutes(1),
        now - Duration::days(30),
        now - Duration::seconds(1),
        "ACTIVE",
    )
    .await?;

    seed_active_subscription(
        &pool,
        "paused",
        now - Duration::minutes(1),
        now - Duration::days(1),
        now + Duration::days(30),
        "PAUSED",
    )
    .await?;

    let scheduler = SchedulerService::new(pool);
    assert!(scheduler.create_due_jobs(now, 10).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn repeated_scheduler_ticks_are_idempotent(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc::now();
    let subscription = seed_active_subscription(
        &pool,
        "repeat",
        now - Duration::minutes(1),
        now - Duration::days(1),
        now + Duration::days(30),
        "ACTIVE",
    )
    .await?;

    let scheduler = SchedulerService::new(pool.clone());
    assert_eq!(scheduler.create_due_jobs(now, 10).await?.len(), 1);
    assert!(scheduler.create_due_jobs(now, 10).await?.is_empty());

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM foster_job
         WHERE subscription_id = ?",
    )
    .bind(subscription)
    .fetch_one(&pool)
    .await?;

    assert_eq!(count, 1);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_schedulers_create_exactly_one_job(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc::now();
    let subscription = seed_active_subscription(
        &pool,
        "race",
        now - Duration::minutes(1),
        now - Duration::days(1),
        now + Duration::days(30),
        "ACTIVE",
    )
    .await?;

    let left = SchedulerService::new(pool.clone());
    let right = SchedulerService::new(pool.clone());

    let (left_result, right_result) = tokio::join!(
        left.create_due_jobs(now, 10),
        right.create_due_jobs(now, 10),
    );

    let total_created = left_result?.len() + right_result?.len();
    assert_eq!(total_created, 1);

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM foster_job
         WHERE subscription_id = ?",
    )
    .bind(subscription)
    .fetch_one(&pool)
    .await?;

    assert_eq!(count, 1);
    Ok(())
}
