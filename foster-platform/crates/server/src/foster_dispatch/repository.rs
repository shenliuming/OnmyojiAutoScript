use sqlx::MySqlPool;

#[derive(Debug, sqlx::FromRow)]
pub struct FosterDispatchTargetRow {
    pub job_id: i64,
    pub game_account_id: i64,
    pub status: String,
    pub host_id: i64,
    pub emulator_code: String,
    pub resource_mode: String,
    pub resource_type: Option<String>,
    pub character_name: Option<String>,
    pub server_name: Option<String>,
    pub game_uid: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct FosterIdentityRow {
    pub identity_type: String,
    pub identity_value: String,
    pub normalized_value: String,
    pub confidence: i32,
}

pub async fn load_dispatch_target(
    pool: &MySqlPool,
    job_id: i64,
) -> Result<Option<FosterDispatchTargetRow>, sqlx::Error> {
    sqlx::query_as::<_, FosterDispatchTargetRow>(
        "SELECT
            j.id AS job_id,
            j.game_account_id,
            j.status,
            e.host_id,
            e.emulator_code,
            s.resource_mode,
            s.resource_type,
            a.character_name,
            a.server_name,
            a.game_uid
         FROM foster_job j
         JOIN foster_subscription s ON s.id = j.subscription_id
         JOIN emulator_instance e ON e.id = j.emulator_id
         JOIN game_account a ON a.id = j.game_account_id
         WHERE j.id = ?",
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await
}

pub async fn load_identity_rows(
    pool: &MySqlPool,
    game_account_id: i64,
) -> Result<Vec<FosterIdentityRow>, sqlx::Error> {
    sqlx::query_as::<_, FosterIdentityRow>(
        "SELECT identity_type, identity_value, normalized_value, confidence
         FROM game_account_identity
         WHERE game_account_id = ?
           AND enabled = 1
           AND identity_type IN (
               'MASKED_ACCOUNT',
               'OCR_ALIAS',
               'CHARACTER_NAME',
               'SERVER_NAME',
               'GAME_UID'
           )
         ORDER BY confidence DESC, id ASC",
    )
    .bind(game_account_id)
    .fetch_all(pool)
    .await
}

pub async fn current_job_status(
    pool: &MySqlPool,
    job_id: i64,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT status
         FROM foster_job
         WHERE id = ?",
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await
}

pub async fn job_belongs_to_host(
    pool: &MySqlPool,
    job_id: i64,
    host_id: i64,
) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM foster_job j
         JOIN emulator_instance e ON e.id = j.emulator_id
         WHERE j.id = ?
           AND e.host_id = ?",
    )
    .bind(job_id)
    .bind(host_id)
    .fetch_one(pool)
    .await?;

    Ok(count == 1)
}

pub async fn set_job_screenshot_url(
    pool: &MySqlPool,
    job_id: i64,
    screenshot_url: Option<&str>,
) -> Result<(), sqlx::Error> {
    if screenshot_url.is_none() {
        return Ok(());
    }

    sqlx::query(
        "UPDATE foster_job
         SET screenshot_url = ?
         WHERE id = ?",
    )
    .bind(screenshot_url)
    .bind(job_id)
    .execute(pool)
    .await?;

    Ok(())
}


pub async fn set_job_waiting_resource(
    pool: &MySqlPool,
    job_id: i64,
    message: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE foster_job
         SET status = 'WAITING_RESOURCE',
             error_code = 'PROVIDER_NOT_FOUND',
             result_message = ?,
             retry_after = NULL
         WHERE id = ?
           AND status IN ('SWITCHING_ACCOUNT', 'WAITING_RESOURCE')",
    )
    .bind(message)
    .bind(job_id)
    .execute(pool)
    .await?;

    Ok(())
}
