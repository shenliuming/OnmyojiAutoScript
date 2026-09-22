use std::time::Duration;

use foster_server::enrollment::EnrollmentService;
use sqlx::MySqlPool;

async fn seed_host(pool: &MySqlPool, suffix: &str) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES (?, ?, 'ONLINE')",
    )
    .bind(format!("host-expiry-{suffix}"))
    .bind(format!("host-expiry-{suffix}"))
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn seed_emulator(
    pool: &MySqlPool,
    host_id: i64,
    suffix: &str,
    capacity: i32,
) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status
         )
         VALUES (?, ?, 'FAKE', ?, 'IDLE')",
    )
    .bind(host_id)
    .bind(format!("emu-expiry-{suffix}"))
    .bind(capacity)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn seed_account(pool: &MySqlPool, customer_id: i64) -> anyhow::Result<i64> {
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
async fn session_expiry_releases_pending_capacity(pool: MySqlPool) -> anyhow::Result<()> {
    let host_id = seed_host(&pool, "release").await?;
    seed_emulator(&pool, host_id, "release", 1).await?;
    let first_account = seed_account(&pool, 8101).await?;
    let second_account = seed_account(&pool, 8102).await?;

    let service = EnrollmentService::new(pool.clone());
    let first = service
        .create_login_session(first_account, Duration::from_secs(900))
        .await?;

    sqlx::query(
        "UPDATE login_session
         SET expires_at = DATE_SUB(NOW(3), INTERVAL 1 SECOND)
         WHERE session_no = ?",
    )
    .bind(&first.session_no)
    .execute(&pool)
    .await?;

    service.expire_login_sessions().await?;

    let row: (String, String) = sqlx::query_as(
        "SELECT ls.status, b.status
         FROM login_session ls
         JOIN emulator_account_binding b ON b.id = ls.binding_id
         WHERE ls.session_no = ?",
    )
    .bind(&first.session_no)
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.0, "CANCELLED");
    assert_eq!(row.1, "UNBOUND");

    let second = service
        .create_login_session(second_account, Duration::from_secs(900))
        .await?;
    assert!(second.binding_id > 0);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn qr_expiry_does_not_release_account_reservation(pool: MySqlPool) -> anyhow::Result<()> {
    let host_id = seed_host(&pool, "qr").await?;
    seed_emulator(&pool, host_id, "qr", 1).await?;
    let account_id = seed_account(&pool, 8201).await?;

    let service = EnrollmentService::new(pool.clone());
    let created = service
        .create_login_session(account_id, Duration::from_secs(900))
        .await?;

    sqlx::query(
        "UPDATE login_session
         SET status = 'QR_EXPIRED',
             qr_payload = NULL,
             qr_expires_at = NULL
         WHERE session_no = ?",
    )
    .bind(&created.session_no)
    .execute(&pool)
    .await?;

    service.expire_login_sessions().await?;

    let row: (String, String) = sqlx::query_as(
        "SELECT ls.status, b.status
         FROM login_session ls
         JOIN emulator_account_binding b ON b.id = ls.binding_id
         WHERE ls.session_no = ?",
    )
    .bind(&created.session_no)
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.0, "QR_EXPIRED");
    assert_eq!(row.1, "PENDING");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn explicit_cancel_releases_pending_reservation(pool: MySqlPool) -> anyhow::Result<()> {
    let host_id = seed_host(&pool, "cancel").await?;
    seed_emulator(&pool, host_id, "cancel", 1).await?;
    let account_id = seed_account(&pool, 8301).await?;

    let service = EnrollmentService::new(pool.clone());
    let created = service
        .create_login_session(account_id, Duration::from_secs(900))
        .await?;

    service.cancel_login_session(&created.control_token).await?;

    let row: (String, String) = sqlx::query_as(
        "SELECT ls.status, b.status
         FROM login_session ls
         JOIN emulator_account_binding b ON b.id = ls.binding_id
         WHERE ls.session_no = ?",
    )
    .bind(&created.session_no)
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.0, "CANCELLED");
    assert_eq!(row.1, "UNBOUND");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn stale_expiry_never_releases_active_binding(pool: MySqlPool) -> anyhow::Result<()> {
    let host_id = seed_host(&pool, "active").await?;
    let emulator_id = seed_emulator(&pool, host_id, "active", 1).await?;
    let account_id = seed_account(&pool, 8401).await?;

    let service = EnrollmentService::new(pool.clone());
    let created = service
        .create_login_session(account_id, Duration::from_secs(900))
        .await?;

    sqlx::query(
        "UPDATE emulator_account_binding
         SET status = 'ACTIVE',
             bound_at = NOW(3)
         WHERE id = ?",
    )
    .bind(created.binding_id)
    .execute(&pool)
    .await?;

    sqlx::query(
        "UPDATE game_account
         SET active_emulator_id = ?,
             login_status = 'LOGGED_IN',
             verify_status = 'VERIFIED'
         WHERE id = ?",
    )
    .bind(emulator_id)
    .bind(account_id)
    .execute(&pool)
    .await?;

    sqlx::query(
        "UPDATE login_session
         SET expires_at = DATE_SUB(NOW(3), INTERVAL 1 SECOND)
         WHERE session_no = ?",
    )
    .bind(&created.session_no)
    .execute(&pool)
    .await?;

    service.expire_login_sessions().await?;

    let binding_status: String = sqlx::query_scalar(
        "SELECT status
         FROM emulator_account_binding
         WHERE id = ?",
    )
    .bind(created.binding_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(binding_status, "ACTIVE");

    Ok(())
}
