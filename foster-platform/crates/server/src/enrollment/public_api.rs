use std::{fmt::Write as _, io::Cursor};

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header::CONTENT_TYPE},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, MySqlPool};

use crate::app::AppState;

use super::{EnrollmentError, EnrollmentService};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PublicLoginStatus {
    pub session_no: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qr_payload: Option<String>,
    pub qr_expires_at: Option<DateTime<Utc>>,
    pub character_name: Option<String>,
    pub server_name: Option<String>,
    pub identity_verified: bool,
    pub identity_verify_reason: Option<String>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct PublicLoginRow {
    pub id: i64,
    pub session_no: String,
    pub status: String,
    pub qr_payload: Option<String>,
    pub qr_expires_at: Option<NaiveDateTime>,
    pub detected_character_name: Option<String>,
    pub detected_server_name: Option<String>,
    pub identity_verified: bool,
    pub identity_verify_reason: Option<String>,
    pub expires_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl PublicLoginRow {
    pub(crate) fn to_public_status(&self) -> PublicLoginStatus {
        let now = Utc::now().naive_utc();
        let qr_is_current = matches!(self.status.as_str(), "QR_READY" | "WAITING_SCAN")
            && self
                .qr_expires_at
                .is_some_and(|expires_at| expires_at > now);

        PublicLoginStatus {
            session_no: self.session_no.clone(),
            status: self.status.clone(),
            qr_payload: qr_is_current.then(|| self.qr_payload.clone()).flatten(),
            qr_expires_at: qr_is_current
                .then_some(self.qr_expires_at)
                .flatten()
                .map(to_utc),
            character_name: self.detected_character_name.clone(),
            server_name: self.detected_server_name.clone(),
            identity_verified: self.identity_verified,
            identity_verify_reason: self.identity_verify_reason.clone(),
            expires_at: to_utc(self.expires_at),
        }
    }

    pub(crate) fn is_expired(&self) -> bool {
        self.expires_at <= Utc::now().naive_utc()
    }
}

#[derive(Debug, Deserialize)]
pub struct ConfirmLoginRequest {
    pub confirmed: bool,
}

#[derive(Debug, Deserialize)]
pub struct SelectLoginPlatformRequest {
    pub platform: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitLoginIdentityRequest {
    pub server_name: String,
    pub character_name: String,
    pub game_uid: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitLoginIdentityResponse {
    pub verified: bool,
    pub waiting_for_detection: bool,
    pub message: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmLoginResponse {
    pub session_no: String,
    pub status: &'static str,
}

pub async fn confirm_login(
    Path(control_token): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<ConfirmLoginRequest>,
) -> Result<Json<ConfirmLoginResponse>, StatusCode> {
    if !request.confirmed {
        return Err(StatusCode::BAD_REQUEST);
    }

    let session_no = EnrollmentService::new(state.pool.clone())
        .confirm_login_session(&control_token)
        .await
        .map_err(activation_status)?;

    Ok(Json(ConfirmLoginResponse {
        session_no,
        status: "SUCCESS",
    }))
}

pub async fn select_login_platform(
    Path(control_token): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<SelectLoginPlatformRequest>,
) -> Result<Json<ConfirmLoginResponse>, StatusCode> {
    let platform = parse_login_platform(&request.platform).ok_or(StatusCode::BAD_REQUEST)?;
    let session_no = EnrollmentService::new(state.pool.clone())
        .select_login_platform(&control_token, platform, &state.registry)
        .await
        .map_err(activation_status)?;

    Ok(Json(ConfirmLoginResponse {
        session_no,
        status: "PLATFORM_SELECTED",
    }))
}

pub async fn submit_login_identity(
    Path(control_token): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<SubmitLoginIdentityRequest>,
) -> Result<Json<SubmitLoginIdentityResponse>, StatusCode> {
    let result = EnrollmentService::new(state.pool.clone())
        .submit_login_identity(
            &control_token,
            &request.server_name,
            &request.character_name,
            &request.game_uid,
        )
        .await
        .map_err(activation_status)?;
    Ok(Json(SubmitLoginIdentityResponse {
        verified: result.verified,
        waiting_for_detection: result.waiting_for_detection,
        message: result.message,
    }))
}

fn parse_login_platform(value: &str) -> Option<foster_protocol::LoginPlatform> {
    match value.trim().to_ascii_lowercase().as_str() {
        "android" | "安卓" => Some(foster_protocol::LoginPlatform::Android),
        "ios" | "apple" | "苹果" => Some(foster_protocol::LoginPlatform::Ios),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_login_platform;
    use foster_protocol::LoginPlatform;

    #[test]
    fn parses_android_and_ios_platforms() {
        assert_eq!(
            parse_login_platform("android"),
            Some(LoginPlatform::Android)
        );
        assert_eq!(parse_login_platform("安卓"), Some(LoginPlatform::Android));
        assert_eq!(parse_login_platform("ios"), Some(LoginPlatform::Ios));
        assert_eq!(parse_login_platform("苹果"), Some(LoginPlatform::Ios));
        assert_eq!(parse_login_platform("windows"), None);
    }
}

fn activation_status(error: EnrollmentError) -> StatusCode {
    match error {
        EnrollmentError::LoginSessionNotFound => StatusCode::NOT_FOUND,
        EnrollmentError::LoginSessionExpired => StatusCode::GONE,
        EnrollmentError::InvalidIdentityInput => StatusCode::BAD_REQUEST,
        EnrollmentError::InvalidLoginState
        | EnrollmentError::IdentityRejected
        | EnrollmentError::BindingMismatch
        | EnrollmentError::AccountBindingConflict => StatusCode::CONFLICT,
        EnrollmentError::Allocation(_)
        | EnrollmentError::Database(_)
        | EnrollmentError::InvalidTtl => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub async fn get_public_login(
    Path(public_token): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<PublicLoginStatus>, StatusCode> {
    let row = load_by_public_token(&state.pool, &public_token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if row.is_expired() {
        return Err(StatusCode::GONE);
    }

    Ok(Json(row.to_public_status()))
}

pub async fn get_public_login_meta(
    Path(public_token): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<PublicLoginStatus>, StatusCode> {
    let row = load_by_public_token(&state.pool, &public_token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if row.is_expired() {
        return Err(StatusCode::GONE);
    }

    let mut status = row.to_public_status();
    status.qr_payload = None;
    Ok(Json(status))
}

pub async fn get_public_login_qr(
    Path(public_token): Path<String>,
    State(state): State<AppState>,
) -> Result<(StatusCode, HeaderMap, Vec<u8>), StatusCode> {
    let row = load_by_public_token(&state.pool, &public_token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if row.is_expired() {
        return Err(StatusCode::GONE);
    }

    let status = row.to_public_status();
    let payload = status.qr_payload.ok_or(StatusCode::NOT_FOUND)?;
    let encoded = payload
        .split_once(',')
        .map(|(_, value)| value)
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;
    let image = STANDARD
        .decode(encoded)
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let image = if let Ok(decoded) = image::load_from_memory(&image) {
        if decoded.width() == 1280 && decoded.height() == 720 {
            let cropped = decoded.crop_imm(545, 250, 190, 190).resize_exact(
                220,
                220,
                image::imageops::FilterType::Nearest,
            );
            let mut output = Cursor::new(Vec::new());
            cropped
                .write_to(&mut output, image::ImageFormat::Png)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            output.into_inner()
        } else {
            image
        }
    } else {
        image
    };

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("image/png"));
    Ok((StatusCode::OK, headers, image))
}

pub(crate) async fn load_by_public_token(
    pool: &MySqlPool,
    public_token: &str,
) -> Result<Option<PublicLoginRow>, sqlx::Error> {
    let token_hash = sha256_hex(public_token);

    sqlx::query_as::<_, PublicLoginRow>(
        "SELECT
            id,
            session_no,
            status,
            qr_payload,
            qr_expires_at,
            detected_character_name,
            detected_server_name,
            identity_verified,
            identity_verify_reason,
            expires_at,
            updated_at
         FROM login_session
         WHERE public_token_hash = ?",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

pub(crate) async fn load_by_id(
    pool: &MySqlPool,
    session_id: i64,
) -> Result<Option<PublicLoginRow>, sqlx::Error> {
    sqlx::query_as::<_, PublicLoginRow>(
        "SELECT
            id,
            session_no,
            status,
            qr_payload,
            qr_expires_at,
            detected_character_name,
            detected_server_name,
            identity_verified,
            identity_verify_reason,
            expires_at,
            updated_at
         FROM login_session
         WHERE id = ?",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
}

fn to_utc(value: NaiveDateTime) -> DateTime<Utc> {
    DateTime::from_naive_utc_and_offset(value, Utc)
}

fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut output = String::with_capacity(64);

    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing into String cannot fail");
    }

    output
}
