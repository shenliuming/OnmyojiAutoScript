use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::MySqlPool;

use super::repository::{
    EmulatorAdminRow, HostAdminRow, get_emulator, get_host, list_host_emulators, list_hosts,
    update_emulator_capacity, upsert_host,
};

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
    #[error("emulator not found")]
    EmulatorNotFound,
    #[error("max account count must be between 1 and 100")]
    InvalidEmulatorCapacity,
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmulatorView {
    pub id: i64,
    pub host_id: i64,
    pub emulator_code: String,
    pub driver_type: String,
    pub max_account_count: i32,
    pub status: String,
    pub adb_serial: Option<String>,
    pub current_job_id: Option<String>,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
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

    pub async fn list_emulators(
        &self,
        host_id: i64,
    ) -> Result<Vec<EmulatorView>, HostAdminError> {
        if get_host(&self.pool, host_id).await?.is_none() {
            return Err(HostAdminError::HostNotFound);
        }

        Ok(list_host_emulators(&self.pool, host_id)
            .await?
            .into_iter()
            .map(emulator_view)
            .collect())
    }

    pub async fn set_emulator_capacity(
        &self,
        emulator_id: i64,
        max_account_count: i32,
    ) -> Result<EmulatorView, HostAdminError> {
        if !(1..=100).contains(&max_account_count) {
            return Err(HostAdminError::InvalidEmulatorCapacity);
        }

        if !update_emulator_capacity(&self.pool, emulator_id, max_account_count).await? {
            return Err(HostAdminError::EmulatorNotFound);
        }

        get_emulator(&self.pool, emulator_id)
            .await?
            .map(emulator_view)
            .ok_or(HostAdminError::EmulatorNotFound)
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

fn emulator_view(row: EmulatorAdminRow) -> EmulatorView {
    EmulatorView {
        id: row.id,
        host_id: row.host_id,
        emulator_code: row.emulator_code,
        driver_type: row.driver_type,
        max_account_count: row.max_account_count,
        status: row.status,
        adb_serial: row.adb_serial,
        current_job_id: row.current_job_id,
        last_heartbeat_at: row.last_heartbeat_at.map(|value| value.and_utc()),
        bound_accounts: row.bound_accounts,
    }
}
