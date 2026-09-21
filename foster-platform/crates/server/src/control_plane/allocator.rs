use std::collections::HashSet;

use sqlx::{mysql::MySqlDatabaseError, MySqlPool};

use super::repository::{
    candidate_emulator_ids, has_reserved_binding, insert_pending_binding, lock_emulator,
    lock_game_account, occupied_slots,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocatedBinding {
    pub binding_id: i64,
    pub emulator_id: i64,
    pub slot_no: i32,
}

#[derive(Debug, thiserror::Error)]
pub enum AllocationError {
    #[error("game account not found")]
    AccountNotFound,
    #[error("game account already has a reserved or active binding")]
    AlreadyBound,
    #[error("no emulator capacity is available")]
    NoCapacity,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct BindingAllocator {
    pool: MySqlPool,
}

impl BindingAllocator {
    pub fn new(pool: MySqlPool) -> Self {
        Self { pool }
    }

    pub async fn allocate_pending(
        &self,
        game_account_id: i64,
    ) -> Result<AllocatedBinding, AllocationError> {
        let mut tx = self.pool.begin().await?;

        let account = lock_game_account(&mut tx, game_account_id)
            .await?
            .ok_or(AllocationError::AccountNotFound)?;

        debug_assert_eq!(account.id, game_account_id);

        if account.active_emulator_id.is_some()
            || has_reserved_binding(&mut tx, game_account_id).await?
        {
            tx.rollback().await?;
            return Err(AllocationError::AlreadyBound);
        }

        let candidate_ids = candidate_emulator_ids(&mut tx).await?;

        for emulator_id in candidate_ids {
            let Some(emulator) = lock_emulator(&mut tx, emulator_id).await? else {
                continue;
            };

            let occupied = occupied_slots(&mut tx, emulator.id).await?;
            if occupied.len() >= emulator.max_account_count as usize {
                continue;
            }

            let occupied: HashSet<i32> = occupied.into_iter().collect();
            let Some(slot_no) =
                (1..=emulator.max_account_count).find(|slot| !occupied.contains(slot))
            else {
                continue;
            };

            let binding_id = match insert_pending_binding(
                &mut tx,
                emulator.id,
                game_account_id,
                slot_no,
            )
            .await
            {
                Ok(id) => id,
                Err(error) if is_duplicate_key(&error) => {
                    tx.rollback().await?;
                    return Err(AllocationError::NoCapacity);
                }
                Err(error) => return Err(AllocationError::Database(error)),
            };

            tx.commit().await?;

            return Ok(AllocatedBinding {
                binding_id,
                emulator_id: emulator.id,
                slot_no,
            });
        }

        tx.rollback().await?;
        Err(AllocationError::NoCapacity)
    }
}

fn is_duplicate_key(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(database_error) => database_error
            .try_downcast_ref::<MySqlDatabaseError>()
            .map(|mysql_error| mysql_error.number() == 1062)
            .unwrap_or(false),
        _ => false,
    }
}
