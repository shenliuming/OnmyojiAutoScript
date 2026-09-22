use chrono::{DateTime, Utc};
use sqlx::MySqlPool;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProviderRow {
    pub id: i64,
    pub provider_code: String,
    pub game_uid: Option<String>,
    pub nickname: String,
    pub provider_alias: String,
    pub server_name: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ResourceCycleRow {
    pub id: i64,
    pub provider_account_id: i64,
    pub resource_type: String,
    pub resource_level: i32,
    pub start_at: chrono::NaiveDateTime,
    pub end_at: chrono::NaiveDateTime,
    pub slot_capacity: i32,
    pub occupied_slots: i32,
    pub status: String,
    pub verified_binding_count: i64,
}

pub async fn provider_exists(pool: &MySqlPool, provider_id: i64) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_account WHERE id = ?")
        .bind(provider_id)
        .fetch_one(pool)
        .await?;
    Ok(count == 1)
}

pub async fn resource_cycle_exists(pool: &MySqlPool, cycle_id: i64) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM foster_resource_cycle WHERE id = ?")
        .bind(cycle_id)
        .fetch_one(pool)
        .await?;
    Ok(count == 1)
}

pub async fn game_account_exists(
    pool: &MySqlPool,
    game_account_id: i64,
) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM game_account WHERE id = ?")
        .bind(game_account_id)
        .fetch_one(pool)
        .await?;
    Ok(count == 1)
}

pub async fn upsert_provider(
    pool: &MySqlPool,
    provider_code: &str,
    game_uid: Option<&str>,
    nickname: &str,
    provider_alias: &str,
    server_name: Option<&str>,
    status: &str,
) -> Result<i64, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let existing_id: Option<i64> = sqlx::query_scalar(
        "SELECT id
         FROM provider_account
         WHERE provider_code = ?
         FOR UPDATE",
    )
    .bind(provider_code)
    .fetch_optional(&mut *tx)
    .await?;

    let id = if let Some(id) = existing_id {
        sqlx::query(
            "UPDATE provider_account
             SET game_uid = ?,
                 nickname = ?,
                 provider_alias = ?,
                 server_name = ?,
                 status = ?
             WHERE id = ?",
        )
        .bind(game_uid)
        .bind(nickname)
        .bind(provider_alias)
        .bind(server_name)
        .bind(status)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        id
    } else {
        sqlx::query(
            "INSERT INTO provider_account(
                provider_code, game_uid, nickname,
                provider_alias, server_name, status
             )
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(provider_code)
        .bind(game_uid)
        .bind(nickname)
        .bind(provider_alias)
        .bind(server_name)
        .bind(status)
        .execute(&mut *tx)
        .await?
        .last_insert_id() as i64
    };

    tx.commit().await?;
    Ok(id)
}

pub async fn update_provider_status(
    pool: &MySqlPool,
    provider_id: i64,
    status: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE provider_account
         SET status = ?
         WHERE id = ?",
    )
    .bind(status)
    .bind(provider_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() == 1)
}

pub async fn upsert_friend_binding(
    pool: &MySqlPool,
    game_account_id: i64,
    provider_id: i64,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO foster_friend_binding(
            game_account_id, provider_account_id, status, verified_at
         )
         VALUES (
            ?, ?, ?,
            CASE WHEN ? = 'VERIFIED' THEN NOW(3) ELSE NULL END
         )
         ON DUPLICATE KEY UPDATE
            status = VALUES(status),
            verified_at = CASE
                WHEN VALUES(status) = 'VERIFIED' THEN NOW(3)
                ELSE verified_at
            END,
            last_failure_at = CASE
                WHEN VALUES(status) = 'VERIFIED' THEN NULL
                ELSE last_failure_at
            END,
            last_failure_code = CASE
                WHEN VALUES(status) = 'VERIFIED' THEN NULL
                ELSE last_failure_code
            END",
    )
    .bind(game_account_id)
    .bind(provider_id)
    .bind(status)
    .bind(status)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn insert_resource_cycle(
    pool: &MySqlPool,
    provider_id: i64,
    resource_type: &str,
    resource_level: i32,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    slot_capacity: i32,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO foster_resource_cycle(
            provider_account_id, resource_type, resource_level,
            start_at, end_at, slot_capacity, occupied_slots, status
         )
         VALUES (?, ?, ?, ?, ?, ?, 0, 'AVAILABLE')",
    )
    .bind(provider_id)
    .bind(resource_type)
    .bind(resource_level)
    .bind(start_at.naive_utc())
    .bind(end_at.naive_utc())
    .bind(slot_capacity)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

pub async fn update_cycle_status(
    pool: &MySqlPool,
    cycle_id: i64,
    status: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE foster_resource_cycle
         SET status = ?
         WHERE id = ?",
    )
    .bind(status)
    .bind(cycle_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() == 1)
}

pub async fn list_providers(pool: &MySqlPool) -> Result<Vec<ProviderRow>, sqlx::Error> {
    sqlx::query_as::<_, ProviderRow>(
        "SELECT
            id, provider_code, game_uid, nickname,
            provider_alias, server_name, status
         FROM provider_account
         ORDER BY id ASC",
    )
    .fetch_all(pool)
    .await
}

pub async fn list_cycles(pool: &MySqlPool) -> Result<Vec<ResourceCycleRow>, sqlx::Error> {
    sqlx::query_as::<_, ResourceCycleRow>(
        "SELECT
            c.id,
            c.provider_account_id,
            c.resource_type,
            c.resource_level,
            c.start_at,
            c.end_at,
            c.slot_capacity,
            c.occupied_slots,
            c.status,
            (
                SELECT COUNT(*)
                FROM foster_friend_binding b
                WHERE b.provider_account_id = c.provider_account_id
                  AND b.status = 'VERIFIED'
            ) AS verified_binding_count
         FROM foster_resource_cycle c
         ORDER BY c.provider_account_id ASC, c.end_at ASC, c.id ASC",
    )
    .fetch_all(pool)
    .await
}
