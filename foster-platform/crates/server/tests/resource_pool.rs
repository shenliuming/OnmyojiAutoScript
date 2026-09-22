use chrono::{TimeZone, Utc};
use foster_server::resource_pool::{ReserveForJobResult, ResourcePoolService};
use sqlx::MySqlPool;

struct JobFixture {
    account_id: i64,
    job_id: i64,
}

async fn seed_platform_job(
    pool: &MySqlPool,
    suffix: &str,
    customer_id: i64,
) -> anyhow::Result<JobFixture> {
    let account_id = sqlx::query(
        "INSERT INTO game_account(
            customer_id, character_name, server_name,
            login_status, verify_status
         )
         VALUES (?, ?, '春之樱', 'LOGGED_IN', 'VERIFIED')",
    )
    .bind(customer_id)
    .bind(format!("角色-{suffix}"))
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let plan_id = sqlx::query(
        "INSERT INTO foster_plan(
            plan_code, plan_name, daily_target_runs,
            interval_minutes, resource_mode, resource_type, status
         )
         VALUES (?, ?, 4, 360, 'PLATFORM', 'FISH', 'ACTIVE')",
    )
    .bind(format!("PLAN-{suffix}"))
    .bind(format!("Plan {suffix}"))
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let subscription_id = sqlx::query(
        "INSERT INTO foster_subscription(
            subscription_no, game_account_id, plan_id,
            resource_mode, resource_type, daily_target_runs,
            interval_minutes, status, start_at, end_at
         )
         VALUES (?, ?, ?, 'PLATFORM', 'FISH', 4, 360, 'ACTIVE', ?, ?)",
    )
    .bind(format!("SUB-{suffix}"))
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
            status, scheduled_at, started_at
         )
         VALUES (?, ?, ?, 'SWITCHING_ACCOUNT', ?, ?)",
    )
    .bind(format!("JOB-{suffix}"))
    .bind(subscription_id)
    .bind(account_id)
    .bind(now.naive_utc())
    .bind(now.naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    Ok(JobFixture { account_id, job_id })
}

async fn seed_provider_cycle(
    pool: &MySqlPool,
    suffix: &str,
    now: chrono::DateTime<Utc>,
    end_at: chrono::DateTime<Utc>,
    slot_capacity: i32,
    occupied_slots: i32,
) -> anyhow::Result<(i64, i64)> {
    let provider_id = sqlx::query(
        "INSERT INTO provider_account(
            provider_code, nickname, provider_alias, server_name, status
         )
         VALUES (?, ?, ?, '春之樱', 'ACTIVE')",
    )
    .bind(format!("PROVIDER-{suffix}"))
    .bind(format!("资源号-{suffix}"))
    .bind(format!("资源{suffix}"))
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let status = if occupied_slots >= slot_capacity {
        "FULL"
    } else {
        "AVAILABLE"
    };

    let cycle_id = sqlx::query(
        "INSERT INTO foster_resource_cycle(
            provider_account_id, resource_type, resource_level,
            start_at, end_at, slot_capacity, occupied_slots, status
         )
         VALUES (?, 'FISH', 6, ?, ?, ?, ?, ?)",
    )
    .bind(provider_id)
    .bind((now - chrono::Duration::hours(1)).naive_utc())
    .bind(end_at.naive_utc())
    .bind(slot_capacity)
    .bind(occupied_slots)
    .bind(status)
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    Ok((provider_id, cycle_id))
}

async fn bind_friend(
    pool: &MySqlPool,
    account_id: i64,
    provider_id: i64,
    status: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO foster_friend_binding(
            game_account_id, provider_account_id, status, verified_at
         )
         VALUES (?, ?, ?, NOW(3))",
    )
    .bind(account_id)
    .bind(provider_id)
    .bind(status)
    .execute(pool)
    .await?;

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn near_expiry_cycle_is_not_allocated(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let job = seed_platform_job(&pool, "NEAR", 1001).await?;
    let (provider_id, _) = seed_provider_cycle(
        &pool,
        "NEAR",
        now,
        now + chrono::Duration::minutes(300),
        2,
        0,
    )
    .await?;
    bind_friend(&pool, job.account_id, provider_id, "VERIFIED").await?;

    let result = ResourcePoolService::new(pool.clone())
        .with_min_remaining_minutes(330)
        .reserve_for_job(job.job_id, now)
        .await?;

    assert_eq!(result, ReserveForJobResult::WaitingResource);
    let status: String = sqlx::query_scalar("SELECT status FROM foster_job WHERE id = ?")
        .bind(job.job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "WAITING_RESOURCE");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn only_verified_friend_binding_is_allocatable(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let job = seed_platform_job(&pool, "SUSPECT", 1002).await?;
    let (provider_id, _) = seed_provider_cycle(
        &pool,
        "SUSPECT",
        now,
        now + chrono::Duration::hours(8),
        2,
        0,
    )
    .await?;
    bind_friend(&pool, job.account_id, provider_id, "SUSPECT").await?;

    let result = ResourcePoolService::new(pool.clone())
        .reserve_for_job(job.job_id, now)
        .await?;

    assert_eq!(result, ReserveForJobResult::WaitingResource);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn earliest_expiry_then_fuller_cycle_wins(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let job = seed_platform_job(&pool, "ORDER", 1003).await?;

    let (provider_late, _) =
        seed_provider_cycle(&pool, "LATE", now, now + chrono::Duration::hours(9), 3, 2).await?;
    let (provider_early_a, _) = seed_provider_cycle(
        &pool,
        "EARLY-A",
        now,
        now + chrono::Duration::hours(8),
        3,
        0,
    )
    .await?;
    let (provider_early_b, cycle_early_b) = seed_provider_cycle(
        &pool,
        "EARLY-B",
        now,
        now + chrono::Duration::hours(8),
        3,
        1,
    )
    .await?;

    for provider in [provider_late, provider_early_a, provider_early_b] {
        bind_friend(&pool, job.account_id, provider, "VERIFIED").await?;
    }

    let result = ResourcePoolService::new(pool.clone())
        .reserve_for_job(job.job_id, now)
        .await?;

    let ReserveForJobResult::Reserved(reservation) = result else {
        anyhow::bail!("expected reservation");
    };
    assert_eq!(reservation.provider_account_id, provider_early_b);
    assert_eq!(reservation.resource_cycle_id, cycle_early_b);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn repeated_reserve_for_same_job_is_idempotent(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let job = seed_platform_job(&pool, "IDEMP", 1004).await?;
    let (provider_id, cycle_id) =
        seed_provider_cycle(&pool, "IDEMP", now, now + chrono::Duration::hours(8), 2, 0).await?;
    bind_friend(&pool, job.account_id, provider_id, "VERIFIED").await?;

    let service = ResourcePoolService::new(pool.clone());
    let first = service.reserve_for_job(job.job_id, now).await?;
    let second = service.reserve_for_job(job.job_id, now).await?;

    assert_eq!(first, second);

    let occupied: i32 =
        sqlx::query_scalar("SELECT occupied_slots FROM foster_resource_cycle WHERE id = ?")
            .bind(cycle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(occupied, 1);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_reservations_do_not_oversell_one_slot(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let job_a = seed_platform_job(&pool, "CONC-A", 1005).await?;
    let job_b = seed_platform_job(&pool, "CONC-B", 1006).await?;
    let (provider_id, cycle_id) =
        seed_provider_cycle(&pool, "CONC", now, now + chrono::Duration::hours(8), 1, 0).await?;

    bind_friend(&pool, job_a.account_id, provider_id, "VERIFIED").await?;
    bind_friend(&pool, job_b.account_id, provider_id, "VERIFIED").await?;

    let service_a = ResourcePoolService::new(pool.clone());
    let service_b = ResourcePoolService::new(pool.clone());
    let (result_a, result_b) = tokio::join!(
        service_a.reserve_for_job(job_a.job_id, now),
        service_b.reserve_for_job(job_b.job_id, now),
    );

    let results = [result_a?, result_b?];
    let reserved = results
        .iter()
        .filter(|result| matches!(result, ReserveForJobResult::Reserved(_)))
        .count();
    let waiting = results
        .iter()
        .filter(|result| matches!(result, ReserveForJobResult::WaitingResource))
        .count();

    assert_eq!(reserved, 1);
    assert_eq!(waiting, 1);

    let occupied: i32 =
        sqlx::query_scalar("SELECT occupied_slots FROM foster_resource_cycle WHERE id = ?")
            .bind(cycle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(occupied, 1);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn release_is_idempotent_and_reopens_full_cycle(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let job = seed_platform_job(&pool, "RELEASE", 1007).await?;
    let (provider_id, cycle_id) = seed_provider_cycle(
        &pool,
        "RELEASE",
        now,
        now + chrono::Duration::hours(8),
        1,
        0,
    )
    .await?;
    bind_friend(&pool, job.account_id, provider_id, "VERIFIED").await?;

    let service = ResourcePoolService::new(pool.clone());
    service.reserve_for_job(job.job_id, now).await?;

    let first = service
        .release_for_job(job.job_id, "NO_SLOT", now + chrono::Duration::seconds(1))
        .await?;
    let second = service
        .release_for_job(job.job_id, "duplicate", now + chrono::Duration::seconds(2))
        .await?;

    assert!(first.is_some());
    assert!(second.is_none());

    let cycle: (i32, String) =
        sqlx::query_as("SELECT occupied_slots, status FROM foster_resource_cycle WHERE id = ?")
            .bind(cycle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(cycle.0, 0);
    assert_eq!(cycle.1, "AVAILABLE");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn confirmed_allocation_expires_and_frees_slot(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
    let job = seed_platform_job(&pool, "REAP", 1008).await?;
    let (provider_id, cycle_id) =
        seed_provider_cycle(&pool, "REAP", now, now + chrono::Duration::hours(8), 1, 0).await?;
    bind_friend(&pool, job.account_id, provider_id, "VERIFIED").await?;

    let service = ResourcePoolService::new(pool.clone());
    service.reserve_for_job(job.job_id, now).await?;
    assert!(service.confirm_for_job(job.job_id, now, Some(60)).await?);

    let report = service.reap(now + chrono::Duration::seconds(61)).await?;
    assert_eq!(report.expired_allocations, 1);

    let allocation_status: String =
        sqlx::query_scalar("SELECT status FROM foster_resource_allocation WHERE job_id = ?")
            .bind(job.job_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(allocation_status, "EXPIRED");

    let cycle: (i32, String) =
        sqlx::query_as("SELECT occupied_slots, status FROM foster_resource_cycle WHERE id = ?")
            .bind(cycle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(cycle.0, 0);
    assert_eq!(cycle.1, "AVAILABLE");

    Ok(())
}
