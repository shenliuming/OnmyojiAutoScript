use foster_server::control_plane::{AllocationError, BindingAllocator};
use sqlx::MySqlPool;

async fn seed_host(pool: &MySqlPool) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('host-a', 'host-a', 'ONLINE')",
    )
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn seed_emulator(
    pool: &MySqlPool,
    host_id: i64,
    code: &str,
    capacity: i32,
) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status
         )
         VALUES (?, ?, 'UNKNOWN', ?, 'IDLE')",
    )
    .bind(host_id)
    .bind(code)
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
async fn pending_binding_consumes_capacity(pool: MySqlPool) -> anyhow::Result<()> {
    let host_id = seed_host(&pool).await?;
    seed_emulator(&pool, host_id, "emu-a", 1).await?;
    let account_a = seed_account(&pool, 1001).await?;
    let account_b = seed_account(&pool, 1002).await?;

    let allocator = BindingAllocator::new(pool.clone());

    let first = allocator.allocate_pending(account_a).await?;
    assert_eq!(first.slot_no, 1);

    let second = allocator.allocate_pending(account_b).await;
    assert!(matches!(second, Err(AllocationError::NoCapacity)));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn same_account_cannot_receive_second_reserved_binding(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let host_id = seed_host(&pool).await?;
    seed_emulator(&pool, host_id, "emu-a", 5).await?;
    seed_emulator(&pool, host_id, "emu-b", 5).await?;
    let account = seed_account(&pool, 1001).await?;

    let allocator = BindingAllocator::new(pool.clone());
    allocator.allocate_pending(account).await?;

    let second = allocator.allocate_pending(account).await;
    assert!(matches!(second, Err(AllocationError::AlreadyBound)));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn allocator_uses_lowest_free_slot(pool: MySqlPool) -> anyhow::Result<()> {
    let host_id = seed_host(&pool).await?;
    let emulator_id = seed_emulator(&pool, host_id, "emu-a", 3).await?;
    let existing = seed_account(&pool, 1001).await?;
    let newcomer = seed_account(&pool, 1002).await?;

    sqlx::query(
        "INSERT INTO emulator_account_binding(
            emulator_id, game_account_id, slot_no, status
         )
         VALUES (?, ?, 1, 'ACTIVE')",
    )
    .bind(emulator_id)
    .bind(existing)
    .execute(&pool)
    .await?;

    let allocator = BindingAllocator::new(pool.clone());
    let allocated = allocator.allocate_pending(newcomer).await?;

    assert_eq!(allocated.emulator_id, emulator_id);
    assert_eq!(allocated.slot_no, 2);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_allocations_do_not_oversell_last_slot(pool: MySqlPool) -> anyhow::Result<()> {
    let host_id = seed_host(&pool).await?;
    seed_emulator(&pool, host_id, "emu-a", 1).await?;
    let account_a = seed_account(&pool, 1001).await?;
    let account_b = seed_account(&pool, 1002).await?;

    let allocator_a = BindingAllocator::new(pool.clone());
    let allocator_b = BindingAllocator::new(pool.clone());

    let (left, right) = tokio::join!(
        allocator_a.allocate_pending(account_a),
        allocator_b.allocate_pending(account_b)
    );

    let success_count = [left.is_ok(), right.is_ok()]
        .into_iter()
        .filter(|value| *value)
        .count();

    assert_eq!(success_count, 1);

    let occupied: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM emulator_account_binding
         WHERE status IN ('PENDING', 'ACTIVE', 'MIGRATING')",
    )
    .fetch_one(&pool)
    .await?;

    assert_eq!(occupied, 1);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn allocation_waits_for_busy_emulator_when_capacity_remains(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let host_id = seed_host(&pool).await?;
    let emulator_id = seed_emulator(&pool, host_id, "emu-a", 2).await?;
    let account = seed_account(&pool, 1001).await?;

    let mut blocker = pool.begin().await?;
    sqlx::query(
        "SELECT id
         FROM emulator_instance
         WHERE id = ?
         FOR UPDATE",
    )
    .bind(emulator_id)
    .fetch_one(&mut *blocker)
    .await?;

    let allocator = BindingAllocator::new(pool.clone());
    let allocation = tokio::spawn(async move { allocator.allocate_pending(account).await });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    blocker.commit().await?;

    let result = tokio::time::timeout(std::time::Duration::from_secs(2), allocation).await??;

    assert!(result.is_ok());

    Ok(())
}
