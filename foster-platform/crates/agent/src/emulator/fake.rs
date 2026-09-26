use async_trait::async_trait;
use foster_domain::EmulatorLifecycleStatus;
use foster_protocol::EmulatorDescriptor;

use super::{EmulatorDriver, EmulatorDriverError};

#[derive(Debug, Clone)]
pub struct FakeEmulatorDriver {
    instances: Vec<EmulatorDescriptor>,
}

impl FakeEmulatorDriver {
    pub fn new(instances: Vec<EmulatorDescriptor>) -> Self {
        Self { instances }
    }

    fn find(&self, instance_id: &str) -> Result<&EmulatorDescriptor, EmulatorDriverError> {
        self.instances
            .iter()
            .find(|instance| instance.emulator_code == instance_id)
            .ok_or_else(|| EmulatorDriverError::UnknownInstance(instance_id.to_string()))
    }
}

#[async_trait]
impl EmulatorDriver for FakeEmulatorDriver {
    async fn list_instances(&self) -> Result<Vec<EmulatorDescriptor>, EmulatorDriverError> {
        Ok(self.instances.clone())
    }

    async fn start(&self, instance_id: &str) -> Result<(), EmulatorDriverError> {
        self.find(instance_id)?;
        Ok(())
    }

    async fn stop(&self, instance_id: &str) -> Result<(), EmulatorDriverError> {
        self.find(instance_id)?;
        Ok(())
    }

    async fn adb_serial(&self, instance_id: &str) -> Result<Option<String>, EmulatorDriverError> {
        Ok(self.find(instance_id)?.adb_serial.clone())
    }

    async fn status(
        &self,
        instance_id: &str,
    ) -> Result<EmulatorLifecycleStatus, EmulatorDriverError> {
        self.find(instance_id)?;
        Ok(EmulatorLifecycleStatus::Ready)
    }

    async fn screenshot(&self, instance_id: &str) -> Result<Vec<u8>, EmulatorDriverError> {
        self.find(instance_id)?;
        Ok(Vec::new())
    }
}
