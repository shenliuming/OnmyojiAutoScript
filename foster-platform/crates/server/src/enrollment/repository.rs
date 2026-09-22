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
