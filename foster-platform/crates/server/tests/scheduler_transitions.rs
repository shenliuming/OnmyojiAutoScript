use chrono::{TimeZone, Utc};
use foster_domain::FosterJobStatus;
use foster_server::scheduler::SchedulerService;
use sqlx::MySqlPool;

async fn seed_job(pool: &MySqlPool, status: &str) -> anyhow::Result<i64> {
    let host_id = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('host-transition', 'host-transition', 'ONLINE')",
    )
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let emulator_id = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status
         )
         VALUES (?, 'emu-transition', 'FAKE', 5, 'IDLE')",
    )
    .bind(host_id)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let account_id = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status, active_emulator_id
         )
         VALUES (60001, 'LOGGED_IN', 'VERIFIED', ?)",
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
         VALUES ('PLAN-TRANSITION', 'Plan Transition', 4, 360,
                 'USER_FRIEND', 'ACTIVE')",
    )
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let now = Utc.with_ymd_and_hms(2026, 9, 21, 10, 0, 0).unwrap();

    let subscription_id = sqlx::query(
        "INSERT INTO foster_subscription(
            subscription_no, game_account_id, plan_id,
            resource_mode, daily_target_runs, interval_minutes,
            status, start_at, end_at, next_run_at
         )
         VALUES ('SUB-TRANSITION', ?, ?, 'USER_FRIEND', 4, 360,
                 'ACTIVE', ?, ?, NULL)",
    )
    .bind(account_id)
    .bind(plan_id)
    .bind((now - chrono::Duration::days(1)).naive_utc())
    .bind((now + chrono::Duration::days(30)).naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    Ok(sqlx::query(
        "INSERT INTO foster_job(
            job_no, subscription_id, game_account_id,
            emulator_id, status, scheduled_at
         )
         VALUES ('JOB-TRANSITION', ?, ?, ?, ?, ?)",
    )
    .bind(subscription_id)
    .bind(account_id)
    .bind(emulator_id)
    .bind(status)
    .bind(now.naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64)
}

#[sqlx::test(migrations = "../../migrations")]
async fn happy_path_transitions_to_success(pool: MySqlPool) -> anyhow::Result<()> {
    let job_id = seed_job(&pool, "SWITCHING_ACCOUNT").await?;
    let scheduler = SchedulerService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 10, 5, 0).unwrap();

    assert!(
        scheduler
            .transition_job(
                job_id,
                FosterJobStatus::SwitchingAccount,
                FosterJobStatus::VerifyingAccount,
                now,
            )
            .await?
    );
    assert!(
        scheduler
            .transition_job(
                job_id,
                FosterJobStatus::VerifyingAccount,
                FosterJobStatus::Running,
                now + chrono::Duration::minutes(1),
            )
            .await?
    );
    assert!(
        scheduler
            .transition_job(
                job_id,
                FosterJobStatus::Running,
                FosterJobStatus::Success,
                now + chrono::Duration::minutes(2),
            )
            .await?
    );

    let row: (String, Option<chrono::NaiveDateTime>, Option<chrono::NaiveDateTime>) =
        sqlx::query_as(
            "SELECT status, started_at, finished_at
             FROM foster_job
             WHERE id = ?",
        )
        .bind(job_id)
        .fetch_one(&pool)
        .await?;

    assert_eq!(row.0, "SUCCESS");
    assert!(row.1.is_some());
    assert!(row.2.is_some());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn stale_expected_state_returns_false(pool: MySqlPool) -> anyhow::Result<()> {
    let job_id = seed_job(&pool, "SWITCHING_ACCOUNT").await?;
    let scheduler = SchedulerService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 10, 5, 0).unwrap();

    assert!(
        !scheduler
            .transition_job(
                job_id,
                FosterJobStatus::VerifyingAccount,
                FosterJobStatus::Running,
                now,
            )
            .await?
    );

    let status: String = sqlx::query_scalar("SELECT status FROM foster_job WHERE id = ?")
        .bind(job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "SWITCHING_ACCOUNT");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn running_cannot_jump_back_to_switching(pool: MySqlPool) -> anyhow::Result<()> {
    let job_id = seed_job(&pool, "RUNNING").await?;
    let scheduler = SchedulerService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 10, 5, 0).unwrap();

    assert!(
        !scheduler
            .transition_job(
                job_id,
                FosterJobStatus::Running,
                FosterJobStatus::SwitchingAccount,
                now,
            )
            .await?
    );

    let status: String = sqlx::query_scalar("SELECT status FROM foster_job WHERE id = ?")
        .bind(job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "RUNNING");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn identity_mismatch_is_terminal(pool: MySqlPool) -> anyhow::Result<()> {
    let job_id = seed_job(&pool, "RUNNING").await?;
    let scheduler = SchedulerService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 10, 5, 0).unwrap();

    assert!(
        scheduler
            .transition_job(
                job_id,
                FosterJobStatus::Running,
                FosterJobStatus::IdentityMismatch,
                now,
            )
            .await?
    );

    assert!(
        !scheduler
            .transition_job(
                job_id,
                FosterJobStatus::IdentityMismatch,
                FosterJobStatus::Retry,
                now + chrono::Duration::minutes(1),
            )
            .await?
    );

    let row: (String, Option<chrono::NaiveDateTime>) = sqlx::query_as(
        "SELECT status, finished_at
         FROM foster_job
         WHERE id = ?",
    )
    .bind(job_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.0, "IDENTITY_MISMATCH");
    assert!(row.1.is_some());
    Ok(())
}
