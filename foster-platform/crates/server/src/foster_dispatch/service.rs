use std::time::Duration;

use foster_domain::{
    AccountIdentity, DetectedIdentity, FosterErrorCode, FosterJobStatus, IdentityDecision,
    IdentityType, ResourceMode, ResourceType, RetryDecision, verify_identity,
};
use foster_protocol::{
    AgentEvent, ExecuteFosterCommand, FosterFailed, FosterStage, FosterStageChanged,
    FosterSucceeded, FosterTargetIdentity, ServerCommand,
};
use sqlx::MySqlPool;
use uuid::Uuid;

use crate::{
    agent_gateway::registry::{AgentRegistry, AgentSendError},
    control_plane::EmulatorLeaseService,
    resource_pool::{ReserveForJobResult, ResourcePoolError, ResourcePoolService},
    scheduler::{SchedulerError, SchedulerService},
};

use super::repository::{
    FosterDispatchTargetRow, FosterIdentityRow, current_job_retry_count, current_job_status,
    job_belongs_to_host, load_dispatch_target, load_identity_rows,
    mark_dispatch_delivery_uncertain, set_job_screenshot_url,
};

#[derive(Debug, thiserror::Error)]
pub enum FosterDispatchError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Scheduler(#[from] SchedulerError),
    #[error(transparent)]
    ResourcePool(#[from] ResourcePoolError),
    #[error("foster job not found")]
    JobNotFound,
    #[error("foster job is not ready for dispatch")]
    InvalidJobState,
    #[error("unsupported resource mode: {0}")]
    UnsupportedResourceMode(String),
    #[error("unsupported resource type: {0}")]
    UnsupportedResourceType(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchFosterResult {
    Dispatched,
    WaitingEmulator,
    RecoveryRequired,
    RejectedIdentity,
    AlreadyHandled,
    WaitingResource,
}

#[derive(Clone)]
pub struct FosterDispatchService {
    pool: MySqlPool,
    scheduler: SchedulerService,
}

impl FosterDispatchService {
    pub fn new(pool: MySqlPool) -> Self {
        Self {
            scheduler: SchedulerService::new(pool.clone()),
            pool,
        }
    }

    pub async fn dispatch_job(
        &self,
        job_id: i64,
        registry: &AgentRegistry,
    ) -> Result<DispatchFosterResult, FosterDispatchError> {
        self.dispatch_job_at(job_id, registry, chrono::Utc::now())
            .await
    }

    pub async fn dispatch_job_at(
        &self,
        job_id: i64,
        registry: &AgentRegistry,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<DispatchFosterResult, FosterDispatchError> {
        let target = load_dispatch_target(&self.pool, job_id)
            .await?
            .ok_or(FosterDispatchError::JobNotFound)?;

        if target.status != "SWITCHING_ACCOUNT" {
            return Ok(DispatchFosterResult::AlreadyHandled);
        }

        let resource_mode = parse_resource_mode(&target.resource_mode)?;

        let identity_rows = load_identity_rows(&self.pool, target.game_account_id).await?;
        let target_identity = build_target_identity(&target, &identity_rows);

        if !identity_is_sufficient(&target_identity) {
            self.scheduler
                .handle_failure(
                    job_id,
                    FosterErrorCode::IdentityMismatch,
                    "insufficient trusted identity hints for foster dispatch",
                    now,
                )
                .await?;
            return Ok(DispatchFosterResult::RejectedIdentity);
        }

        let resource_reservation = if resource_mode == ResourceMode::Platform {
            match ResourcePoolService::new(self.pool.clone())
                .reserve_for_job(job_id, now)
                .await?
            {
                ReserveForJobResult::Reserved(reservation) => Some(reservation),
                ReserveForJobResult::WaitingResource => {
                    return Ok(DispatchFosterResult::WaitingResource);
                }
            }
        } else {
            None
        };

        let (resource_type, provider_alias) = match resource_reservation.as_ref() {
            Some(reservation) => (
                Some(reservation.resource_type),
                Some(reservation.provider_alias.clone()),
            ),
            None => (
                target
                    .resource_type
                    .as_deref()
                    .map(parse_resource_type)
                    .transpose()?,
                None,
            ),
        };

        let command_id = foster_command_id(job_id, target.retry_count);
        let lease_owner = job_id.to_string();
        let lease = EmulatorLeaseService::new(self.pool.clone())
            .try_acquire(
                target.emulator_id,
                command_id,
                "FOSTER",
                &lease_owner,
                Duration::from_secs(15 * 60),
            )
            .await?;

        if lease.is_none() {
            if resource_mode == ResourceMode::Platform {
                ResourcePoolService::new(self.pool.clone())
                    .release_for_job(job_id, "emulator is busy", now)
                    .await?;
            }
            return Ok(DispatchFosterResult::WaitingEmulator);
        }

        let command = ServerCommand::ExecuteFoster(ExecuteFosterCommand {
            job_id,
            attempt: target.retry_count,
            game_account_id: target.game_account_id,
            emulator_code: target.emulator_code,
            resource_mode,
            resource_type,
            provider_alias,
            target_identity,
        });

        let delivered = tokio::time::timeout(
            Duration::from_secs(5),
            registry.send_command_with_id(target.host_id, command_id, command),
        )
        .await;

        match delivered {
            Ok(Ok(())) => Ok(DispatchFosterResult::Dispatched),
            Ok(Err(AgentSendError::Offline | AgentSendError::QueueClosed)) => {
                // Definitively not queued on an active WebSocket.
                EmulatorLeaseService::new(self.pool.clone())
                    .release_owner("FOSTER", &lease_owner)
                    .await?;
                if resource_mode == ResourceMode::Platform {
                    ResourcePoolService::new(self.pool.clone())
                        .release_for_job(job_id, "agent was offline before delivery", now)
                        .await?;
                }
                self.scheduler
                    .handle_failure(
                        job_id,
                        FosterErrorCode::EmulatorOffline,
                        "agent was offline before delivery",
                        now,
                    )
                    .await?;
                Ok(DispatchFosterResult::WaitingEmulator)
            }
            Ok(Err(AgentSendError::DeliveryClosed | AgentSendError::DeliveryFailed(_)))
            | Err(_) => {
                // The Agent may already have received the command. Never
                // release the resource allocation or blindly retry.
                let marked =
                    mark_dispatch_delivery_uncertain(&self.pool, job_id, target.retry_count)
                        .await?;
                Ok(if marked {
                    DispatchFosterResult::RecoveryRequired
                } else {
                    DispatchFosterResult::AlreadyHandled
                })
            }
        }
    }

    pub async fn process_agent_event(
        &self,
        host_id: i64,
        event: &AgentEvent,
    ) -> Result<(), FosterDispatchError> {
        match event {
            AgentEvent::FosterStageChanged(event) => {
                self.process_stage(host_id, event).await?;
            }
            AgentEvent::FosterSucceeded(event) => {
                self.process_success(host_id, event).await?;
            }
            AgentEvent::FosterFailed(event) => {
                self.process_failure(host_id, event).await?;
            }
            _ => {}
        }

        Ok(())
    }

    async fn process_stage(
        &self,
        host_id: i64,
        event: &FosterStageChanged,
    ) -> Result<(), FosterDispatchError> {
        if !job_belongs_to_host(&self.pool, event.job_id, host_id).await? {
            return Ok(());
        }
        if !self.is_current_attempt(event.job_id, event.attempt).await? {
            return Ok(());
        }

        EmulatorLeaseService::new(self.pool.clone())
            .release_owner("FOSTER", &event.job_id.to_string())
            .await?;

        EmulatorLeaseService::new(self.pool.clone())
            .release_owner("FOSTER", &event.job_id.to_string())
            .await?;

        let Some(status) = current_job_status(&self.pool, event.job_id).await? else {
            return Ok(());
        };

        if is_terminal_status(&status) {
            return Ok(());
        }

        match event.stage {
            FosterStage::SwitchingAccount => {}
            FosterStage::VerifyingAccount => {
                if status == "SWITCHING_ACCOUNT" {
                    let _ = self
                        .scheduler
                        .transition_job(
                            event.job_id,
                            FosterJobStatus::SwitchingAccount,
                            FosterJobStatus::VerifyingAccount,
                            event.occurred_at,
                        )
                        .await?;
                }
            }
            FosterStage::Running => {
                if status == "SWITCHING_ACCOUNT" {
                    let _ = self
                        .scheduler
                        .transition_job(
                            event.job_id,
                            FosterJobStatus::SwitchingAccount,
                            FosterJobStatus::VerifyingAccount,
                            event.occurred_at,
                        )
                        .await?;
                }

                let current = current_job_status(&self.pool, event.job_id)
                    .await?
                    .unwrap_or_default();
                if current == "VERIFYING_ACCOUNT" {
                    let _ = self
                        .scheduler
                        .transition_job(
                            event.job_id,
                            FosterJobStatus::VerifyingAccount,
                            FosterJobStatus::Running,
                            event.occurred_at,
                        )
                        .await?;
                }
            }
        }

        Ok(())
    }

    async fn process_success(
        &self,
        host_id: i64,
        event: &FosterSucceeded,
    ) -> Result<(), FosterDispatchError> {
        if !job_belongs_to_host(&self.pool, event.job_id, host_id).await? {
            return Ok(());
        }
        if !self.is_current_attempt(event.job_id, event.attempt).await? {
            return Ok(());
        }

        let Some(status) = current_job_status(&self.pool, event.job_id).await? else {
            return Ok(());
        };

        if is_terminal_status(&status) {
            return Ok(());
        }
        if !matches!(
            status.as_str(),
            "SWITCHING_ACCOUNT" | "VERIFYING_ACCOUNT" | "RUNNING"
        ) {
            return Ok(());
        }

        let target = load_dispatch_target(&self.pool, event.job_id)
            .await?
            .ok_or(FosterDispatchError::JobNotFound)?;
        let identity_rows = load_identity_rows(&self.pool, target.game_account_id).await?;
        let stored = to_account_identities(&identity_rows);
        let detected = DetectedIdentity {
            masked_account: event.detected_identity.masked_account.clone(),
            character_name: event.detected_identity.character_name.clone(),
            server_name: event.detected_identity.server_name.clone(),
            game_uid: event.detected_identity.game_uid.clone(),
            ocr_aliases: Vec::new(),
        };

        if !matches!(
            verify_identity(&stored, &detected),
            IdentityDecision::Verified { .. }
        ) {
            if target.resource_mode == "PLATFORM" {
                ResourcePoolService::new(self.pool.clone())
                    .release_for_job(
                        event.job_id,
                        "identity mismatch after OAS execution",
                        event.completed_at,
                    )
                    .await?;
            }
            self.scheduler
                .handle_failure(
                    event.job_id,
                    FosterErrorCode::IdentityMismatch,
                    "OAS detected identity did not verify against trusted account identity",
                    event.completed_at,
                )
                .await?;
            set_job_screenshot_url(&self.pool, event.job_id, event.screenshot_url.as_deref())
                .await?;
            return Ok(());
        }

        self.ensure_running(event.job_id, event.completed_at)
            .await?;

        if target.resource_mode == "PLATFORM" {
            ResourcePoolService::new(self.pool.clone())
                .confirm_for_job(event.job_id, event.completed_at, event.remaining_seconds)
                .await?;
        }

        let current = current_job_status(&self.pool, event.job_id)
            .await?
            .unwrap_or_default();
        if current == "RUNNING" {
            self.scheduler
                .complete_success(event.job_id, event.completed_at, event.remaining_seconds)
                .await?;
        }

        set_job_screenshot_url(&self.pool, event.job_id, event.screenshot_url.as_deref()).await?;

        Ok(())
    }

    async fn process_failure(
        &self,
        host_id: i64,
        event: &FosterFailed,
    ) -> Result<(), FosterDispatchError> {
        if !job_belongs_to_host(&self.pool, event.job_id, host_id).await? {
            return Ok(());
        }
        if !self.is_current_attempt(event.job_id, event.attempt).await? {
            return Ok(());
        }

        let Some(status) = current_job_status(&self.pool, event.job_id).await? else {
            return Ok(());
        };

        if is_terminal_status(&status) {
            return Ok(());
        }
        if !matches!(
            status.as_str(),
            "SWITCHING_ACCOUNT" | "VERIFYING_ACCOUNT" | "RUNNING"
        ) {
            return Ok(());
        }

        let target = load_dispatch_target(&self.pool, event.job_id)
            .await?
            .ok_or(FosterDispatchError::JobNotFound)?;

        if target.resource_mode == "PLATFORM" {
            let resource_pool = ResourcePoolService::new(self.pool.clone());
            let released = resource_pool
                .release_for_job(event.job_id, &event.message, event.failed_at)
                .await?;

            if let Some(released) = released {
                match event.error_code {
                    FosterErrorCode::ProviderNotFound => {
                        resource_pool
                            .mark_provider_not_found(
                                target.game_account_id,
                                released.provider_account_id,
                                event.failed_at,
                            )
                            .await?;
                    }
                    FosterErrorCode::NoSlot => {
                        resource_pool
                            .mark_no_slot(released.resource_cycle_id)
                            .await?;
                    }
                    _ => {}
                }
            }
        }

        let _decision: RetryDecision = self
            .scheduler
            .handle_failure(
                event.job_id,
                event.error_code,
                &event.message,
                event.failed_at,
            )
            .await?;

        set_job_screenshot_url(&self.pool, event.job_id, event.screenshot_url.as_deref()).await?;

        Ok(())
    }

    async fn is_current_attempt(
        &self,
        job_id: i64,
        attempt: i32,
    ) -> Result<bool, FosterDispatchError> {
        Ok(current_job_retry_count(&self.pool, job_id).await? == Some(attempt))
    }

    async fn ensure_running(
        &self,
        job_id: i64,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), FosterDispatchError> {
        let status = current_job_status(&self.pool, job_id)
            .await?
            .ok_or(FosterDispatchError::JobNotFound)?;

        if status == "SWITCHING_ACCOUNT" {
            let _ = self
                .scheduler
                .transition_job(
                    job_id,
                    FosterJobStatus::SwitchingAccount,
                    FosterJobStatus::VerifyingAccount,
                    at,
                )
                .await?;
        }

        let status = current_job_status(&self.pool, job_id)
            .await?
            .ok_or(FosterDispatchError::JobNotFound)?;
        if status == "VERIFYING_ACCOUNT" {
            let _ = self
                .scheduler
                .transition_job(
                    job_id,
                    FosterJobStatus::VerifyingAccount,
                    FosterJobStatus::Running,
                    at,
                )
                .await?;
        }

        Ok(())
    }
}

fn parse_resource_mode(value: &str) -> Result<ResourceMode, FosterDispatchError> {
    match value {
        "USER_FRIEND" => Ok(ResourceMode::UserFriend),
        "PLATFORM" => Ok(ResourceMode::Platform),
        other => Err(FosterDispatchError::UnsupportedResourceMode(
            other.to_string(),
        )),
    }
}

fn parse_resource_type(value: &str) -> Result<ResourceType, FosterDispatchError> {
    match value {
        "FISH" | "DOUYU" => Ok(ResourceType::Fish),
        "TAIKO_JADE" | "JADE" | "TAIKO" => Ok(ResourceType::TaikoJade),
        other => Err(FosterDispatchError::UnsupportedResourceType(
            other.to_string(),
        )),
    }
}

fn build_target_identity(
    target: &FosterDispatchTargetRow,
    rows: &[FosterIdentityRow],
) -> FosterTargetIdentity {
    let masked_account = first_identity(rows, "MASKED_ACCOUNT");
    let account_aliases = rows
        .iter()
        .filter(|row| row.identity_type == "OCR_ALIAS")
        .map(|row| row.identity_value.clone())
        .collect();

    FosterTargetIdentity {
        masked_account,
        account_aliases,
        character_name: target
            .character_name
            .clone()
            .or_else(|| first_identity(rows, "CHARACTER_NAME")),
        server_name: target
            .server_name
            .clone()
            .or_else(|| first_identity(rows, "SERVER_NAME")),
        game_uid: target
            .game_uid
            .clone()
            .or_else(|| first_identity(rows, "GAME_UID")),
    }
}

fn first_identity(rows: &[FosterIdentityRow], identity_type: &str) -> Option<String> {
    rows.iter()
        .find(|row| row.identity_type == identity_type)
        .map(|row| row.identity_value.clone())
}

fn identity_is_sufficient(identity: &FosterTargetIdentity) -> bool {
    identity.game_uid.is_some()
        || (identity.character_name.is_some() && identity.server_name.is_some())
        || ((identity.masked_account.is_some() || !identity.account_aliases.is_empty())
            && (identity.character_name.is_some() || identity.server_name.is_some()))
}

fn to_account_identities(rows: &[FosterIdentityRow]) -> Vec<AccountIdentity> {
    rows.iter()
        .filter_map(|row| {
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
                value: row.identity_value.clone(),
                normalized_value: row.normalized_value.clone(),
                confidence: row.confidence.clamp(0, 100) as u8,
            })
        })
        .collect()
}

fn is_terminal_status(status: &str) -> bool {
    matches!(
        status,
        "SUCCESS" | "FAILED" | "IDENTITY_MISMATCH" | "CANCELLED"
    )
}

const FOSTER_COMMAND_NAMESPACE: Uuid = Uuid::from_bytes([
    0x1f, 0x82, 0x7d, 0x4d, 0x41, 0x6e, 0x47, 0x9a, 0xa2, 0x44, 0x8a, 0x67, 0x11, 0x53, 0xc2, 0x90,
]);

pub fn foster_command_id(job_id: i64, attempt: i32) -> Uuid {
    let key = format!("foster:{job_id}:{attempt}");
    Uuid::new_v5(&FOSTER_COMMAND_NAMESPACE, key.as_bytes())
}
