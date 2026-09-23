use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;

use crate::{
    app::AppState,
    onboarding::{AdminAuthConfig, authorize_admin},
};

use super::service::{
    EmulatorView, HostAdminError, HostAdminService, HostView, UpsertHostRequest,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminHostRequest {
    pub host_code: String,
    pub hostname: String,
    pub status: Option<String>,
}



#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminEmulatorCapacityRequest {
    pub max_account_count: i32,
}

pub async fn admin_upsert_host(
    State(state): State<AppState>,
    Extension(admin_auth): Extension<AdminAuthConfig>,
    headers: HeaderMap,
    Json(request): Json<AdminHostRequest>,
) -> Result<Json<HostView>, StatusCode> {
    authorize_admin(&admin_auth, &headers)?;

    HostAdminService::new(state.pool)
        .upsert_host(UpsertHostRequest {
            host_code: request.host_code,
            hostname: request.hostname,
            status: request.status,
        })
        .await
        .map(Json)
        .map_err(status_code)
}

pub async fn admin_list_hosts(
    State(state): State<AppState>,
    Extension(admin_auth): Extension<AdminAuthConfig>,
    headers: HeaderMap,
) -> Result<Json<Vec<HostView>>, StatusCode> {
    authorize_admin(&admin_auth, &headers)?;

    HostAdminService::new(state.pool)
        .list_hosts()
        .await
        .map(Json)
        .map_err(status_code)
}



pub async fn admin_list_host_emulators(
    State(state): State<AppState>,
    Extension(admin_auth): Extension<AdminAuthConfig>,
    headers: HeaderMap,
    Path(host_id): Path<i64>,
) -> Result<Json<Vec<EmulatorView>>, StatusCode> {
    authorize_admin(&admin_auth, &headers)?;

    HostAdminService::new(state.pool)
        .list_emulators(host_id)
        .await
        .map(Json)
        .map_err(status_code)
}

pub async fn admin_set_emulator_capacity(
    State(state): State<AppState>,
    Extension(admin_auth): Extension<AdminAuthConfig>,
    headers: HeaderMap,
    Path(emulator_id): Path<i64>,
    Json(request): Json<AdminEmulatorCapacityRequest>,
) -> Result<Json<EmulatorView>, StatusCode> {
    authorize_admin(&admin_auth, &headers)?;

    HostAdminService::new(state.pool)
        .set_emulator_capacity(emulator_id, request.max_account_count)
        .await
        .map(Json)
        .map_err(status_code)
}

fn status_code(error: HostAdminError) -> StatusCode {
    match error {
        HostAdminError::InvalidHostCode
        | HostAdminError::InvalidHostname
        | HostAdminError::InvalidStatus
        | HostAdminError::InvalidEmulatorCapacity => StatusCode::BAD_REQUEST,
        HostAdminError::HostNotFound | HostAdminError::EmulatorNotFound => StatusCode::NOT_FOUND,
        HostAdminError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
