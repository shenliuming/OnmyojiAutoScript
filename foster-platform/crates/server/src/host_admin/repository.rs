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
                  AND e.status <> 'OFFLINE'
            ) AS online_emulators,
            COALESCE((
                SELECT SUM(e.max_account_count)
                FROM emulator_instance e
                WHERE e.host_id = h.id
            ), 0) AS configured_capacity,
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
                  AND e.status <> 'OFFLINE'
            ) AS online_emulators,
            COALESCE((
                SELECT SUM(e.max_account_count)
                FROM emulator_instance e
                WHERE e.host_id = h.id
            ), 0) AS configured_capacity,
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
