use sqlx::MySqlPool;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HostAdminRow {
    pub id: i64,
    pub host_code: String,
    pub hostname: String,
    pub status: String,
    pub agent_version: Option<String>,
    pub last_heartbeat_at: Option<chrono::NaiveDateTime>,
    pub total_emulators: i64,
    pub online_emulators: i64,
    pub configured_capacity: i64,
    pub bound_accounts: i64,
}

pub async fn upsert_host(
    pool: &MySqlPool,
    host_code: &str,
    hostname: &str,
    status: &str,
) -> Result<i64, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let existing_id: Option<i64> = sqlx::query_scalar(
        "SELECT id
         FROM host
         WHERE host_code = ?
         FOR UPDATE",
    )
    .bind(host_code)
    .fetch_optional(&mut *tx)
    .await?;

    let id = if let Some(id) = existing_id {
        sqlx::query(
            "UPDATE host
             SET hostname = ?,
                 status = CASE
                    WHEN status = 'ONLINE' THEN status
                    ELSE ?
                 END
             WHERE id = ?",
        )
        .bind(hostname)
        .bind(status)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        id
    } else {
        sqlx::query(
            "INSERT INTO host(host_code, hostname, status)
             VALUES (?, ?, ?)",
        )
        .bind(host_code)
        .bind(hostname)
        .bind(status)
        .execute(&mut *tx)
        .await?
        .last_insert_id() as i64
    };

    tx.commit().await?;
    Ok(id)
}

pub async fn get_host(pool: &MySqlPool, host_id: i64) -> Result<Option<HostAdminRow>, sqlx::Error> {
    sqlx::query_as::<_, HostAdminRow>(
        "SELECT
            h.id,
            h.host_code,
            h.hostname,
            h.status,
            h.agent_version,
            h.last_heartbeat_at,
            (
                SELECT COUNT(*)
                FROM emulator_instance e
                WHERE e.host_id = h.id
            ) AS total_emulators,
            (
                SELECT COUNT(*)
                FROM emulator_instance e
                WHERE e.host_id = h.id
                  AND e.lifecycle_status <> 'OFFLINE'
            ) AS online_emulators,
            CAST(COALESCE((
                SELECT SUM(e.max_account_count)
                FROM emulator_instance e
                WHERE e.host_id = h.id
            ), 0) AS SIGNED) AS configured_capacity,
            (
                SELECT COUNT(*)
                FROM emulator_account_binding b
                JOIN emulator_instance e ON e.id = b.emulator_id
                WHERE e.host_id = h.id
                  AND b.status IN ('PENDING', 'ACTIVE', 'MIGRATING')
            ) AS bound_accounts
         FROM host h
         WHERE h.id = ?",
    )
    .bind(host_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_hosts(pool: &MySqlPool) -> Result<Vec<HostAdminRow>, sqlx::Error> {
    sqlx::query_as::<_, HostAdminRow>(
        "SELECT
            h.id,
            h.host_code,
            h.hostname,
            h.status,
            h.agent_version,
            h.last_heartbeat_at,
            (
                SELECT COUNT(*)
                FROM emulator_instance e
                WHERE e.host_id = h.id
            ) AS total_emulators,
            (
                SELECT COUNT(*)
                FROM emulator_instance e
                WHERE e.host_id = h.id
                  AND e.lifecycle_status <> 'OFFLINE'
            ) AS online_emulators,
            CAST(COALESCE((
                SELECT SUM(e.max_account_count)
                FROM emulator_instance e
                WHERE e.host_id = h.id
            ), 0) AS SIGNED) AS configured_capacity,
            (
                SELECT COUNT(*)
                FROM emulator_account_binding b
                JOIN emulator_instance e ON e.id = b.emulator_id
                WHERE e.host_id = h.id
                  AND b.status IN ('PENDING', 'ACTIVE', 'MIGRATING')
            ) AS bound_accounts
         FROM host h
         ORDER BY h.id ASC",
    )
    .fetch_all(pool)
    .await
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EmulatorAdminRow {
    pub id: i64,
    pub host_id: i64,
    pub emulator_code: String,
    pub driver_type: String,
    pub max_account_count: i32,
    pub status: String,
    pub lifecycle_status: String,
    pub occupancy_status: String,
    pub activity_type: String,
    pub activity_stage: Option<String>,
    pub adb_serial: Option<String>,
    pub current_job_id: Option<String>,
    pub current_command_id: Option<String>,
    pub current_foster_job_id: Option<i64>,
    pub current_login_session_no: Option<String>,
    pub current_game_account_id: Option<i64>,
    pub activity_started_at: Option<chrono::NaiveDateTime>,
    pub lease_owner_type: Option<String>,
    pub lease_owner_key: Option<String>,
    pub lease_expires_at: Option<chrono::NaiveDateTime>,
    pub last_heartbeat_at: Option<chrono::NaiveDateTime>,
    pub bound_accounts: i64,
}

pub async fn list_host_emulators(
    pool: &MySqlPool,
    host_id: i64,
) -> Result<Vec<EmulatorAdminRow>, sqlx::Error> {
    sqlx::query_as::<_, EmulatorAdminRow>(
        "SELECT
            e.id,
            e.host_id,
            e.emulator_code,
            e.driver_type,
            e.max_account_count,
            e.status,
            e.lifecycle_status,
            e.occupancy_status,
            e.activity_type,
            e.activity_stage,
            e.adb_serial,
            e.current_job_id,
            e.current_command_id,
            e.current_foster_job_id,
            e.current_login_session_no,
            e.current_game_account_id,
            e.activity_started_at,
            l.owner_type AS lease_owner_type,
            l.owner_key AS lease_owner_key,
            l.expires_at AS lease_expires_at,
            e.last_heartbeat_at,
            (
                SELECT COUNT(*)
                FROM emulator_account_binding b
                WHERE b.emulator_id = e.id
                  AND b.status IN ('PENDING', 'ACTIVE', 'MIGRATING')
            ) AS bound_accounts
         FROM emulator_instance e
         LEFT JOIN emulator_lease l
           ON l.emulator_id = e.id
          AND l.expires_at > NOW(3)
         WHERE e.host_id = ?
         ORDER BY e.id ASC",
    )
    .bind(host_id)
    .fetch_all(pool)
    .await
}

pub async fn get_emulator(
    pool: &MySqlPool,
    emulator_id: i64,
) -> Result<Option<EmulatorAdminRow>, sqlx::Error> {
    sqlx::query_as::<_, EmulatorAdminRow>(
        "SELECT
            e.id,
            e.host_id,
            e.emulator_code,
            e.driver_type,
            e.max_account_count,
            e.status,
            e.lifecycle_status,
            e.occupancy_status,
            e.activity_type,
            e.activity_stage,
            e.adb_serial,
            e.current_job_id,
            e.current_command_id,
            e.current_foster_job_id,
            e.current_login_session_no,
            e.current_game_account_id,
            e.activity_started_at,
            l.owner_type AS lease_owner_type,
            l.owner_key AS lease_owner_key,
            l.expires_at AS lease_expires_at,
            e.last_heartbeat_at,
            (
                SELECT COUNT(*)
                FROM emulator_account_binding b
                WHERE b.emulator_id = e.id
                  AND b.status IN ('PENDING', 'ACTIVE', 'MIGRATING')
            ) AS bound_accounts
         FROM emulator_instance e
         LEFT JOIN emulator_lease l
           ON l.emulator_id = e.id
          AND l.expires_at > NOW(3)
         WHERE e.id = ?",
    )
    .bind(emulator_id)
    .fetch_optional(pool)
    .await
}

pub async fn update_emulator_capacity(
    pool: &MySqlPool,
    emulator_id: i64,
    max_account_count: i32,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE emulator_instance
         SET max_account_count = ?
         WHERE id = ?",
    )
    .bind(max_account_count)
    .bind(emulator_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() == 1)
}
