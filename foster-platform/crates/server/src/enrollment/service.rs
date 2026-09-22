use std::{fmt::Write as _, time::Duration};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use foster_protocol::AgentEvent;
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use sqlx::MySqlPool;
use uuid::Uuid;

use crate::control_plane::{AllocationError, BindingAllocator};

use super::{
    model::CreatedLoginSession,
    repository::{
        NewLoginSession, insert_login_session, mark_login_failed, mark_login_identity_detected,
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
