use axum::{
    Extension, Json,
    extract::State,
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
};
use serde::Deserialize;

use crate::app::AppState;

use super::service::{
    OnboardCustomerRequest, OnboardCustomerResult, OnboardingError, OnboardingService,
};

#[derive(Debug, Clone)]
pub struct AdminAuthConfig {
    pub token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminOnboardRequest {
    pub customer_id: i64,
    pub plan_code: String,
    pub service_days: i32,
    pub login_ttl_minutes: i32,
}

pub async fn admin_onboard(
    State(state): State<AppState>,
    Extension(admin_auth): Extension<AdminAuthConfig>,
    headers: HeaderMap,
    Json(request): Json<AdminOnboardRequest>,
) -> Result<Json<OnboardCustomerResult>, StatusCode> {
    authorize_admin(&admin_auth, &headers)?;

    OnboardingService::new(state.pool.clone())
        .onboard(
            OnboardCustomerRequest {
                customer_id: request.customer_id,
                plan_code: request.plan_code,
                service_days: request.service_days,
                login_ttl_minutes: request.login_ttl_minutes,
            },
            &state.registry,
        )
        .await
        .map(Json)
        .map_err(status_code)
}

fn authorize_admin(config: &AdminAuthConfig, headers: &HeaderMap) -> Result<(), StatusCode> {
    let Some(expected) = config.token.as_deref().filter(|value| !value.is_empty()) else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };

    let actual = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    if actual == Some(expected) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

fn status_code(error: OnboardingError) -> StatusCode {
    match error {
        OnboardingError::UnknownPlan
        | OnboardingError::InvalidServiceDays
        | OnboardingError::InvalidLoginTtl => StatusCode::BAD_REQUEST,
        OnboardingError::NoCapacity => StatusCode::CONFLICT,
        OnboardingError::Enrollment(error) => match error {
            crate::enrollment::EnrollmentError::LoginSessionNotFound => StatusCode::NOT_FOUND,
            crate::enrollment::EnrollmentError::LoginSessionExpired => StatusCode::GONE,
            crate::enrollment::EnrollmentError::InvalidLoginState
            | crate::enrollment::EnrollmentError::IdentityRejected
            | crate::enrollment::EnrollmentError::BindingMismatch
            | crate::enrollment::EnrollmentError::AccountBindingConflict => StatusCode::CONFLICT,
            crate::enrollment::EnrollmentError::Allocation(_)
            | crate::enrollment::EnrollmentError::Database(_)
            | crate::enrollment::EnrollmentError::InvalidTtl => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        },
        OnboardingError::Portal(_) | OnboardingError::Database(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
