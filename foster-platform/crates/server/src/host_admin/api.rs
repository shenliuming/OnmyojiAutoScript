use axum::{
    Extension, Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;

use crate::{
    app::AppState,
    onboarding::{AdminAuthConfig, authorize_admin},
};

use super::service::{HostAdminError, HostAdminService, HostView, UpsertHostRequest};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminHostRequest {
    pub host_code: String,
    pub hostname: String,
    pub status: Option<String>,
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

fn status_code(error: HostAdminError) -> StatusCode {
    match error {
        HostAdminError::InvalidHostCode
        | HostAdminError::InvalidHostname
        | HostAdminError::InvalidStatus => StatusCode::BAD_REQUEST,
        HostAdminError::HostNotFound => StatusCode::NOT_FOUND,
        HostAdminError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
