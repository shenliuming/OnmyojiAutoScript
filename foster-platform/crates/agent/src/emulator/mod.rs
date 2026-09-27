mod driver;
mod fake;
mod generic_adb;
mod lease;

pub use driver::{EmulatorDriver, EmulatorDriverError};
pub use fake::FakeEmulatorDriver;
pub use lease::{EmulatorLease, EmulatorLeaseError, EmulatorLeaseManager};
pub use generic_adb::{
    CommandOutput, CommandRunner, EmulatorInstanceConfig, GenericAdbEmulatorDriver,
    SystemCommandRunner, capture_screenshot, ensure_adb_connected, launch_package,
    parse_package_listing, query_installed_packages, uninstall_package, wait_adb_online,
    wait_package_running,
};
