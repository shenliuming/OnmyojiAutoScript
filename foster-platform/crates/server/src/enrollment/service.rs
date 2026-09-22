use std::{fmt::Write as _, time::Duration};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use foster_domain::{
    AccountIdentity, DetectedIdentity, IdentityDecision, IdentityType, normalize_identity,
    verify_identity,
};
use foster_protocol::AgentEvent;
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use sqlx::MySqlPool;
use uuid::Uuid;

use crate::control_plane::{AllocationError, BindingAllocator};

use super::{
    model::CreatedLoginSession,
    repository::{
        NewLoginSession, TrustedIdentityRow, activate_game_account, activate_pending_binding,
        complete_login_session, insert_enrollment_identity, insert_login_session,
        load_trusted_identities, lock_binding, lock_game_account,
        lock_login_session_by_control_hash, mark_login_failed, mark_login_identity_detected,
        mark_login_preparing, mark_login_qr_expired, mark_login_qr_ready, release_pending_binding,
    },
};

#[derive(Debug, thiserror::Error)]
pub enum EnrollmentError {
    #[error(transparent)]
    Allocation(#[from] AllocationError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("login session ttl is too large")]
    InvalidTtl,
    #[error("login session not found")]
    LoginSessionNotFound,
    #[error("login session expired")]
    LoginSessionExpired,
    #[error("login session is not ready for confirmation")]
    InvalidLoginState,
    #[error("detected account identity is not sufficiently verified")]
    IdentityRejected,
    #[error("login session binding does not match")]
    BindingMismatch,
    #[error("game account is already active on another emulator")]
    AccountBindingConflict,
}

#[derive(Clone)]
pub struct EnrollmentService {
    pool: MySqlPool,
    allocator: BindingAllocator,
}

impl EnrollmentService {
    pub fn new(pool: MySqlPool) -> Self {
        Self {
            allocator: BindingAllocator::new(pool.clone()),
            pool,
        }
    }

    pub async fn process_agent_event(
        &self,
        host_id: i64,
        event: &AgentEvent,
    ) -> Result<(), EnrollmentError> {
        match event {
            AgentEvent::LoginPreparing(event) => {
                mark_login_preparing(&self.pool, host_id, &event.session_no).await?;
            }
            AgentEvent::LoginQrReady(event) => {
                mark_login_qr_ready(
                    &self.pool,
                    host_id,
                    &event.session_no,
                    &event.qr_payload,
                    event.expires_at,
                )
                .await?;
            }
            AgentEvent::LoginQrExpired(event) => {
                mark_login_qr_expired(&self.pool, host_id, &event.session_no).await?;
            }
            AgentEvent::LoginIdentityDetected(event) => {
                mark_login_identity_detected(
                    &self.pool,
                    host_id,
                    &event.session_no,
                    event.masked_account.as_deref(),
                    event.character_name.as_deref(),
                    event.server_name.as_deref(),
                    event.game_uid.as_deref(),
                )
                .await?;
            }
            AgentEvent::LoginFailed(event) => {
                let reason = format!("{}: {}", event.code, event.message);
                mark_login_failed(&self.pool, host_id, &event.session_no, &reason).await?;
            }
            AgentEvent::Hello(_)
            | AgentEvent::Heartbeat(_)
            | AgentEvent::EmulatorSnapshot(_)
            | AgentEvent::Pong(_) => {}
        }

        Ok(())
    }

    pub async fn confirm_login_session(
        &self,
        control_token: &str,
    ) -> Result<String, EnrollmentError> {
        let control_token_hash = sha256_hex(control_token);
        let mut tx = self.pool.begin().await?;

        let session = lock_login_session_by_control_hash(&mut tx, &control_token_hash)
            .await?
            .ok_or(EnrollmentError::LoginSessionNotFound)?;

        if session.status == "SUCCESS" {
            tx.commit().await?;
            return Ok(session.session_no);
        }

        if session.expires_at <= Utc::now().naive_utc() {
            return Err(EnrollmentError::LoginSessionExpired);
        }

        if session.status != "VERIFYING_ACCOUNT" {
            return Err(EnrollmentError::InvalidLoginState);
        }

        let binding = lock_binding(&mut tx, session.binding_id)
            .await?
            .ok_or(EnrollmentError::BindingMismatch)?;

        if binding.status != "PENDING"
            || binding.game_account_id != session.game_account_id
            || binding.emulator_id != session.emulator_id
        {
            return Err(EnrollmentError::BindingMismatch);
        }

        let account = lock_game_account(&mut tx, session.game_account_id)
            .await?
            .ok_or(EnrollmentError::BindingMismatch)?;

        if account
            .active_emulator_id
            .is_some_and(|emulator_id| emulator_id != session.emulator_id)
        {
            return Err(EnrollmentError::AccountBindingConflict);
        }

        let detected = DetectedIdentity {
            masked_account: session.detected_masked_account.clone(),
            character_name: session.detected_character_name.clone(),
            server_name: session.detected_server_name.clone(),
            game_uid: session.detected_game_uid.clone(),
            ocr_aliases: Vec::new(),
        };

        let trusted_rows = load_trusted_identities(&mut tx, session.game_account_id).await?;

        if trusted_rows.is_empty() {
            if !first_enrollment_has_strong_identity(&detected) {
                return Err(EnrollmentError::IdentityRejected);
            }

            persist_first_enrollment_identities(&mut tx, session.game_account_id, &detected).await?;
        } else {
            let trusted = trusted_rows
                .into_iter()
                .filter_map(to_account_identity)
                .collect::<Vec<_>>();

            if !matches!(
                verify_identity(&trusted, &detected),
                IdentityDecision::Verified { .. }
            ) {
                return Err(EnrollmentError::IdentityRejected);
            }
        }

        if !activate_pending_binding(&mut tx, binding.id).await? {
            return Err(EnrollmentError::BindingMismatch);
        }

        activate_game_account(
            &mut tx,
            account.id,
            session.emulator_id,
            detected.character_name.as_deref(),
            detected.server_name.as_deref(),
            detected.game_uid.as_deref(),
        )
        .await?;

        if !complete_login_session(&mut tx, session.id).await? {
            return Err(EnrollmentError::InvalidLoginState);
        }

        tx.commit().await?;
        Ok(session.session_no)
    }

    pub async fn create_login_session(
        &self,
        game_account_id: i64,
        ttl: Duration,
    ) -> Result<CreatedLoginSession, EnrollmentError> {
        let binding = self.allocator.allocate_pending(game_account_id).await?;

        let public_token = random_token();
        let control_token = random_token();
        let public_token_hash = sha256_hex(&public_token);
        let control_token_hash = sha256_hex(&control_token);

        let ttl = chrono::Duration::from_std(ttl).map_err(|_| EnrollmentError::InvalidTtl)?;
        let expires_at = Utc::now() + ttl;
        let session_no = format!("LOGIN-{}", Uuid::new_v4());

        let session_id = match insert_login_session(
            &self.pool,
            NewLoginSession {
                session_no: &session_no,
                game_account_id,
                binding_id: binding.binding_id,
                emulator_id: binding.emulator_id,
                public_token_hash: &public_token_hash,
                control_token_hash: &control_token_hash,
                expires_at,
            },
        )
        .await
        {
            Ok(session_id) => session_id,
            Err(error) => {
                release_pending_binding(&self.pool, binding.binding_id).await?;
                return Err(EnrollmentError::Database(error));
            }
        };

        Ok(CreatedLoginSession {
            session_id,
            session_no,
            public_token,
            control_token,
            emulator_id: binding.emulator_id,
            binding_id: binding.binding_id,
        })
    }
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut output = String::with_capacity(64);

    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing into String cannot fail");
    }

    output
}


fn first_enrollment_has_strong_identity(detected: &DetectedIdentity) -> bool {
    has_text(detected.game_uid.as_deref())
        || (has_text(detected.character_name.as_deref())
            && has_text(detected.server_name.as_deref()))
}

fn has_text(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

fn to_account_identity(row: TrustedIdentityRow) -> Option<AccountIdentity> {
    let kind = match row.identity_type.as_str() {
        "MASKED_ACCOUNT" => IdentityType::MaskedAccount,
        "OCR_ALIAS" => IdentityType::OcrAlias,
        "CHARACTER_NAME" => IdentityType::CharacterName,
        "SERVER_NAME" => IdentityType::ServerName,
        "GAME_UID" => IdentityType::GameUid,
        _ => return None,
    };

    Some(AccountIdentity {
        kind,
        value: row.identity_value,
        normalized_value: row.normalized_value,
        confidence: row.confidence.clamp(0, u8::MAX as i32) as u8,
    })
}

async fn persist_first_enrollment_identities(
    tx: &mut sqlx::Transaction<'_, sqlx::MySql>,
    game_account_id: i64,
    detected: &DetectedIdentity,
) -> Result<(), sqlx::Error> {
    let values = [
        (IdentityType::MaskedAccount, detected.masked_account.as_deref()),
        (IdentityType::CharacterName, detected.character_name.as_deref()),
        (IdentityType::ServerName, detected.server_name.as_deref()),
        (IdentityType::GameUid, detected.game_uid.as_deref()),
    ];

    for (kind, value) in values {
        let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
            continue;
        };

        insert_enrollment_identity(
            tx,
            game_account_id,
            identity_type_name(kind),
            value,
            &normalize_identity(kind, value),
        )
        .await?;
    }

    Ok(())
}

fn identity_type_name(kind: IdentityType) -> &'static str {
    match kind {
        IdentityType::MaskedAccount => "MASKED_ACCOUNT",
        IdentityType::OcrAlias => "OCR_ALIAS",
        IdentityType::CharacterName => "CHARACTER_NAME",
        IdentityType::ServerName => "SERVER_NAME",
        IdentityType::GameUid => "GAME_UID",
    }
}
