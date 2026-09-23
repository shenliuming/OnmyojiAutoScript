use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::MySqlPool;

use super::repository::{HostAdminRow, get_host, list_hosts, upsert_host};

#[derive(Debug, thiserror::Error)]
pub enum HostAdminError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("host code is required")]
    InvalidHostCode,
    #[error("hostname is required")]
    InvalidHostname,
    #[error("invalid bootstrap host status")]
    InvalidStatus,
    #[error("host not found")]
    HostNotFound,
}

#[derive(Debug, Clone)]
pub struct UpsertHostRequest {
    pub host_code: String,
    pub hostname: String,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostView {
    pub id: i64,
    pub host_code: String,
    pub hostname: String,
    pub status: String,
    pub agent_version: Option<String>,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub total_emulators: i64,
    pub online_emulators: i64,
    pub configured_capacity: i64,
    pub bound_accounts: i64,
}

#[derive(Clone)]
pub struct HostAdminService {
    pool: MySqlPool,
}

impl HostAdminService {
    pub fn new(pool: MySqlPool) -> Self {
        Self { pool }
    }

    pub async fn upsert_host(
        &self,
        request: UpsertHostRequest,
    ) -> Result<HostView, HostAdminError> {
        let host_code = request.host_code.trim();
        let hostname = request.hostname.trim();

        if host_code.is_empty() {
            return Err(HostAdminError::InvalidHostCode);
        }
        if hostname.is_empty() {
            return Err(HostAdminError::InvalidHostname);
        }

        let status = normalize_bootstrap_status(request.status.as_deref().unwrap_or("OFFLINE"))?;
        let id = upsert_host(&self.pool, host_code, hostname, status).await?;

        get_host(&self.pool, id)
            .await?
            .map(host_view)
            .ok_or(HostAdminError::HostNotFound)
    }

    pub async fn list_hosts(&self) -> Result<Vec<HostView>, HostAdminError> {
        Ok(list_hosts(&self.pool)
            .await?
            .into_iter()
            .map(host_view)
            .collect())
    }
}

fn normalize_bootstrap_status(value: &str) -> Result<&'static str, HostAdminError> {
    match value.trim().to_ascii_uppercase().as_str() {
        "OFFLINE" => Ok("OFFLINE"),
        "MAINTENANCE" => Ok("MAINTENANCE"),
        _ => Err(HostAdminError::InvalidStatus),
    }
}

fn host_view(row: HostAdminRow) -> HostView {
    HostView {
        id: row.id,
        host_code: row.host_code,
        hostname: row.hostname,
        status: row.status,
        agent_version: row.agent_version,
        last_heartbeat_at: row.last_heartbeat_at.map(|value| value.and_utc()),
        total_emulators: row.total_emulators,
        online_emulators: row.online_emulators,
        configured_capacity: row.configured_capacity,
        bound_accounts: row.bound_accounts,
    }
}
