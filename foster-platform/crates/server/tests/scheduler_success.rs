use chrono::{TimeZone, Utc};
use foster_server::scheduler::{SchedulerError, SchedulerService};
use sqlx::MySqlPool;

struct Fixture {
    account_id: i64,
    subscription_id: i64,
    job_id: i64,
}

async fn seed_running_job(pool: &MySqlPool) -> anyhow::Result<Fixture> {
    let host_id = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('host-success', 'host-success', 'ONLINE')",
    )
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let emulator_id = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status
         )
         VALUES (?, 'emu-success', 'FAKE', 5, 'IDLE')",
    )
    .bind(host_id)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let account_id = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status, active_emulator_id
         )
         VALUES (70001, 'LOGGED_IN', 'VERIFIED', ?)",
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
         VALUES ('PLAN-SUCCESS', 'Plan Success', 4, 360,
                 'USER_FRIEND', 'ACTIVE')",
    )
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let base = Utc.with_ymd_and_hms(2026, 9, 22, 8, 0, 0).unwrap();

    let subscription_id = sqlx::query(
        "INSERT INTO foster_subscription(
            subscription_no, game_account_id, plan_id,
            resource_mode, daily_target_runs, interval_minutes,
            status, start_at, end_at, next_run_at
         )
         VALUES ('SUB-SUCCESS', ?, ?, 'USER_FRIEND', 4, 360,
                 'ACTIVE', ?, ?, NULL)",
    )
    .bind(account_id)
    .bind(plan_id)
    .bind((base - chrono::Duration::days(1)).naive_utc())
    .bind((base + chrono::Duration::days(30)).naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let job_id = sqlx::query(
        "INSERT INTO foster_job(
            job_no, subscription_id, game_account_id,
            emulator_id, status, scheduled_at, started_at
         )
         VALUES ('JOB-SUCCESS', ?, ?, ?, 'RUNNING', ?, ?)",
    )
    .bind(subscription_id)
    .bind(account_id)
    .bind(emulator_id)
    .bind((base - chrono::Duration::hours(6)).naive_utc())
    .bind((base - chrono::Duration::minutes(3)).naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    Ok(Fixture {
        account_id,
        subscription_id,
        job_id,
    })
}

#[sqlx::test(migrations = "../../migrations")]
async fn success_falls_back_to_configured_interval(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_running_job(&pool).await?;
    let scheduler = SchedulerService::new(pool.clone());
    let success_at = Utc.with_ymd_and_hms(2026, 9, 22, 10, 15, 0).unwrap();

    let next = scheduler
        .complete_success(fixture.job_id, success_at, None)
        .await?;

    assert_eq!(next, success_at + chrono::Duration::minutes(360));

    let row: (
        String,
        Option<chrono::NaiveDateTime>,
        Option<chrono::NaiveDateTime>,
    ) = sqlx::query_as(
        "SELECT status, finished_at, remaining_seconds
         FROM foster_job
         WHERE id = ?",
    )
    .bind(fixture.job_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.0, "SUCCESS");
    assert!(row.1.is_some());
    assert!(row.2.is_none());

    let subscription: (Option<chrono::NaiveDateTime>, Option<chrono::NaiveDateTime>) =
        sqlx::query_as(
            "SELECT last_success_at, next_run_at
         FROM foster_subscription
         WHERE id = ?",
        )
        .bind(fixture.subscription_id)
        .fetch_one(&pool)
        .await?;

    assert_eq!(subscription.0, Some(success_at.naive_utc()));
    assert_eq!(
        subscription.1,
        Some((success_at + chrono::Duration::minutes(360)).naive_utc())
    );

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn ocr_remaining_seconds_override_interval(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_running_job(&pool).await?;
    let scheduler = SchedulerService::new(pool.clone());
    let success_at = Utc.with_ymd_and_hms(2026, 9, 22, 10, 15, 0).unwrap();

    let next = scheduler
        .complete_success(fixture.job_id, success_at, Some(1_800))
        .await?;

    assert_eq!(next, success_at + chrono::Duration::seconds(1_800));

    let remaining: Option<i32> =
        sqlx::query_scalar("SELECT remaining_seconds FROM foster_job WHERE id = ?")
            .bind(fixture.job_id)
            .fetch_one(&pool)
            .await?;

    assert_eq!(remaining, Some(1_800));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn next_run_is_based_on_actual_success_time(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_running_job(&pool).await?;
    let scheduler = SchedulerService::new(pool.clone());
    let success_at = Utc.with_ymd_and_hms(2026, 9, 22, 17, 42, 0).unwrap();

    let next = scheduler
        .complete_success(fixture.job_id, success_at, None)
        .await?;

    assert_eq!(next, success_at + chrono::Duration::hours(6));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn same_job_cannot_be_completed_twice(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_running_job(&pool).await?;
    let scheduler = SchedulerService::new(pool.clone());
    let success_at = Utc.with_ymd_and_hms(2026, 9, 22, 10, 15, 0).unwrap();

    scheduler
        .complete_success(fixture.job_id, success_at, None)
        .await?;

    let second = scheduler
        .complete_success(
            fixture.job_id,
            success_at + chrono::Duration::minutes(1),
            None,
        )
        .await;

    assert!(matches!(second, Err(SchedulerError::JobNotRunning)));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn daily_success_count_only_counts_success(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_running_job(&pool).await?;
    let scheduler = SchedulerService::new(pool.clone());
    let success_at = Utc.with_ymd_and_hms(2026, 9, 22, 10, 15, 0).unwrap();

    scheduler
        .complete_success(fixture.job_id, success_at, None)
        .await?;

    sqlx::query(
        "INSERT INTO foster_job(
            job_no, subscription_id, game_account_id,
            status, scheduled_at, finished_at
         )
         VALUES ('JOB-FAILED-SAME-DAY', ?, ?, 'FAILED', ?, ?)",
    )
    .bind(fixture.subscription_id)
    .bind(fixture.account_id)
    .bind((success_at + chrono::Duration::minutes(5)).naive_utc())
    .bind((success_at + chrono::Duration::minutes(6)).naive_utc())
    .execute(&pool)
    .await?;

    let start = Utc.with_ymd_and_hms(2026, 9, 22, 0, 0, 0).unwrap();
    let end = Utc.with_ymd_and_hms(2026, 9, 23, 0, 0, 0).unwrap();

    let count = scheduler
        .count_successes_between(fixture.account_id, start, end)
        .await?;

    assert_eq!(count, 1);
    Ok(())
}
