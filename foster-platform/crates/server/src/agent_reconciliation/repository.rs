use sqlx::MySqlPool;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ExecutingFosterJob {
    pub id: i64,
    pub retry_count: i32,
    pub status: String,
}

pub async fn list_host_executing_foster_jobs(
    pool: &MySqlPool,
    host_id: i64,
) -> Result<Vec<ExecutingFosterJob>, sqlx::Error> {
    sqlx::query_as::<_, ExecutingFosterJob>(
        "SELECT j.id, j.retry_count, j.status
         FROM foster_job j
         JOIN emulator_instance e ON e.id = j.emulator_id
         WHERE e.host_id = ?
           AND j.status IN ('VERIFYING_ACCOUNT', 'RUNNING')
         ORDER BY j.id",
    )
    .bind(host_id)
    .fetch_all(pool)
    .await
}

pub async fn mark_recovery_required(
    pool: &MySqlPool,
    host_id: i64,
    job_id: i64,
    attempt: i32,
    reason: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE foster_job j
         JOIN emulator_instance e ON e.id = j.emulator_id
         SET j.status = 'RECOVERY_REQUIRED',
             j.error_code = 'AGENT_RECOVERY_REQUIRED',
             j.result_message = ?,
             j.retry_after = NULL
         WHERE j.id = ?
           AND j.retry_count = ?
           AND e.host_id = ?
           AND j.status IN ('SWITCHING_ACCOUNT', 'VERIFYING_ACCOUNT', 'RUNNING')",
    )
    .bind(reason)
    .bind(job_id)
    .bind(attempt)
    .bind(host_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() == 1)
}
