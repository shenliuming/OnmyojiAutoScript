use sqlx::MySqlPool;

async fn seed_host(pool: &MySqlPool, suffix: &str) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES (?, ?, 'ONLINE')",
    )
    .bind(format!("host-scheduler-{suffix}"))
    .bind(format!("host-scheduler-{suffix}"))
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn seed_emulator(pool: &MySqlPool, host_id: i64, suffix: &str) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status
         )
         VALUES (?, ?, 'FAKE', 5, 'IDLE')",
    )
    .bind(host_id)
    .bind(format!("emu-scheduler-{suffix}"))
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn seed_account(pool: &MySqlPool, customer_id: i64) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status
         )
         VALUES (?, 'LOGGED_IN', 'VERIFIED')",
    )
    .bind(customer_id)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn seed_plan(pool: &MySqlPool, suffix: &str) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO foster_plan(
            plan_code, plan_name, daily_target_runs,
            interval_minutes, resource_mode, status
         )
         VALUES (?, ?, 4, 360, 'USER_FRIEND', 'ACTIVE')",
    )
    .bind(format!("PLAN-{suffix}"))
    .bind(format!("Plan {suffix}"))
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn seed_subscription(
    pool: &MySqlPool,
    suffix: &str,
    game_account_id: i64,
    plan_id: i64,
) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO foster_subscription(
            subscription_no, game_account_id, plan_id,
            resource_mode, daily_target_runs, interval_minutes,
            status, start_at, end_at, next_run_at
         )
         VALUES (?, ?, ?, 'USER_FRIEND', 4, 360,
                 'ACTIVE', DATE_SUB(NOW(3), INTERVAL 1 DAY),
                 DATE_ADD(NOW(3), INTERVAL 30 DAY), NOW(3))",
    )
    .bind(format!("SUB-{suffix}"))
    .bind(game_account_id)
    .bind(plan_id)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn insert_job(
    pool: &MySqlPool,
    suffix: &str,
    subscription_id: i64,
    game_account_id: i64,
    emulator_id: Option<i64>,
    status: &str,
) -> sqlx::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO foster_job(
            job_no, subscription_id, game_account_id,
            emulator_id, status, scheduled_at
         )
         VALUES (?, ?, ?, ?, ?, NOW(3))",
    )
    .bind(format!("JOB-{suffix}"))
    .bind(subscription_id)
    .bind(game_account_id)
    .bind(emulator_id)
    .bind(status)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

#[sqlx::test(migrations = "../../migrations")]
async fn database_rejects_two_nonterminal_jobs_for_subscription(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let account_id = seed_account(&pool, 20001).await?;
    let plan_id = seed_plan(&pool, "one-active").await?;
    let subscription_id = seed_subscription(&pool, "one-active", account_id, plan_id).await?;

    insert_job(
        &pool,
        "one-active-a",
        subscription_id,
        account_id,
        None,
        "PENDING",
    )
    .await?;

    let second = insert_job(
        &pool,
        "one-active-b",
        subscription_id,
        account_id,
        None,
        "DEFERRED_QUIET",
    )
    .await;

    assert!(second.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn database_rejects_two_executing_jobs_for_same_account(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let host_id = seed_host(&pool, "same-account").await?;
    let emulator_a = seed_emulator(&pool, host_id, "same-account-a").await?;
    let emulator_b = seed_emulator(&pool, host_id, "same-account-b").await?;
    let account_id = seed_account(&pool, 20002).await?;
    let plan_a = seed_plan(&pool, "same-account-a").await?;
    let plan_b = seed_plan(&pool, "same-account-b").await?;
    let subscription_a = seed_subscription(&pool, "same-account-a", account_id, plan_a).await?;
    let subscription_b = seed_subscription(&pool, "same-account-b", account_id, plan_b).await?;

    insert_job(
        &pool,
        "same-account-a",
        subscription_a,
        account_id,
        Some(emulator_a),
        "RUNNING",
    )
    .await?;

    let second = insert_job(
        &pool,
        "same-account-b",
        subscription_b,
        account_id,
        Some(emulator_b),
        "VERIFYING_ACCOUNT",
    )
    .await;

    assert!(second.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn database_rejects_two_executing_jobs_for_same_emulator(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let host_id = seed_host(&pool, "same-emulator").await?;
    let emulator_id = seed_emulator(&pool, host_id, "same-emulator").await?;
    let account_a = seed_account(&pool, 20003).await?;
    let account_b = seed_account(&pool, 20004).await?;
    let plan_a = seed_plan(&pool, "same-emulator-a").await?;
    let plan_b = seed_plan(&pool, "same-emulator-b").await?;
    let subscription_a = seed_subscription(&pool, "same-emulator-a", account_a, plan_a).await?;
    let subscription_b = seed_subscription(&pool, "same-emulator-b", account_b, plan_b).await?;

    insert_job(
        &pool,
        "same-emulator-a",
        subscription_a,
        account_a,
        Some(emulator_id),
        "RUNNING",
    )
    .await?;

    let second = insert_job(
        &pool,
        "same-emulator-b",
        subscription_b,
        account_b,
        Some(emulator_id),
        "SWITCHING_ACCOUNT",
    )
    .await;

    assert!(second.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn terminal_history_does_not_block_next_job(pool: MySqlPool) -> anyhow::Result<()> {
    let account_id = seed_account(&pool, 20005).await?;
    let plan_id = seed_plan(&pool, "history").await?;
    let subscription_id = seed_subscription(&pool, "history", account_id, plan_id).await?;

    insert_job(
        &pool,
        "history-old",
        subscription_id,
        account_id,
        None,
        "SUCCESS",
    )
    .await?;

    insert_job(
        &pool,
        "history-new",
        subscription_id,
        account_id,
        None,
        "PENDING",
    )
    .await?;

    Ok(())
}
