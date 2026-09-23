use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;

use crate::{
    app::AppState,
    onboarding::{AdminAuthConfig, authorize_admin},
};

use super::service::{
    RecoveryError, RecoveryJobView, RecoveryResolutionResult, RecoveryService,
    ResolveRecoveryRequest,
};

#[derive(Debug, Deserialize)]
pub struct RecoveryListQuery {
    pub limit: Option<u32>,
}

pub async fn admin_list_recovery_jobs(
    State(state): State<AppState>,
    Extension(auth): Extension<AdminAuthConfig>,
    headers: HeaderMap,
    Query(query): Query<RecoveryListQuery>,
) -> Result<Json<Vec<RecoveryJobView>>, StatusCode> {
    authorize_admin(&auth, &headers)?;

    RecoveryService::new(state.pool)
        .list(query.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(status_code)
}

pub async fn admin_resolve_recovery_job(
    State(state): State<AppState>,
    Extension(auth): Extension<AdminAuthConfig>,
    headers: HeaderMap,
    Path(job_id): Path<i64>,
    Json(request): Json<ResolveRecoveryRequest>,
) -> Result<Json<RecoveryResolutionResult>, StatusCode> {
    authorize_admin(&auth, &headers)?;

    RecoveryService::new(state.pool)
        .resolve(job_id, request, chrono::Utc::now())
        .await
        .map(Json)
        .map_err(status_code)
}

fn status_code(error: RecoveryError) -> StatusCode {
    match error {
        RecoveryError::InvalidOperator
        | RecoveryError::InvalidNote
        | RecoveryError::MustConfirmStopped
        | RecoveryError::RemainingRequired
        | RecoveryError::InvalidRemaining => StatusCode::BAD_REQUEST,
        RecoveryError::JobNotFound => StatusCode::NOT_FOUND,
        RecoveryError::StateConflict
        | RecoveryError::AttemptMismatch
        | RecoveryError::AllocationUnavailable
        | RecoveryError::SubscriptionUnavailable => StatusCode::CONFLICT,
        RecoveryError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
