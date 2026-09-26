use foster_domain::{EmulatorActivity, EmulatorLifecycleStatus, EmulatorOccupancyStatus};
use foster_protocol::{EmulatorDescriptor, EmulatorHeartbeat};
use sqlx::{MySql, MySqlPool, Transaction};

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

pub async fn host_exists(pool: &MySqlPool, host_id: i64) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM host WHERE id = ?")
        .bind(host_id)
        .fetch_one(pool)
        .await?;

    Ok(count > 0)
}

pub async fn mark_host_online(
    pool: &MySqlPool,
    host_id: i64,
    agent_version: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE host
         SET status = 'ONLINE',
             agent_version = ?,
             last_heartbeat_at = NOW(3)
         WHERE id = ?",
    )
    .bind(agent_version)
    .bind(host_id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn touch_host_heartbeat(pool: &MySqlPool, host_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE host
         SET last_heartbeat_at = NOW(3)
         WHERE id = ?",
    )
    .bind(host_id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn mark_host_offline(pool: &MySqlPool, host_id: i64) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "UPDATE host
         SET status = 'OFFLINE'
         WHERE id = ?",
    )
    .bind(host_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE emulator_instance
         SET status = 'OFFLINE',
             lifecycle_status = 'OFFLINE',
             occupancy_status = CASE
                 WHEN occupancy_status = 'BUSY' THEN 'RECOVERY'
                 ELSE occupancy_status
             END
         WHERE host_id = ?
           AND lifecycle_status <> 'MAINTENANCE'",
    )
    .bind(host_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn upsert_emulator_snapshot(
    pool: &MySqlPool,
    host_id: i64,
    items: &[EmulatorDescriptor],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    for item in items {
        sqlx::query(
            "INSERT INTO emulator_instance(
                host_id, emulator_code, driver_type, adb_serial,
                status, lifecycle_status, occupancy_status,
                activity_type, last_heartbeat_at
             )
             VALUES (?, ?, ?, ?, 'OFFLINE', 'OFFLINE', 'IDLE', 'NONE', NOW(3))
             ON DUPLICATE KEY UPDATE
                host_id = VALUES(host_id),
                driver_type = VALUES(driver_type),
                adb_serial = VALUES(adb_serial),
                last_heartbeat_at = VALUES(last_heartbeat_at)",
        )
        .bind(host_id)
        .bind(&item.emulator_code)
        .bind(&item.driver_type)
        .bind(&item.adb_serial)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
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
         WHERE lifecycle_status = 'READY'
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
           AND lifecycle_status = 'READY'
         FOR UPDATE",
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

pub async fn update_emulator_heartbeats(
    pool: &MySqlPool,
    host_id: i64,
    items: &[EmulatorHeartbeat],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    for item in items {
        sqlx::query(
            "UPDATE emulator_instance
             SET status = ?,
                 lifecycle_status = ?,
                 occupancy_status = ?,
                 activity_type = ?,
                 activity_stage = ?,
                 current_command_id = ?,
                 current_foster_job_id = ?,
                 current_login_session_no = ?,
                 current_game_account_id = ?,
                 activity_started_at = ?,
                 last_heartbeat_at = NOW(3)
             WHERE host_id = ?
               AND emulator_code = ?",
        )
        .bind(emulator_legacy_status_name(
            item.lifecycle,
            item.occupancy,
            item.activity,
        ))
        .bind(emulator_lifecycle_name(item.lifecycle))
        .bind(emulator_occupancy_name(item.occupancy))
        .bind(emulator_activity_name(item.activity))
        .bind(&item.activity_stage)
        .bind(item.current_command_id.map(|value| value.to_string()))
        .bind(item.current_job_id)
        .bind(&item.current_login_session_no)
        .bind(item.current_game_account_id)
        .bind(item.activity_started_at.map(|value| value.naive_utc()))
        .bind(host_id)
        .bind(&item.emulator_code)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

fn emulator_lifecycle_name(status: EmulatorLifecycleStatus) -> &'static str {
    match status {
        EmulatorLifecycleStatus::Offline => "OFFLINE",
        EmulatorLifecycleStatus::Starting => "STARTING",
        EmulatorLifecycleStatus::Booting => "BOOTING",
        EmulatorLifecycleStatus::Ready => "READY",
        EmulatorLifecycleStatus::Stopping => "STOPPING",
        EmulatorLifecycleStatus::Maintenance => "MAINTENANCE",
        EmulatorLifecycleStatus::Error => "ERROR",
    }
}

fn emulator_occupancy_name(status: EmulatorOccupancyStatus) -> &'static str {
    match status {
        EmulatorOccupancyStatus::Idle => "IDLE",
        EmulatorOccupancyStatus::Busy => "BUSY",
        EmulatorOccupancyStatus::Recovery => "RECOVERY",
        EmulatorOccupancyStatus::Maintenance => "MAINTENANCE",
    }
}

fn emulator_activity_name(activity: EmulatorActivity) -> &'static str {
    match activity {
        EmulatorActivity::None => "NONE",
        EmulatorActivity::Login => "LOGIN",
        EmulatorActivity::Foster => "FOSTER",
        EmulatorActivity::ManualControl => "MANUAL_CONTROL",
    }
}

fn emulator_legacy_status_name(
    lifecycle: EmulatorLifecycleStatus,
    occupancy: EmulatorOccupancyStatus,
    activity: EmulatorActivity,
) -> &'static str {
    match lifecycle {
        EmulatorLifecycleStatus::Offline => "OFFLINE",
        EmulatorLifecycleStatus::Maintenance => "MAINTENANCE",
        EmulatorLifecycleStatus::Error => "ERROR",
        EmulatorLifecycleStatus::Starting
        | EmulatorLifecycleStatus::Booting
        | EmulatorLifecycleStatus::Stopping => "RUNNING",
        EmulatorLifecycleStatus::Ready => match occupancy {
            EmulatorOccupancyStatus::Idle => "IDLE",
            EmulatorOccupancyStatus::Recovery => "ERROR",
            EmulatorOccupancyStatus::Maintenance => "MAINTENANCE",
            EmulatorOccupancyStatus::Busy => match activity {
                EmulatorActivity::Login => "LOGIN_SESSION",
                EmulatorActivity::None
                | EmulatorActivity::Foster
                | EmulatorActivity::ManualControl => "RUNNING",
            },
        },
    }
}
