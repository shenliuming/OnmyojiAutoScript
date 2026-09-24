use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::{MumuCli, MumuError, MumuInstanceInfo};

#[async_trait]
pub trait MumuInstanceSource: Send + Sync + 'static {
    async fn info_all(&self) -> Result<Vec<MumuInstanceInfo>, MumuError>;
}

#[async_trait]
impl MumuInstanceSource for MumuCli {
    async fn info_all(&self) -> Result<Vec<MumuInstanceInfo>, MumuError> {
        MumuCli::info_all(self).await
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LeaseError {
    #[error("no idle MuMu instance is available")]
    NoIdleInstance,
}

#[derive(Clone, Default)]
pub struct InstanceLeaseManager {
    locks: Arc<Mutex<HashMap<u32, Arc<Mutex<()>>>>>,
}

pub struct InstanceLease {
    pub instance: MumuInstanceInfo,
    pub adb_serial: String,
    _guard: OwnedMutexGuard<()>,
}

impl InstanceLease {
    /// Release immediately; dropping the lease also releases it, including on task cancellation.
    pub async fn release(self) {
        drop(self);
    }
}

impl InstanceLeaseManager {
    pub async fn acquire(
        &self,
        instances: &[MumuInstanceInfo],
        active_command_instances: &[u32],
    ) -> Result<InstanceLease, LeaseError> {
        let mut candidates = instances.iter().collect::<Vec<_>>();
        candidates.sort_by_key(|instance| instance.index);

        for instance in candidates {
            if active_command_instances.contains(&instance.index)
                || !instance.is_android_started
                || !instance.is_process_started
                || instance.player_state.as_deref() != Some("start_finished")
            {
                continue;
            }
            let (Some(host), Some(port)) = (&instance.adb_host_ip, instance.adb_port) else {
                continue;
            };
            if host.trim().is_empty() || port == 0 {
                continue;
            }

            let lock = self
                .locks
                .lock()
                .await
                .entry(instance.index)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone();
            if let Ok(guard) = lock.try_lock_owned() {
                return Ok(InstanceLease {
                    instance: instance.clone(),
                    adb_serial: format!("{host}:{port}"),
                    _guard: guard,
                });
            }
        }

        Err(LeaseError::NoIdleInstance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(index: u32) -> MumuInstanceInfo {
        MumuInstanceInfo {
            index,
            name: format!("MuMu-{index}"),
            adb_host_ip: Some("127.0.0.1".into()),
            adb_port: Some(16384 + index as u16 * 32),
            is_android_started: true,
            is_process_started: true,
            player_state: Some("start_finished".into()),
        }
    }

    #[tokio::test]
    async fn selects_first_idle_instance_and_serial() {
        let manager = InstanceLeaseManager::default();
        let lease = manager
            .acquire(&[instance(3), instance(1)], &[])
            .await
            .unwrap();
        assert_eq!(lease.instance.index, 1);
        assert_eq!(lease.adb_serial, "127.0.0.1:16416");
    }

    #[tokio::test]
    async fn excludes_active_and_not_ready_instances() {
        let manager = InstanceLeaseManager::default();
        let mut offline = instance(1);
        offline.adb_port = None;
        let mut starting = instance(2);
        starting.player_state = Some("starting".into());
        let lease = manager
            .acquire(&[instance(4), offline, starting, instance(0)], &[0])
            .await
            .unwrap();
        assert_eq!(lease.instance.index, 4);
    }

    #[tokio::test]
    async fn concurrent_acquisition_never_leases_same_index() {
        let manager = InstanceLeaseManager::default();
        let instances = [instance(0)];
        let (first, second) = tokio::join!(
            manager.acquire(&instances, &[]),
            manager.acquire(&instances, &[])
        );
        assert_eq!(first.is_ok() as usize + second.is_ok() as usize, 1);
        drop(first);
        drop(second);
        assert_eq!(
            manager
                .acquire(&instances, &[])
                .await
                .unwrap()
                .instance
                .index,
            0
        );
    }

    #[tokio::test]
    async fn async_release_makes_instance_available() {
        let manager = InstanceLeaseManager::default();
        let instances = [instance(0)];
        let first = manager.acquire(&instances, &[]).await.unwrap();
        first.release().await;
        assert_eq!(
            manager
                .acquire(&instances, &[])
                .await
                .unwrap()
                .instance
                .index,
            0
        );
    }
}
