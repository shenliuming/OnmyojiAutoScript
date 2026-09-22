use chrono::{DateTime, Utc};
use sqlx::MySqlPool;

pub async fn insert_login_session(
    pool: &MySqlPool,
    session_no: &str,
    game_account_id: i64,
    binding_id: i64,
    emulator_id: i64,
    public_token_hash: &str,
    control_token_hash: &str,
    expires_at: DateTime<Utc>,
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
    .bind(session_no)
    .bind(game_account_id)
    .bind(binding_id)
    .bind(emulator_id)
    .bind(public_token_hash)
    .bind(control_token_hash)
    .bind(expires_at.naive_utc())
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

pub async fn release_pending_binding(
    pool: &MySqlPool,
    binding_id: i64,
) -> Result<(), sqlx::Error> {
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
