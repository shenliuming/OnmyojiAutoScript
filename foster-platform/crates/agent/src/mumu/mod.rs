pub mod app_market;
pub mod cli;
pub mod lease;
pub mod preparer;

use async_trait::async_trait;

pub use app_market::{
    AppMarketInstaller, AdbMarketUi, InstallError, MarketNode, MarketUi, parse_bounds,
    parse_ui_dump,
};
pub use cli::{
    CommandOutput, MumuCli, MumuConfig, MumuError, MumuInstanceInfo, launch_args,
    resolution_args,
};
pub use lease::{InstanceLease, InstanceLeaseManager, LeaseError, MumuInstanceSource};
pub use preparer::{MumuLoginPreparer, PreparedInstance, PrepareError};

/// Control surface for MuMu instances used during login preparation.
#[async_trait]
pub trait MumuController: Send + Sync + 'static {
    async fn info_all(&self) -> Result<Vec<MumuInstanceInfo>, MumuError>;
    async fn launch_instance(&self, index: u32) -> Result<(), MumuError>;
    async fn apply_resolution(&self, index: u32) -> Result<(), MumuError>;
}
