use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::app::AppState;

use super::service::{
    PublicPortalError, PublicPortalService, PublicQuietPeriod, PublicServiceStatus,
    QuietPeriodInput,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PauseRequest {
    pub preset: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PauseResponse {
    pub manual_pause_until: chrono::DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceQuietPeriodsRequest {
    pub quiet_periods: Vec<QuietPeriodRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuietPeriodRequest {
    pub weekday_mask: i32,
    pub start_time: String,
    pub end_time: String,
    pub before_buffer_minutes: i32,
    pub after_buffer_minutes: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuietPeriodsResponse {
    pub quiet_periods: Vec<PublicQuietPeriod>,
}

pub async fn get_service_status(
    Path(public_token): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<PublicServiceStatus>, StatusCode> {
    PublicPortalService::new(state.pool)
        .load_public_status(&public_token, Utc::now())
        .await
        .map(Json)
        .map_err(status_code)
}

pub async fn pause_service(
    Path(control_token): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<PauseRequest>,
) -> Result<Json<PauseResponse>, StatusCode> {
    let until = PublicPortalService::new(state.pool)
        .pause(&control_token, &request.preset, Utc::now())
        .await
        .map_err(status_code)?;

    Ok(Json(PauseResponse {
        manual_pause_until: until,
    }))
}

pub async fn clear_pause(
    Path(control_token): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode, StatusCode> {
    PublicPortalService::new(state.pool)
        .clear_pause(&control_token, Utc::now())
        .await
        .map_err(status_code)?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn replace_quiet_periods(
    Path(control_token): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<ReplaceQuietPeriodsRequest>,
) -> Result<Json<QuietPeriodsResponse>, StatusCode> {
    let inputs = request
        .quiet_periods
        .into_iter()
        .map(|item| QuietPeriodInput {
            weekday_mask: item.weekday_mask,
            start_time: item.start_time,
            end_time: item.end_time,
            before_buffer_minutes: item.before_buffer_minutes,
            after_buffer_minutes: item.after_buffer_minutes,
        })
        .collect();

    let quiet_periods = PublicPortalService::new(state.pool)
        .replace_quiet_periods(&control_token, inputs, Utc::now())
        .await
        .map_err(status_code)?;

    Ok(Json(QuietPeriodsResponse { quiet_periods }))
}

fn status_code(error: PublicPortalError) -> StatusCode {
    match error {
        PublicPortalError::NotFound => StatusCode::NOT_FOUND,
        PublicPortalError::Expired => StatusCode::GONE,
        PublicPortalError::InvalidPausePreset | PublicPortalError::InvalidQuietPeriod => {
            StatusCode::BAD_REQUEST
        }
        PublicPortalError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
