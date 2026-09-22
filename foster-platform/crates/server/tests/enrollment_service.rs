use std::time::Duration;

use foster_server::{
    control_plane::AllocationError,
    enrollment::{EnrollmentError, EnrollmentService},
};
use sqlx::MySqlPool;

async fn seed_host(pool: &MySqlPool) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('host-enroll', 'host-enroll', 'ONLINE')",
    )
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn seed_emulator(
    pool: &MySqlPool,
    host_id: i64,
    capacity: i32,
) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status
         )
         VALUES (?, 'emu-enroll', 'FAKE', ?, 'IDLE')",
    )
    .bind(host_id)
    .bind(capacity)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn seed_account(
    pool: &MySqlPool,
    customer_id: i64,
) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status
         )
         VALUES (?, 'PENDING', 'PENDING')",
    )
    .bind(customer_id)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

#[sqlx::test(migrations = "../../migrations")]
async fn created_session_stores_only_token_hashes(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let host_id = seed_host(&pool).await?;
    seed_emulator(&pool, host_id, 2).await?;
    let account_id = seed_account(&pool, 1001).await?;

    let service = EnrollmentService::new(pool.clone());
    let created = service
        .create_login_session(account_id, Duration::from_secs(900))
        .await?;

    assert_ne!(created.public_token, created.control_token);

    let (public_hash, control_hash): (String, String) = sqlx::query_as(
        "SELECT public_token_hash, control_token_hash
         FROM login_session
         WHERE id = ?",
    )
    .bind(created.session_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(public_hash.len(), 64);
    assert_eq!(control_hash.len(), 64);
    assert_ne!(public_hash, created.public_token);
    assert_ne!(control_hash, created.control_token);

    let binding: (String, i64) = sqlx::query_as(
        "SELECT status, emulator_id
         FROM emulator_account_binding
         WHERE id = ?",
    )
    .bind(created.binding_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(binding.0, "PENDING");
    assert_eq!(binding.1, created.emulator_id);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn pending_login_session_consumes_emulator_capacity(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let host_id = seed_host(&pool).await?;
    seed_emulator(&pool, host_id, 1).await?;
    let account_a = seed_account(&pool, 1001).await?;
    let account_b = seed_account(&pool, 1002).await?;

    let service = EnrollmentService::new(pool.clone());
    service
        .create_login_session(account_a, Duration::from_secs(900))
        .await?;

    let second = service
        .create_login_session(account_b, Duration::from_secs(900))
        .await;

    assert!(matches!(
        second,
        Err(EnrollmentError::Allocation(AllocationError::NoCapacity))
    ));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn same_account_cannot_create_second_login_session(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let host_id = seed_host(&pool).await?;
    seed_emulator(&pool, host_id, 2).await?;
    let account_id = seed_account(&pool, 1001).await?;

    let service = EnrollmentService::new(pool.clone());
    service
        .create_login_session(account_id, Duration::from_secs(900))
        .await?;

    let second = service
        .create_login_session(account_id, Duration::from_secs(900))
        .await;

    assert!(matches!(
        second,
        Err(EnrollmentError::Allocation(AllocationError::AlreadyBound))
    ));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn failed_session_insert_releases_pending_binding(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let host_id = seed_host(&pool).await?;
    seed_emulator(&pool, host_id, 1).await?;
    let account_id = seed_account(&pool, 1001).await?;

    sqlx::query("DROP TABLE login_session").execute(&pool).await?;

    let service = EnrollmentService::new(pool.clone());
    let result = service
        .create_login_session(account_id, Duration::from_secs(900))
        .await;

    assert!(matches!(result, Err(EnrollmentError::Database(_))));

    let reserved: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM emulator_account_binding
         WHERE game_account_id = ?
           AND status IN ('PENDING', 'ACTIVE', 'MIGRATING')",
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(reserved, 0);

    Ok(())
}
