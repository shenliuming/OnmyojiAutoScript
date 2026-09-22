use std::time::Duration;

use foster_protocol::{
    AgentEvent, LoginFailed, LoginIdentityDetected, LoginPreparing, LoginQrExpired, LoginQrReady,
};
use foster_server::enrollment::EnrollmentService;
use sqlx::MySqlPool;

async fn seed_host(pool: &MySqlPool) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('host-events', 'host-events', 'ONLINE')",
    )
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn seed_emulator(pool: &MySqlPool, host_id: i64) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status
         )
         VALUES (?, 'emu-events', 'FAKE', 5, 'IDLE')",
    )
    .bind(host_id)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn seed_account(pool: &MySqlPool) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status
         )
         VALUES (7001, 'PENDING', 'PENDING')",
    )
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn create_session(pool: &MySqlPool) -> anyhow::Result<(EnrollmentService, i64, String)> {
    let host_id = seed_host(pool).await?;
    seed_emulator(pool, host_id).await?;
    let account_id = seed_account(pool).await?;

    let service = EnrollmentService::new(pool.clone());
    let created = service
        .create_login_session(account_id, Duration::from_secs(900))
        .await?;

    Ok((service, host_id, created.session_no))
}

#[sqlx::test(migrations = "../../migrations")]
async fn valid_qr_event_persists_qr_and_expiry(pool: MySqlPool) -> anyhow::Result<()> {
    let (service, host_id, session_no) = create_session(&pool).await?;

    service
        .process_agent_event(
            host_id,
            &AgentEvent::LoginPreparing(LoginPreparing {
                session_no: session_no.clone(),
            }),
        )
        .await?;

    let expires_at = chrono::Utc::now() + chrono::Duration::minutes(2);
    service
        .process_agent_event(
            host_id,
            &AgentEvent::LoginQrReady(LoginQrReady {
                session_no: session_no.clone(),
                qr_payload: "qr-v1".into(),
                expires_at,
            }),
        )
        .await?;

    let row: (String, Option<String>, Option<chrono::NaiveDateTime>) = sqlx::query_as(
        "SELECT status, qr_payload, qr_expires_at
         FROM login_session
         WHERE session_no = ?",
    )
    .bind(&session_no)
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.0, "QR_READY");
    assert_eq!(row.1.as_deref(), Some("qr-v1"));
    assert!(row.2.is_some());

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn refreshed_qr_replaces_previous_qr(pool: MySqlPool) -> anyhow::Result<()> {
    let (service, host_id, session_no) = create_session(&pool).await?;

    service
        .process_agent_event(
            host_id,
            &AgentEvent::LoginPreparing(LoginPreparing {
                session_no: session_no.clone(),
            }),
        )
        .await?;

    service
        .process_agent_event(
            host_id,
            &AgentEvent::LoginQrReady(LoginQrReady {
                session_no: session_no.clone(),
                qr_payload: "qr-v1".into(),
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(1),
            }),
        )
        .await?;

    service
        .process_agent_event(
            host_id,
            &AgentEvent::LoginQrExpired(LoginQrExpired {
                session_no: session_no.clone(),
            }),
        )
        .await?;

    service
        .process_agent_event(
            host_id,
            &AgentEvent::LoginQrReady(LoginQrReady {
                session_no: session_no.clone(),
                qr_payload: "qr-v2".into(),
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(2),
            }),
        )
        .await?;

    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT status, qr_payload
         FROM login_session
         WHERE session_no = ?",
    )
    .bind(&session_no)
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.0, "QR_READY");
    assert_eq!(row.1.as_deref(), Some("qr-v2"));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn wrong_host_cannot_mutate_login_session(pool: MySqlPool) -> anyhow::Result<()> {
    let (service, host_id, session_no) = create_session(&pool).await?;

    service
        .process_agent_event(
            host_id + 999,
            &AgentEvent::LoginPreparing(LoginPreparing {
                session_no: session_no.clone(),
            }),
        )
        .await?;

    let status: String = sqlx::query_scalar(
        "SELECT status
         FROM login_session
         WHERE session_no = ?",
    )
    .bind(&session_no)
    .fetch_one(&pool)
    .await?;

    assert_eq!(status, "CREATED");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn terminal_session_ignores_late_qr_event(pool: MySqlPool) -> anyhow::Result<()> {
    let (service, host_id, session_no) = create_session(&pool).await?;

    sqlx::query(
        "UPDATE login_session
         SET status = 'SUCCESS'
         WHERE session_no = ?",
    )
    .bind(&session_no)
    .execute(&pool)
    .await?;

    service
        .process_agent_event(
            host_id,
            &AgentEvent::LoginQrReady(LoginQrReady {
                session_no: session_no.clone(),
                qr_payload: "late-qr".into(),
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(2),
            }),
        )
        .await?;

    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT status, qr_payload
         FROM login_session
         WHERE session_no = ?",
    )
    .bind(&session_no)
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.0, "SUCCESS");
    assert!(row.1.is_none());

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn identity_event_moves_session_to_verifying_account(pool: MySqlPool) -> anyhow::Result<()> {
    let (service, host_id, session_no) = create_session(&pool).await?;

    service
        .process_agent_event(
            host_id,
            &AgentEvent::LoginPreparing(LoginPreparing {
                session_no: session_no.clone(),
            }),
        )
        .await?;

    service
        .process_agent_event(
            host_id,
            &AgentEvent::LoginQrReady(LoginQrReady {
                session_no: session_no.clone(),
                qr_payload: "qr".into(),
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(2),
            }),
        )
        .await?;

    service
        .process_agent_event(
            host_id,
            &AgentEvent::LoginIdentityDetected(LoginIdentityDetected {
                session_no: session_no.clone(),
                masked_account: Some("138****5678".into()),
                character_name: Some("角色A".into()),
                server_name: Some("春之樱".into()),
                game_uid: Some("10001".into()),
            }),
        )
        .await?;

    let row: (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT
            status,
            detected_masked_account,
            detected_character_name,
            detected_server_name,
            detected_game_uid
         FROM login_session
         WHERE session_no = ?",
    )
    .bind(&session_no)
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.0, "VERIFYING_ACCOUNT");
    assert_eq!(row.1.as_deref(), Some("138****5678"));
    assert_eq!(row.2.as_deref(), Some("角色A"));
    assert_eq!(row.3.as_deref(), Some("春之樱"));
    assert_eq!(row.4.as_deref(), Some("10001"));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_failed_releases_pending_binding(pool: MySqlPool) -> anyhow::Result<()> {
    let (service, host_id, session_no) = create_session(&pool).await?;

    let binding_id: i64 = sqlx::query_scalar(
        "SELECT binding_id
         FROM login_session
         WHERE session_no = ?",
    )
    .bind(&session_no)
    .fetch_one(&pool)
    .await?;

    service
        .process_agent_event(
            host_id,
            &AgentEvent::LoginFailed(LoginFailed {
                session_no: session_no.clone(),
                code: "QR_LOGIN_FAILED".into(),
                message: "login rejected".into(),
            }),
        )
        .await?;

    let session_status: String = sqlx::query_scalar(
        "SELECT status
         FROM login_session
         WHERE session_no = ?",
    )
    .bind(&session_no)
    .fetch_one(&pool)
    .await?;

    let binding_status: String = sqlx::query_scalar(
        "SELECT status
         FROM emulator_account_binding
         WHERE id = ?",
    )
    .bind(binding_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(session_status, "FAILED");
    assert_eq!(binding_status, "UNBOUND");

    Ok(())
}
