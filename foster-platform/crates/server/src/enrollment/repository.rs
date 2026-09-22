use chrono::{DateTime, Utc};
use sqlx::MySqlPool;

pub struct NewLoginSession<'a> {
    pub session_no: &'a str,
    pub game_account_id: i64,
    pub binding_id: i64,
    pub emulator_id: i64,
    pub public_token_hash: &'a str,
    pub control_token_hash: &'a str,
    pub expires_at: DateTime<Utc>,
}

pub async fn insert_login_session(
    pool: &MySqlPool,
    session: NewLoginSession<'_>,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO login_session(
            session_no,
            game_account_id,
            binding_id,
            emulator_id,
            status,
            public_token_hash,
            control_token_hash,
            expires_at
         )
         VALUES (?, ?, ?, ?, 'CREATED', ?, ?, ?)",
    )
    .bind(session.session_no)
    .bind(session.game_account_id)
    .bind(session.binding_id)
    .bind(session.emulator_id)
    .bind(session.public_token_hash)
    .bind(session.control_token_hash)
    .bind(session.expires_at.naive_utc())
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

pub async fn release_pending_binding(pool: &MySqlPool, binding_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE emulator_account_binding
         SET status = 'UNBOUND',
             unbound_at = NOW(3)
         WHERE id = ?
           AND status = 'PENDING'",
    )
    .bind(binding_id)
    .execute(pool)
    .await?;

    Ok(())
}


pub async fn mark_login_preparing(
    pool: &MySqlPool,
    host_id: i64,
    session_no: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE login_session ls
         JOIN emulator_instance e ON e.id = ls.emulator_id
         SET ls.status = 'PREPARING'
         WHERE ls.session_no = ?
           AND e.host_id = ?
           AND ls.status NOT IN ('SUCCESS', 'FAILED', 'CANCELLED')",
    )
    .bind(session_no)
    .bind(host_id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn mark_login_qr_ready(
    pool: &MySqlPool,
    host_id: i64,
    session_no: &str,
    qr_payload: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE login_session ls
         JOIN emulator_instance e ON e.id = ls.emulator_id
         SET ls.status = 'QR_READY',
             ls.qr_payload = ?,
             ls.qr_expires_at = ?
         WHERE ls.session_no = ?
           AND e.host_id = ?
           AND ls.status NOT IN ('SUCCESS', 'FAILED', 'CANCELLED')",
    )
    .bind(qr_payload)
    .bind(expires_at.naive_utc())
    .bind(session_no)
    .bind(host_id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn mark_login_qr_expired(
    pool: &MySqlPool,
    host_id: i64,
    session_no: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE login_session ls
         JOIN emulator_instance e ON e.id = ls.emulator_id
         SET ls.status = 'QR_EXPIRED',
             ls.qr_payload = NULL,
             ls.qr_expires_at = NULL
         WHERE ls.session_no = ?
           AND e.host_id = ?
           AND ls.status NOT IN ('SUCCESS', 'FAILED', 'CANCELLED')",
    )
    .bind(session_no)
    .bind(host_id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn mark_login_identity_detected(
    pool: &MySqlPool,
    host_id: i64,
    session_no: &str,
    masked_account: Option<&str>,
    character_name: Option<&str>,
    server_name: Option<&str>,
    game_uid: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE login_session ls
         JOIN emulator_instance e ON e.id = ls.emulator_id
         SET ls.status = 'VERIFYING_ACCOUNT',
             ls.detected_masked_account = ?,
             ls.detected_character_name = ?,
             ls.detected_server_name = ?,
             ls.detected_game_uid = ?
         WHERE ls.session_no = ?
           AND e.host_id = ?
           AND ls.status NOT IN ('SUCCESS', 'FAILED', 'CANCELLED')",
    )
    .bind(masked_account)
    .bind(character_name)
    .bind(server_name)
    .bind(game_uid)
    .bind(session_no)
    .bind(host_id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn mark_login_failed(
    pool: &MySqlPool,
    host_id: i64,
    session_no: &str,
    reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE login_session ls
         JOIN emulator_instance e ON e.id = ls.emulator_id
         SET ls.status = 'FAILED',
             ls.failed_reason = ?
         WHERE ls.session_no = ?
           AND e.host_id = ?
           AND ls.status NOT IN ('SUCCESS', 'FAILED', 'CANCELLED')",
    )
    .bind(reason)
    .bind(session_no)
    .bind(host_id)
    .execute(pool)
    .await?;

    Ok(())
}
