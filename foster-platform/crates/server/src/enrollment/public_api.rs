use std::fmt::Write as _;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, MySqlPool};

use crate::app::AppState;

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
    pub expires_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl PublicLoginRow {
    pub(crate) fn to_public_status(&self) -> PublicLoginStatus {
        let now = Utc::now().naive_utc();
        let qr_is_current = matches!(self.status.as_str(), "QR_READY" | "WAITING_SCAN")
            && self.qr_expires_at.is_some_and(|expires_at| expires_at > now);

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
            expires_at: to_utc(self.expires_at),
        }
    }

    pub(crate) fn is_expired(&self) -> bool {
        self.expires_at <= Utc::now().naive_utc()
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
