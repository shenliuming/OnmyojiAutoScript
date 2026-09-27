use std::{collections::HashMap, sync::Arc};

use tokio::sync::{Mutex, OwnedMutexGuard};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EmulatorLeaseError {
    #[error("emulator is already busy: {0}")]
    Busy(String),
}

#[derive(Clone, Default)]
pub struct EmulatorLeaseManager {
    locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

pub struct EmulatorLease {
    pub emulator_code: String,
    _guard: OwnedMutexGuard<()>,
}

impl EmulatorLeaseManager {
    pub async fn try_acquire(
        &self,
        emulator_code: &str,
    ) -> Result<EmulatorLease, EmulatorLeaseError> {
        let lock = self
            .locks
            .lock()
            .await
            .entry(emulator_code.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();

        let guard = lock
            .try_lock_owned()
            .map_err(|_| EmulatorLeaseError::Busy(emulator_code.to_string()))?;

        Ok(EmulatorLease {
            emulator_code: emulator_code.to_string(),
            _guard: guard,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_one_task_can_hold_the_same_emulator() {
        let manager = EmulatorLeaseManager::default();
        let first = manager.try_acquire("ld-01").await.unwrap();
        assert!(manager.try_acquire("ld-01").await.is_err());

        drop(first);

        assert!(manager.try_acquire("ld-01").await.is_ok());
    }

    #[tokio::test]
    async fn different_emulators_can_run_in_parallel() {
        let manager = EmulatorLeaseManager::default();
        let first = manager.try_acquire("ld-01").await.unwrap();
        let second = manager.try_acquire("ld-02").await.unwrap();

        assert_eq!(first.emulator_code, "ld-01");
        assert_eq!(second.emulator_code, "ld-02");
    }
}
