mod driver;
mod fake;
mod generic_adb;

pub use driver::{EmulatorDriver, EmulatorDriverError};
pub use fake::FakeEmulatorDriver;
pub use generic_adb::{
    CommandOutput, CommandRunner, EmulatorInstanceConfig, GenericAdbEmulatorDriver,
    SystemCommandRunner, capture_screenshot, launch_package, parse_package_listing,
    query_installed_packages, uninstall_package, wait_adb_online, wait_package_running,
};
