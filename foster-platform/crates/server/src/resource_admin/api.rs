use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppState,
    onboarding::{AdminAuthConfig, authorize_admin},
};

use super::service::{
    CreateResourceCycleRequest, ResourceAdminError, ResourceAdminService, ResourceCycleView,
    ResourcePoolView, UpsertProviderRequest, ProviderView,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminProviderRequest {
    pub provider_code: String,
    pub game_uid: Option<String>,
    pub nickname: String,
    pub provider_alias: String,
    pub server_name: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminStatusRequest {
    pub status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminResourceCycleRequest {
    pub resource_type: String,
    pub resource_level: i32,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub slot_capacity: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminMutationResult {
    pub success: bool,
}

pub async fn admin_upsert_provider(
    State(state): State<AppState>,
    Extension(admin_auth): Extension<AdminAuthConfig>,
    headers: HeaderMap,
    Json(request): Json<AdminProviderRequest>,
) -> Result<Json<ProviderView>, StatusCode> {
    authorize_admin(&admin_auth, &headers)?;

    ResourceAdminService::new(state.pool)
        .upsert_provider(UpsertProviderRequest {
            provider_code: request.provider_code,
            game_uid: request.game_uid,
            nickname: request.nickname,
            provider_alias: request.provider_alias,
            server_name: request.server_name,
            status: request.status,
        })
        .await
        .map(Json)
        .map_err(status_code)
}

pub async fn admin_set_provider_status(
    State(state): State<AppState>,
    Extension(admin_auth): Extension<AdminAuthConfig>,
    headers: HeaderMap,
    Path(provider_id): Path<i64>,
    Json(request): Json<AdminStatusRequest>,
) -> Result<Json<AdminMutationResult>, StatusCode> {
    authorize_admin(&admin_auth, &headers)?;

    ResourceAdminService::new(state.pool)
        .set_provider_status(provider_id, &request.status)
        .await
        .map(|_| Json(AdminMutationResult { success: true }))
        .map_err(status_code)
}

pub async fn admin_set_friend_binding(
    State(state): State<AppState>,
    Extension(admin_auth): Extension<AdminAuthConfig>,
    headers: HeaderMap,
    Path((game_account_id, provider_id)): Path<(i64, i64)>,
    Json(request): Json<AdminStatusRequest>,
) -> Result<Json<AdminMutationResult>, StatusCode> {
    authorize_admin(&admin_auth, &headers)?;

    ResourceAdminService::new(state.pool)
        .set_friend_binding(game_account_id, provider_id, &request.status)
        .await
        .map(|_| Json(AdminMutationResult { success: true }))
        .map_err(status_code)
}

pub async fn admin_create_cycle(
    State(state): State<AppState>,
    Extension(admin_auth): Extension<AdminAuthConfig>,
    headers: HeaderMap,
    Path(provider_id): Path<i64>,
    Json(request): Json<AdminResourceCycleRequest>,
) -> Result<Json<ResourceCycleView>, StatusCode> {
    authorize_admin(&admin_auth, &headers)?;

    ResourceAdminService::new(state.pool)
        .create_cycle(
            provider_id,
            CreateResourceCycleRequest {
                resource_type: request.resource_type,
                resource_level: request.resource_level,
                start_at: request.start_at,
                end_at: request.end_at,
                slot_capacity: request.slot_capacity,
            },
        )
        .await
        .map(Json)
        .map_err(status_code)
}

pub async fn admin_set_cycle_status(
    State(state): State<AppState>,
    Extension(admin_auth): Extension<AdminAuthConfig>,
    headers: HeaderMap,
    Path(cycle_id): Path<i64>,
    Json(request): Json<AdminStatusRequest>,
) -> Result<Json<AdminMutationResult>, StatusCode> {
    authorize_admin(&admin_auth, &headers)?;

    ResourceAdminService::new(state.pool)
        .set_cycle_status(cycle_id, &request.status)
        .await
        .map(|_| Json(AdminMutationResult { success: true }))
        .map_err(status_code)
}

pub async fn admin_get_resource_pool(
    State(state): State<AppState>,
    Extension(admin_auth): Extension<AdminAuthConfig>,
    headers: HeaderMap,
) -> Result<Json<ResourcePoolView>, StatusCode> {
    authorize_admin(&admin_auth, &headers)?;

    ResourceAdminService::new(state.pool)
        .get_pool()
        .await
        .map(Json)
        .map_err(status_code)
}

fn status_code(error: ResourceAdminError) -> StatusCode {
    match error {
        ResourceAdminError::InvalidProviderStatus
        | ResourceAdminError::InvalidBindingStatus
        | ResourceAdminError::InvalidCycleStatus
        | ResourceAdminError::InvalidResourceType
        | ResourceAdminError::InvalidProviderCode
        | ResourceAdminError::InvalidProviderAlias
        | ResourceAdminError::InvalidNickname
        | ResourceAdminError::InvalidTimeRange
        | ResourceAdminError::InvalidSlotCapacity => StatusCode::BAD_REQUEST,
        ResourceAdminError::ProviderNotFound
        | ResourceAdminError::GameAccountNotFound
        | ResourceAdminError::CycleNotFound => StatusCode::NOT_FOUND,
        ResourceAdminError::ProviderAliasConflict => StatusCode::CONFLICT,
        ResourceAdminError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
