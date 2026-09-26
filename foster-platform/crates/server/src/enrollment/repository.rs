use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{MySql, MySqlPool, Transaction};

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
    let mut tx = pool.begin().await?;

    let session: Option<(i64, i64, String)> = sqlx::query_as(
        "SELECT ls.id, ls.binding_id, ls.status
         FROM login_session ls
         JOIN emulator_instance e ON e.id = ls.emulator_id
         WHERE ls.session_no = ?
           AND e.host_id = ?
         FOR UPDATE",
    )
    .bind(session_no)
    .bind(host_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((session_id, binding_id, status)) = session else {
        tx.commit().await?;
        return Ok(());
    };

    if matches!(status.as_str(), "SUCCESS" | "FAILED" | "CANCELLED") {
        tx.commit().await?;
        return Ok(());
    }

    let updated = sqlx::query(
        "UPDATE login_session
         SET status = 'FAILED',
             failed_reason = ?,
             qr_payload = NULL,
             qr_expires_at = NULL
         WHERE id = ?
           AND status NOT IN ('SUCCESS', 'FAILED', 'CANCELLED')",
    )
    .bind(reason)
    .bind(session_id)
    .execute(&mut *tx)
    .await?;

    if updated.rows_affected() == 1 {
        release_pending_binding_tx(&mut tx, binding_id).await?;
    }

    tx.commit().await?;
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
pub struct LockedLoginSession {
    pub id: i64,
    pub session_no: String,
    pub game_account_id: i64,
    pub binding_id: i64,
    pub emulator_id: i64,
    pub status: String,
    pub expires_at: NaiveDateTime,
    pub detected_masked_account: Option<String>,
    pub detected_character_name: Option<String>,
    pub detected_server_name: Option<String>,
    pub detected_game_uid: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct LockedBinding {
    pub id: i64,
    pub emulator_id: i64,
    pub game_account_id: i64,
    pub status: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct LockedGameAccount {
    pub id: i64,
    pub active_emulator_id: Option<i64>,
    pub platform: Option<String>,
    pub character_name: Option<String>,
    pub game_uid: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct TrustedIdentityRow {
    pub identity_type: String,
    pub identity_value: String,
    pub normalized_value: String,
    pub confidence: i32,
}

pub async fn lock_login_session_by_control_hash(
    tx: &mut Transaction<'_, MySql>,
    control_token_hash: &str,
) -> Result<Option<LockedLoginSession>, sqlx::Error> {
    sqlx::query_as::<_, LockedLoginSession>(
        "SELECT
            id,
            session_no,
            game_account_id,
            binding_id,
            emulator_id,
            status,
            expires_at,
            detected_masked_account,
            detected_character_name,
            detected_server_name,
            detected_game_uid
         FROM login_session
         WHERE control_token_hash = ?
         FOR UPDATE",
    )
    .bind(control_token_hash)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn lock_binding(
    tx: &mut Transaction<'_, MySql>,
    binding_id: i64,
) -> Result<Option<LockedBinding>, sqlx::Error> {
    sqlx::query_as::<_, LockedBinding>(
        "SELECT id, emulator_id, game_account_id, status
         FROM emulator_account_binding
         WHERE id = ?
         FOR UPDATE",
    )
    .bind(binding_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn lock_game_account(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
) -> Result<Option<LockedGameAccount>, sqlx::Error> {
    sqlx::query_as::<_, LockedGameAccount>(
        "SELECT id, active_emulator_id, platform, character_name, game_uid
         FROM game_account
         WHERE id = ?
         FOR UPDATE",
    )
    .bind(game_account_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn load_trusted_identities(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
) -> Result<Vec<TrustedIdentityRow>, sqlx::Error> {
    sqlx::query_as::<_, TrustedIdentityRow>(
        "SELECT
            identity_type,
            identity_value,
            normalized_value,
            confidence
         FROM game_account_identity
         WHERE game_account_id = ?
           AND enabled = 1
           AND identity_type IN (
               'MASKED_ACCOUNT',
               'OCR_ALIAS',
               'CHARACTER_NAME',
               'SERVER_NAME',
               'GAME_UID'
           )
         FOR UPDATE",
    )
    .bind(game_account_id)
    .fetch_all(&mut **tx)
    .await
}

pub async fn insert_enrollment_identity(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
    identity_type: &str,
    identity_value: &str,
    normalized_value: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO game_account_identity(
            game_account_id,
            identity_type,
            identity_value,
            normalized_value,
            source,
            confidence,
            enabled,
            last_seen_at
         )
         VALUES (?, ?, ?, ?, 'LOGIN_ENROLLMENT', 100, 1, NOW(3))",
    )
    .bind(game_account_id)
    .bind(identity_type)
    .bind(identity_value)
    .bind(normalized_value)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn activate_pending_binding(
    tx: &mut Transaction<'_, MySql>,
    binding_id: i64,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE emulator_account_binding
         SET status = 'ACTIVE',
             bound_at = NOW(3),
             unbound_at = NULL
         WHERE id = ?
           AND status = 'PENDING'",
    )
    .bind(binding_id)
    .execute(&mut **tx)
    .await?;

    Ok(result.rows_affected() == 1)
}

pub async fn update_game_account_login_target(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
    platform: &str,
    character_name: &str,
    game_uid: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE game_account
         SET platform = ?,
             character_name = ?,
             game_uid = ?
         WHERE id = ?",
    )
    .bind(platform)
    .bind(character_name)
    .bind(game_uid)
    .bind(game_account_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn activate_game_account(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
    emulator_id: i64,
    character_name: Option<&str>,
    server_name: Option<&str>,
    game_uid: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE game_account
         SET active_emulator_id = ?,
             login_status = 'LOGGED_IN',
             verify_status = 'VERIFIED',
             last_verified_at = NOW(3),
             character_name = COALESCE(?, character_name),
             server_name = COALESCE(?, server_name),
             game_uid = COALESCE(?, game_uid)
         WHERE id = ?",
    )
    .bind(emulator_id)
    .bind(character_name)
    .bind(server_name)
    .bind(game_uid)
    .bind(game_account_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn complete_login_session(
    tx: &mut Transaction<'_, MySql>,
    session_id: i64,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE login_session
         SET status = 'SUCCESS',
             confirmed_at = NOW(3),
             qr_payload = NULL,
             qr_expires_at = NULL
         WHERE id = ?
           AND status = 'VERIFYING_ACCOUNT'",
    )
    .bind(session_id)
    .execute(&mut **tx)
    .await?;

    Ok(result.rows_affected() == 1)
}

pub async fn find_expired_login_session_ids(pool: &MySqlPool) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id
         FROM login_session
         WHERE expires_at <= NOW(3)
           AND status NOT IN ('SUCCESS', 'FAILED', 'CANCELLED')
         ORDER BY id ASC",
    )
    .fetch_all(pool)
    .await
}

pub async fn lock_login_session_by_id(
    tx: &mut Transaction<'_, MySql>,
    session_id: i64,
) -> Result<Option<LockedLoginSession>, sqlx::Error> {
    sqlx::query_as::<_, LockedLoginSession>(
        "SELECT
            id,
            session_no,
            game_account_id,
            binding_id,
            emulator_id,
            status,
            expires_at,
            detected_masked_account,
            detected_character_name,
            detected_server_name,
            detected_game_uid
         FROM login_session
         WHERE id = ?
         FOR UPDATE",
    )
    .bind(session_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn cancel_login_session_row(
    tx: &mut Transaction<'_, MySql>,
    session_id: i64,
    reason: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE login_session
         SET status = 'CANCELLED',
             failed_reason = ?,
             qr_payload = NULL,
             qr_expires_at = NULL
         WHERE id = ?
           AND status NOT IN ('SUCCESS', 'FAILED', 'CANCELLED')",
    )
    .bind(reason)
    .bind(session_id)
    .execute(&mut **tx)
    .await?;

    Ok(result.rows_affected() == 1)
}

pub async fn release_pending_binding_tx(
    tx: &mut Transaction<'_, MySql>,
    binding_id: i64,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE emulator_account_binding
         SET status = 'UNBOUND',
             unbound_at = NOW(3)
         WHERE id = ?
           AND status = 'PENDING'",
    )
    .bind(binding_id)
    .execute(&mut **tx)
    .await?;

    Ok(result.rows_affected() == 1)
}

#[derive(Debug, sqlx::FromRow)]
pub struct LoginDispatchTarget {
    pub id: i64,
    pub session_no: String,
    pub game_account_id: i64,
    pub status: String,
    pub host_id: i64,
    pub emulator_code: String,
    pub platform: Option<String>,
    pub character_name: Option<String>,
    pub game_uid: Option<String>,
}

pub async fn lock_login_dispatch_target(
    tx: &mut Transaction<'_, MySql>,
    session_no: &str,
) -> Result<Option<LoginDispatchTarget>, sqlx::Error> {
    sqlx::query_as::<_, LoginDispatchTarget>(
        "SELECT
            ls.id,
            ls.session_no,
            ls.game_account_id,
            ls.status,
            e.host_id,
            e.emulator_code,
            a.platform,
            a.character_name,
            a.game_uid
         FROM login_session ls
         JOIN emulator_instance e ON e.id = ls.emulator_id
         JOIN game_account a ON a.id = ls.game_account_id
         WHERE ls.session_no = ?
         FOR UPDATE",
    )
    .bind(session_no)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn mark_login_waiting_emulator(
    tx: &mut Transaction<'_, MySql>,
    session_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE login_session
         SET status = 'WAITING_EMULATOR'
         WHERE id = ?
           AND status IN ('CREATED', 'WAITING_EMULATOR')",
    )
    .bind(session_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn mark_login_preparing_after_dispatch(
    tx: &mut Transaction<'_, MySql>,
    session_id: i64,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE login_session
         SET status = 'PREPARING'
         WHERE id = ?
           AND status IN ('CREATED', 'WAITING_EMULATOR')",
    )
    .bind(session_id)
    .execute(&mut **tx)
    .await?;

    Ok(result.rows_affected() == 1)
}

pub async fn activate_pending_subscriptions(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE foster_subscription
         SET status = 'ACTIVE',
             next_run_at = COALESCE(next_run_at, NOW(3))
         WHERE game_account_id = ?
           AND status = 'PENDING_LOGIN'
           AND end_at > NOW(3)",
    )
    .bind(game_account_id)
    .execute(&mut **tx)
    .await?;

    Ok(result.rows_affected())
}
