use chrono::{DateTime, Utc};
use foster_domain::{ResourceMode, ResourceType};
use sqlx::MySqlPool;

use super::repository::{
    AllocationRow, attach_allocation_to_job, confirm_allocation, finish_allocation,
    insert_reserved_allocation, list_expirable_job_ids, list_resource_candidates,
    load_live_allocation, lock_releasable_allocation, lock_resource_job, mark_cycle_full,
    mark_expired_cycles, mark_friend_binding_suspect, release_cycle_slot, set_job_waiting_resource,
    try_reserve_cycle,
};

#[derive(Debug, thiserror::Error)]
pub enum ResourcePoolError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("foster job not found")]
    JobNotFound,
    #[error("job is not a PLATFORM foster job")]
    NotPlatformJob,
    #[error("job is not ready for resource allocation: {0}")]
    InvalidJobState(String),
    #[error("PLATFORM subscription has no resource type")]
    MissingResourceType,
    #[error("unsupported resource type: {0}")]
    UnsupportedResourceType(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceReservation {
    pub allocation_id: i64,
    pub provider_account_id: i64,
    pub provider_alias: String,
    pub resource_cycle_id: i64,
    pub resource_type: ResourceType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveForJobResult {
    Reserved(ResourceReservation),
    WaitingResource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleasedAllocation {
    pub allocation_id: i64,
    pub provider_account_id: i64,
    pub resource_cycle_id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResourceReapReport {
    pub expired_allocations: usize,
    pub ended_cycles: u64,
}

#[derive(Clone)]
pub struct ResourcePoolService {
    pool: MySqlPool,
    min_remaining_minutes: i64,
    retry_seconds: i64,
}

impl ResourcePoolService {
    pub fn new(pool: MySqlPool) -> Self {
        let min_remaining_minutes = std::env::var("FOSTER_RESOURCE_MIN_REMAINING_MINUTES")
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|value| *value >= 0)
            .unwrap_or(330);

        let retry_seconds = std::env::var("FOSTER_RESOURCE_RETRY_SECONDS")
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(60);

        Self {
            pool,
            min_remaining_minutes,
            retry_seconds,
        }
    }

    pub fn with_min_remaining_minutes(mut self, minutes: i64) -> Self {
        self.min_remaining_minutes = minutes.max(0);
        self
    }

    pub fn with_retry_seconds(mut self, seconds: i64) -> Self {
        self.retry_seconds = seconds.max(1);
        self
    }

    pub async fn reserve_for_job(
        &self,
        job_id: i64,
        now: DateTime<Utc>,
    ) -> Result<ReserveForJobResult, ResourcePoolError> {
        let mut tx = self.pool.begin().await?;

        let job = lock_resource_job(&mut tx, job_id)
            .await?
            .ok_or(ResourcePoolError::JobNotFound)?;

        if parse_resource_mode(&job.resource_mode) != ResourceMode::Platform {
            return Err(ResourcePoolError::NotPlatformJob);
        }
        if job.status != "SWITCHING_ACCOUNT" {
            return Err(ResourcePoolError::InvalidJobState(job.status));
        }

        if let Some(allocation) = load_live_allocation(&mut tx, job_id).await? {
            let reservation = reservation_from_row(allocation)?;
            tx.commit().await?;
            return Ok(ReserveForJobResult::Reserved(reservation));
        }

        let resource_type = job
            .resource_type
            .as_deref()
            .ok_or(ResourcePoolError::MissingResourceType)
            .and_then(parse_resource_type)?;
        let resource_type_name = resource_type_name(resource_type);
        let min_end_at = now + chrono::Duration::minutes(self.min_remaining_minutes);

        let candidates = list_resource_candidates(
            &mut tx,
            job.game_account_id,
            resource_type_name,
            now,
            min_end_at,
        )
        .await?;

        for candidate in candidates {
            if !try_reserve_cycle(&mut tx, candidate.resource_cycle_id, min_end_at).await? {
                continue;
            }

            let allocation_id = insert_reserved_allocation(
                &mut tx,
                job.id,
                candidate.resource_cycle_id,
                candidate.provider_account_id,
                now,
            )
            .await?;

            attach_allocation_to_job(
                &mut tx,
                job.id,
                candidate.provider_account_id,
                candidate.resource_cycle_id,
                allocation_id,
            )
            .await?;

            tx.commit().await?;
            return Ok(ReserveForJobResult::Reserved(ResourceReservation {
                allocation_id,
                provider_account_id: candidate.provider_account_id,
                provider_alias: candidate.provider_alias,
                resource_cycle_id: candidate.resource_cycle_id,
                resource_type,
            }));
        }

        set_job_waiting_resource(
            &mut tx,
            job.id,
            "no eligible platform resource is currently available",
            now + chrono::Duration::seconds(self.retry_seconds),
        )
        .await?;
        tx.commit().await?;

        Ok(ReserveForJobResult::WaitingResource)
    }

    pub async fn confirm_for_job(
        &self,
        job_id: i64,
        completed_at: DateTime<Utc>,
        remaining_seconds: Option<i64>,
    ) -> Result<bool, ResourcePoolError> {
        let mut tx = self.pool.begin().await?;
        let job = lock_resource_job(&mut tx, job_id)
            .await?
            .ok_or(ResourcePoolError::JobNotFound)?;

        if parse_resource_mode(&job.resource_mode) != ResourceMode::Platform {
            tx.commit().await?;
            return Ok(false);
        }

        let Some(allocation) = load_live_allocation(&mut tx, job_id).await? else {
            tx.commit().await?;
            return Ok(false);
        };

        if allocation.status == "CONFIRMED" {
            tx.commit().await?;
            return Ok(true);
        }

        let duration_seconds = remaining_seconds
            .filter(|seconds| *seconds > 0)
            .unwrap_or_else(|| i64::from(job.interval_minutes) * 60);
        let occupied_until = completed_at + chrono::Duration::seconds(duration_seconds);

        let confirmed =
            confirm_allocation(&mut tx, allocation.id, completed_at, occupied_until).await?;
        tx.commit().await?;

        Ok(confirmed)
    }

    pub async fn release_for_job(
        &self,
        job_id: i64,
        reason: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<ReleasedAllocation>, ResourcePoolError> {
        self.finish_for_job(job_id, "RELEASED", reason, now).await
    }

    async fn expire_for_job(
        &self,
        job_id: i64,
        now: DateTime<Utc>,
    ) -> Result<Option<ReleasedAllocation>, ResourcePoolError> {
        self.finish_for_job(job_id, "EXPIRED", "allocation expired", now)
            .await
    }

    async fn finish_for_job(
        &self,
        job_id: i64,
        next_status: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<ReleasedAllocation>, ResourcePoolError> {
        let mut tx = self.pool.begin().await?;

        let Some(allocation) = lock_releasable_allocation(&mut tx, job_id).await? else {
            tx.commit().await?;
            return Ok(None);
        };

        if !finish_allocation(&mut tx, allocation.id, next_status, now, reason).await? {
            tx.commit().await?;
            return Ok(None);
        }

        release_cycle_slot(&mut tx, allocation.resource_cycle_id, now).await?;
        tx.commit().await?;

        Ok(Some(ReleasedAllocation {
            allocation_id: allocation.id,
            provider_account_id: allocation.provider_account_id,
            resource_cycle_id: allocation.resource_cycle_id,
        }))
    }

    pub async fn mark_no_slot(&self, resource_cycle_id: i64) -> Result<(), ResourcePoolError> {
        let mut tx = self.pool.begin().await?;
        mark_cycle_full(&mut tx, resource_cycle_id).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn mark_provider_not_found(
        &self,
        game_account_id: i64,
        provider_account_id: i64,
        now: DateTime<Utc>,
    ) -> Result<(), ResourcePoolError> {
        let mut tx = self.pool.begin().await?;
        mark_friend_binding_suspect(
            &mut tx,
            game_account_id,
            provider_account_id,
            "PROVIDER_NOT_FOUND",
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn reap(&self, now: DateTime<Utc>) -> Result<ResourceReapReport, ResourcePoolError> {
        let mut tx = self.pool.begin().await?;
        let job_ids = list_expirable_job_ids(&mut tx, now).await?;
        tx.commit().await?;

        let mut expired_allocations = 0;
        for job_id in job_ids {
            if self.expire_for_job(job_id, now).await?.is_some() {
                expired_allocations += 1;
            }
        }

        let mut tx = self.pool.begin().await?;
        let ended_cycles = mark_expired_cycles(&mut tx, now).await?;
        tx.commit().await?;

        Ok(ResourceReapReport {
            expired_allocations,
            ended_cycles,
        })
    }
}

fn reservation_from_row(
    allocation: AllocationRow,
) -> Result<ResourceReservation, ResourcePoolError> {
    Ok(ResourceReservation {
        allocation_id: allocation.id,
        provider_account_id: allocation.provider_account_id,
        provider_alias: allocation.provider_alias,
        resource_cycle_id: allocation.resource_cycle_id,
        resource_type: parse_resource_type(&allocation.resource_type)?,
    })
}

fn parse_resource_mode(value: &str) -> ResourceMode {
    match value {
        "PLATFORM" => ResourceMode::Platform,
        _ => ResourceMode::UserFriend,
    }
}

fn parse_resource_type(value: &str) -> Result<ResourceType, ResourcePoolError> {
    match value {
        "FISH" | "DOUYU" => Ok(ResourceType::Fish),
        "TAIKO_JADE" | "JADE" | "TAIKO" => Ok(ResourceType::TaikoJade),
        other => Err(ResourcePoolError::UnsupportedResourceType(
            other.to_string(),
        )),
    }
}

fn resource_type_name(resource_type: ResourceType) -> &'static str {
    match resource_type {
        ResourceType::Fish => "FISH",
        ResourceType::TaikoJade => "TAIKO_JADE",
    }
}
