use std::time::Duration;

use foster_server::control_plane::EmulatorLeaseService;
use sqlx::MySqlPool;
use uuid::Uuid;

async fn seed_emulator(pool: &MySqlPool) -> anyhow::Result<i64> {
    let host = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('lease-host', 'lease-host', 'ONLINE')",
    )
    .execute(pool)
    .await?;

    let emulator = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            status, lifecycle_status, occupancy_status
         )
         VALUES (?, 'emu-lease', 'FAKE', 'IDLE', 'READY', 'IDLE')",
    )
    .bind(host.last_insert_id() as i64)
    .execute(pool)
    .await?;

    Ok(emulator.last_insert_id() as i64)
}

#[sqlx::test(migrations = "../../migrations")]
async fn lease_excludes_competing_owner_until_release(pool: MySqlPool) -> anyhow::Result<()> {
    let emulator_id = seed_emulator(&pool).await?;
    let service = EmulatorLeaseService::new(pool.clone());

    let first = service
        .try_acquire(
            emulator_id,
            Uuid::new_v4(),
            "LOGIN",
            "LOGIN-001",
            Duration::from_secs(900),
        )
        .await?;
    assert!(first.is_some());

    let competing = service
        .try_acquire(
            emulator_id,
            Uuid::new_v4(),
            "FOSTER",
            "42",
            Duration::from_secs(900),
        )
        .await?;
    assert!(competing.is_none());

    service.release_owner("LOGIN", "LOGIN-001").await?;

    let after_release = service
        .try_acquire(
            emulator_id,
            Uuid::new_v4(),
            "FOSTER",
            "42",
            Duration::from_secs(900),
        )
        .await?;
    assert!(after_release.is_some());

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn busy_agent_runtime_blocks_server_lease(pool: MySqlPool) -> anyhow::Result<()> {
    let emulator_id = seed_emulator(&pool).await?;

    sqlx::query(
        "UPDATE emulator_instance
         SET occupancy_status = 'BUSY',
             activity_type = 'LOGIN'
         WHERE id = ?",
    )
    .bind(emulator_id)
    .execute(&pool)
    .await?;

    let acquired = EmulatorLeaseService::new(pool)
        .try_acquire(
            emulator_id,
            Uuid::new_v4(),
            "FOSTER",
            "99",
            Duration::from_secs(900),
        )
        .await?;

    assert!(acquired.is_none());
    Ok(())
}
