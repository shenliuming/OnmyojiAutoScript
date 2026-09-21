use sqlx::{MySql, Transaction};

#[derive(Debug, sqlx::FromRow)]
pub struct GameAccountLockRow {
    pub id: i64,
    pub active_emulator_id: Option<i64>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct EmulatorCapacityRow {
    pub id: i64,
    pub max_account_count: i32,
}

pub async fn lock_game_account(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
) -> Result<Option<GameAccountLockRow>, sqlx::Error> {
    sqlx::query_as::<_, GameAccountLockRow>(
        "SELECT id, active_emulator_id
         FROM game_account
         WHERE id = ?
         FOR UPDATE",
    )
    .bind(game_account_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn has_reserved_binding(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM emulator_account_binding
         WHERE game_account_id = ?
           AND status IN ('PENDING', 'ACTIVE', 'MIGRATING')",
    )
    .bind(game_account_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(count > 0)
}

pub async fn candidate_emulator_ids(
    tx: &mut Transaction<'_, MySql>,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id
         FROM emulator_instance
         WHERE status = 'IDLE'
         ORDER BY id ASC",
    )
    .fetch_all(&mut **tx)
    .await
}

pub async fn lock_emulator(
    tx: &mut Transaction<'_, MySql>,
    emulator_id: i64,
) -> Result<Option<EmulatorCapacityRow>, sqlx::Error> {
    sqlx::query_as::<_, EmulatorCapacityRow>(
        "SELECT id, max_account_count
         FROM emulator_instance
         WHERE id = ?
           AND status = 'IDLE'
         FOR UPDATE SKIP LOCKED",
    )
    .bind(emulator_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn occupied_slots(
    tx: &mut Transaction<'_, MySql>,
    emulator_id: i64,
) -> Result<Vec<i32>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT slot_no
         FROM emulator_account_binding
         WHERE emulator_id = ?
           AND status IN ('PENDING', 'ACTIVE', 'MIGRATING')
         ORDER BY slot_no ASC
         FOR UPDATE",
    )
    .bind(emulator_id)
    .fetch_all(&mut **tx)
    .await
}

pub async fn insert_pending_binding(
    tx: &mut Transaction<'_, MySql>,
    emulator_id: i64,
    game_account_id: i64,
    slot_no: i32,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO emulator_account_binding(
            emulator_id, game_account_id, slot_no, status
         )
         VALUES (?, ?, ?, 'PENDING')",
    )
    .bind(emulator_id)
    .bind(game_account_id)
    .bind(slot_no)
    .execute(&mut **tx)
    .await?;

    Ok(result.last_insert_id() as i64)
}
