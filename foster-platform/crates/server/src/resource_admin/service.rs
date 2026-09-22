use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::MySqlPool;

use super::repository::{
    ProviderRow, ResourceCycleRow, game_account_exists, insert_resource_cycle, list_cycles,
    list_providers, provider_exists, resource_cycle_exists, update_cycle_status,
    update_provider_status,
    upsert_friend_binding, upsert_provider,
};

#[derive(Debug, thiserror::Error)]
pub enum ResourceAdminError {
    #[error(transparent)]
    Database(sqlx::Error),
    #[error("provider not found")]
    ProviderNotFound,
    #[error("game account not found")]
    GameAccountNotFound,
    #[error("resource cycle not found")]
    CycleNotFound,
    #[error("invalid provider status")]
    InvalidProviderStatus,
    #[error("invalid friend binding status")]
    InvalidBindingStatus,
    #[error("invalid resource cycle status")]
    InvalidCycleStatus,
    #[error("invalid resource type")]
    InvalidResourceType,
    #[error("provider code is required")]
    InvalidProviderCode,
    #[error("provider alias is required")]
    InvalidProviderAlias,
    #[error("provider nickname is required")]
    InvalidNickname,
    #[error("resource cycle end_at must be later than start_at")]
    InvalidTimeRange,
    #[error("slot capacity must be positive")]
    InvalidSlotCapacity,
    #[error("provider alias is already used by another provider")]
    ProviderAliasConflict,
}

impl From<sqlx::Error> for ResourceAdminError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

#[derive(Debug, Clone)]
pub struct UpsertProviderRequest {
    pub provider_code: String,
    pub game_uid: Option<String>,
    pub nickname: String,
    pub provider_alias: String,
    pub server_name: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreateResourceCycleRequest {
    pub resource_type: String,
    pub resource_level: i32,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub slot_capacity: i32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    pub id: i64,
    pub provider_code: String,
    pub game_uid: Option<String>,
    pub nickname: String,
    pub provider_alias: String,
    pub server_name: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceCycleView {
    pub id: i64,
    pub resource_type: String,
    pub resource_level: i32,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub slot_capacity: i32,
    pub occupied_slots: i32,
    pub status: String,
    pub verified_binding_count: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPoolView {
    #[serde(flatten)]
    pub provider: ProviderView,
    pub cycles: Vec<ResourceCycleView>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePoolView {
    pub providers: Vec<ProviderPoolView>,
}

#[derive(Clone)]
pub struct ResourceAdminService {
    pool: MySqlPool,
}

impl ResourceAdminService {
    pub fn new(pool: MySqlPool) -> Self {
        Self { pool }
    }

    pub async fn upsert_provider(
        &self,
        request: UpsertProviderRequest,
    ) -> Result<ProviderView, ResourceAdminError> {
        let provider_code = request.provider_code.trim();
        let provider_alias = request.provider_alias.trim();
        let nickname = request.nickname.trim();

        if provider_code.is_empty() {
            return Err(ResourceAdminError::InvalidProviderCode);
        }
        if provider_alias.is_empty() {
            return Err(ResourceAdminError::InvalidProviderAlias);
        }
        if nickname.is_empty() {
            return Err(ResourceAdminError::InvalidNickname);
        }

        let status = normalize_provider_status(request.status.as_deref().unwrap_or("ACTIVE"))?;

        let id = upsert_provider(
            &self.pool,
            provider_code,
            clean_optional(request.game_uid.as_deref()),
            nickname,
            provider_alias,
            clean_optional(request.server_name.as_deref()),
            status,
        )
        .await
        .map_err(map_provider_db_error)?;

        let providers = list_providers(&self.pool).await?;
        providers
            .into_iter()
            .find(|provider| provider.id == id)
            .map(provider_view)
            .ok_or(ResourceAdminError::ProviderNotFound)
    }

    pub async fn set_provider_status(
        &self,
        provider_id: i64,
        status: &str,
    ) -> Result<(), ResourceAdminError> {
        let status = normalize_provider_status(status)?;
        if !provider_exists(&self.pool, provider_id).await? {
            return Err(ResourceAdminError::ProviderNotFound);
        }
        update_provider_status(&self.pool, provider_id, status).await?;
        Ok(())
    }

    pub async fn set_friend_binding(
        &self,
        game_account_id: i64,
        provider_id: i64,
        status: &str,
    ) -> Result<(), ResourceAdminError> {
        let status = normalize_binding_status(status)?;

        if !game_account_exists(&self.pool, game_account_id).await? {
            return Err(ResourceAdminError::GameAccountNotFound);
        }
        if !provider_exists(&self.pool, provider_id).await? {
            return Err(ResourceAdminError::ProviderNotFound);
        }

        upsert_friend_binding(&self.pool, game_account_id, provider_id, status).await?;
        Ok(())
    }

    pub async fn create_cycle(
        &self,
        provider_id: i64,
        request: CreateResourceCycleRequest,
    ) -> Result<ResourceCycleView, ResourceAdminError> {
        if !provider_exists(&self.pool, provider_id).await? {
            return Err(ResourceAdminError::ProviderNotFound);
        }
        if request.end_at <= request.start_at {
            return Err(ResourceAdminError::InvalidTimeRange);
        }
        if request.slot_capacity <= 0 {
            return Err(ResourceAdminError::InvalidSlotCapacity);
        }

        let resource_type = normalize_resource_type(&request.resource_type)?;
        let id = insert_resource_cycle(
            &self.pool,
            provider_id,
            resource_type,
            request.resource_level,
            request.start_at,
            request.end_at,
            request.slot_capacity,
        )
        .await?;

        let cycles = list_cycles(&self.pool).await?;
        cycles
            .into_iter()
            .find(|cycle| cycle.id == id)
            .map(cycle_view)
            .ok_or(ResourceAdminError::CycleNotFound)
    }

    pub async fn set_cycle_status(
        &self,
        cycle_id: i64,
        status: &str,
    ) -> Result<(), ResourceAdminError> {
        let status = normalize_cycle_status(status)?;
        if !resource_cycle_exists(&self.pool, cycle_id).await? {
            return Err(ResourceAdminError::CycleNotFound);
        }
        update_cycle_status(&self.pool, cycle_id, status).await?;
        Ok(())
    }

    pub async fn get_pool(&self) -> Result<ResourcePoolView, ResourceAdminError> {
        let providers = list_providers(&self.pool).await?;
        let cycles = list_cycles(&self.pool).await?;

        let mut grouped: BTreeMap<i64, Vec<ResourceCycleView>> = BTreeMap::new();
        for cycle in cycles {
            grouped
                .entry(cycle.provider_account_id)
                .or_default()
                .push(cycle_view(cycle));
        }

        Ok(ResourcePoolView {
            providers: providers
                .into_iter()
                .map(|provider| {
                    let id = provider.id;
                    ProviderPoolView {
                        provider: provider_view(provider),
                        cycles: grouped.remove(&id).unwrap_or_default(),
                    }
                })
                .collect(),
        })
    }
}

fn normalize_provider_status(value: &str) -> Result<&'static str, ResourceAdminError> {
    match value.trim().to_ascii_uppercase().as_str() {
        "ACTIVE" => Ok("ACTIVE"),
        "DISABLED" => Ok("DISABLED"),
        _ => Err(ResourceAdminError::InvalidProviderStatus),
    }
}

fn normalize_binding_status(value: &str) -> Result<&'static str, ResourceAdminError> {
    match value.trim().to_ascii_uppercase().as_str() {
        "VERIFIED" => Ok("VERIFIED"),
        "SUSPECT" => Ok("SUSPECT"),
        "DISABLED" => Ok("DISABLED"),
        _ => Err(ResourceAdminError::InvalidBindingStatus),
    }
}

fn normalize_cycle_status(value: &str) -> Result<&'static str, ResourceAdminError> {
    match value.trim().to_ascii_uppercase().as_str() {
        "AVAILABLE" => Ok("AVAILABLE"),
        "DISABLED" => Ok("DISABLED"),
        _ => Err(ResourceAdminError::InvalidCycleStatus),
    }
}

fn normalize_resource_type(value: &str) -> Result<&'static str, ResourceAdminError> {
    match value.trim().to_ascii_uppercase().as_str() {
        "FISH" | "DOUYU" => Ok("FISH"),
        "TAIKO_JADE" | "JADE" | "TAIKO" => Ok("TAIKO_JADE"),
        _ => Err(ResourceAdminError::InvalidResourceType),
    }
}

fn clean_optional(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn provider_view(row: ProviderRow) -> ProviderView {
    ProviderView {
        id: row.id,
        provider_code: row.provider_code,
        game_uid: row.game_uid,
        nickname: row.nickname,
        provider_alias: row.provider_alias,
        server_name: row.server_name,
        status: row.status,
    }
}

fn cycle_view(row: ResourceCycleRow) -> ResourceCycleView {
    ResourceCycleView {
        id: row.id,
        resource_type: row.resource_type,
        resource_level: row.resource_level,
        start_at: row.start_at.and_utc(),
        end_at: row.end_at.and_utc(),
        slot_capacity: row.slot_capacity,
        occupied_slots: row.occupied_slots,
        status: row.status,
        verified_binding_count: row.verified_binding_count,
    }
}

fn map_provider_db_error(error: sqlx::Error) -> ResourceAdminError {
    if let sqlx::Error::Database(database) = &error {
        if database.code().as_deref() == Some("1062")
            && database.message().contains("uk_provider_alias")
        {
            return ResourceAdminError::ProviderAliasConflict;
        }
    }

    ResourceAdminError::Database(error)
}
