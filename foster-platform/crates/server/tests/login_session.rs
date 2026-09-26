use foster_domain::LoginSessionStatus;
use sqlx::MySqlPool;

async fn seed_host(pool: &MySqlPool) -> sqlx::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('host-login', 'host-login', 'ONLINE')",
    )
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn seed_emulator(pool: &MySqlPool, host_id: i64) -> sqlx::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status, lifecycle_status
         )
         VALUES (?, 'emu-login', 'FAKE', 5, 'IDLE', 'READY')",
    )
    .bind(host_id)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn seed_account(pool: &MySqlPool) -> sqlx::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status
         )
         VALUES (9001, 'PENDING', 'PENDING')",
    )
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn seed_binding(pool: &MySqlPool, emulator_id: i64, account_id: i64) -> sqlx::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO emulator_account_binding(
            emulator_id, game_account_id, slot_no, status
         )
         VALUES (?, ?, 1, 'PENDING')",
    )
    .bind(emulator_id)
    .bind(account_id)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn insert_session(
    pool: &MySqlPool,
    session_no: &str,
    account_id: i64,
    binding_id: i64,
    emulator_id: i64,
    public_token_hash: &str,
    control_token_hash: &str,
) -> sqlx::Result<i64> {
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
         VALUES (?, ?, ?, ?, 'CREATED', ?, ?, NOW(3) + INTERVAL 15 MINUTE)",
    )
    .bind(session_no)
    .bind(account_id)
    .bind(binding_id)
    .bind(emulator_id)
    .bind(public_token_hash)
    .bind(control_token_hash)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

#[test]
fn login_session_status_uses_stable_uppercase_names() {
    assert_eq!(
        serde_json::to_string(&LoginSessionStatus::WaitingQr).unwrap(),
        "\"WAITING_QR\""
    );
    assert_eq!(
        serde_json::to_string(&LoginSessionStatus::VerifyingAccount).unwrap(),
        "\"VERIFYING_ACCOUNT\""
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_session_enforces_unique_session_and_tokens(pool: MySqlPool) -> sqlx::Result<()> {
    let host_id = seed_host(&pool).await?;
    let emulator_id = seed_emulator(&pool, host_id).await?;
    let account_id = seed_account(&pool).await?;
    let binding_id = seed_binding(&pool, emulator_id, account_id).await?;

    let public_hash = "a".repeat(64);
    let control_hash = "b".repeat(64);

    insert_session(
        &pool,
        "LOGIN-001",
        account_id,
        binding_id,
        emulator_id,
        &public_hash,
        &control_hash,
    )
    .await?;

    let duplicate_session = insert_session(
        &pool,
        "LOGIN-001",
        account_id,
        binding_id,
        emulator_id,
        &"c".repeat(64),
        &"d".repeat(64),
    )
    .await;
    assert!(duplicate_session.is_err());

    let duplicate_public = insert_session(
        &pool,
        "LOGIN-002",
        account_id,
        binding_id,
        emulator_id,
        &public_hash,
        &"e".repeat(64),
    )
    .await;
    assert!(duplicate_public.is_err());

    let duplicate_control = insert_session(
        &pool,
        "LOGIN-003",
        account_id,
        binding_id,
        emulator_id,
        &"f".repeat(64),
        &control_hash,
    )
    .await;
    assert!(duplicate_control.is_err());

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_session_rejects_unknown_binding(pool: MySqlPool) -> sqlx::Result<()> {
    let host_id = seed_host(&pool).await?;
    let emulator_id = seed_emulator(&pool, host_id).await?;
    let account_id = seed_account(&pool).await?;

    let result = insert_session(
        &pool,
        "LOGIN-BAD-BINDING",
        account_id,
        999_999,
        emulator_id,
        &"1".repeat(64),
        &"2".repeat(64),
    )
    .await;

    assert!(result.is_err());
    Ok(())
}
