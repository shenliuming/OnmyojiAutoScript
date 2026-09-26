use std::time::Duration;

use chrono::Utc;
use sqlx::{MySql, MySqlPool, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AcquiredEmulatorLease {
    pub emulator_id: i64,
    pub lease_token: Uuid,
}

#[derive(Debug, sqlx::FromRow)]
struct EmulatorLeaseCandidate {
    id: i64,
    lifecycle_status: String,
    occupancy_status: String,
}

#[derive(Clone)]
pub struct EmulatorLeaseService {
    pool: MySqlPool,
}

impl EmulatorLeaseService {
    pub fn new(pool: MySqlPool) -> Self {
        Self { pool }
    }

    pub async fn try_acquire(
        &self,
        emulator_id: i64,
        command_id: Uuid,
        owner_type: &str,
        owner_key: &str,
        ttl: Duration,
    ) -> Result<Option<AcquiredEmulatorLease>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let lease = try_acquire_emulator_lease(
            &mut tx,
            emulator_id,
            command_id,
            owner_type,
            owner_key,
            ttl,
        )
        .await?;
        tx.commit().await?;
        Ok(lease)
    }

    pub async fn release_owner(
        &self,
        owner_type: &str,
        owner_key: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "DELETE FROM emulator_lease
             WHERE owner_type = ?
               AND owner_key = ?",
        )
        .bind(owner_type)
        .bind(owner_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

pub async fn try_acquire_emulator_lease(
    tx: &mut Transaction<'_, MySql>,
    emulator_id: i64,
    command_id: Uuid,
    owner_type: &str,
    owner_key: &str,
    ttl: Duration,
) -> Result<Option<AcquiredEmulatorLease>, sqlx::Error> {
    let Some(candidate) = sqlx::query_as::<_, EmulatorLeaseCandidate>(
        "SELECT id, lifecycle_status, occupancy_status
         FROM emulator_instance
         WHERE id = ?
         FOR UPDATE",
    )
    .bind(emulator_id)
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(None);
    };

    sqlx::query(
        "DELETE FROM emulator_lease
         WHERE emulator_id = ?
           AND expires_at <= NOW(3)",
    )
    .bind(candidate.id)
    .execute(&mut **tx)
    .await?;

    if candidate.lifecycle_status != "READY" || candidate.occupancy_status != "IDLE" {
        return Ok(None);
    }

    let existing: Option<(String, String, String)> = sqlx::query_as(
        "SELECT lease_token, owner_type, owner_key
         FROM emulator_lease
         WHERE emulator_id = ?",
    )
    .bind(candidate.id)
    .fetch_optional(&mut **tx)
    .await?;

    if let Some((token, existing_type, existing_key)) = existing {
        if existing_type == owner_type && existing_key == owner_key {
            if let Ok(lease_token) = Uuid::parse_str(&token) {
                return Ok(Some(AcquiredEmulatorLease {
                    emulator_id: candidate.id,
                    lease_token,
                }));
            }
        }
        return Ok(None);
    }

    let expires_at = Utc::now()
        + chrono::Duration::from_std(ttl)
            .unwrap_or_else(|_| chrono::Duration::minutes(15));
    let lease_token = command_id;

    sqlx::query(
        "INSERT INTO emulator_lease(
            emulator_id, lease_token, owner_type, owner_key,
            command_id, expires_at
         )
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(candidate.id)
    .bind(lease_token.to_string())
    .bind(owner_type)
    .bind(owner_key)
    .bind(command_id.to_string())
    .bind(expires_at.naive_utc())
    .execute(&mut **tx)
    .await?;

    Ok(Some(AcquiredEmulatorLease {
        emulator_id: candidate.id,
        lease_token,
    }))
}
