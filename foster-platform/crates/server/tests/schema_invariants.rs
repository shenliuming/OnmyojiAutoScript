use sqlx::MySqlPool;

async fn insert_host(pool: &MySqlPool, code: &str) -> sqlx::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES (?, ?, 'OFFLINE')",
    )
    .bind(code)
    .bind(code)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn insert_emulator(
    pool: &MySqlPool,
    host_id: i64,
    code: &str,
    max_account_count: i32,
) -> sqlx::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status
         )
         VALUES (?, ?, 'UNKNOWN', ?, 'IDLE')",
    )
    .bind(host_id)
    .bind(code)
    .bind(max_account_count)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn insert_account(pool: &MySqlPool, customer_id: i64) -> sqlx::Result<i64> {
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

async fn insert_binding(
    pool: &MySqlPool,
    emulator_id: i64,
    game_account_id: i64,
    slot_no: i32,
    status: &str,
) -> sqlx::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO emulator_account_binding(
            emulator_id, game_account_id, slot_no, status
         )
         VALUES (?, ?, ?, ?)",
    )
    .bind(emulator_id)
    .bind(game_account_id)
    .bind(slot_no)
    .bind(status)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

#[sqlx::test(migrations = "../../migrations")]
async fn database_rejects_two_active_bindings_for_same_account(
    pool: MySqlPool,
) -> sqlx::Result<()> {
    let host_id = insert_host(&pool, "host-a").await?;
    let emu_a = insert_emulator(&pool, host_id, "emu-a", 5).await?;
    let emu_b = insert_emulator(&pool, host_id, "emu-b", 5).await?;
    let account_id = insert_account(&pool, 1001).await?;

    insert_binding(&pool, emu_a, account_id, 1, "ACTIVE").await?;

    let second = insert_binding(&pool, emu_b, account_id, 1, "ACTIVE").await;
    assert!(second.is_err());

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn database_rejects_two_occupied_bindings_for_same_slot(
    pool: MySqlPool,
) -> sqlx::Result<()> {
    let host_id = insert_host(&pool, "host-a").await?;
    let emulator_id = insert_emulator(&pool, host_id, "emu-a", 5).await?;
    let account_a = insert_account(&pool, 1001).await?;
    let account_b = insert_account(&pool, 1002).await?;

    insert_binding(&pool, emulator_id, account_a, 1, "PENDING").await?;

    let second = insert_binding(&pool, emulator_id, account_b, 1, "PENDING").await;
    assert!(second.is_err());

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn unbound_history_does_not_block_slot_reuse(
    pool: MySqlPool,
) -> sqlx::Result<()> {
    let host_id = insert_host(&pool, "host-a").await?;
    let emulator_id = insert_emulator(&pool, host_id, "emu-a", 5).await?;
    let account_a = insert_account(&pool, 1001).await?;
    let account_b = insert_account(&pool, 1002).await?;

    insert_binding(&pool, emulator_id, account_a, 1, "UNBOUND").await?;
    insert_binding(&pool, emulator_id, account_b, 1, "PENDING").await?;

    Ok(())
}
