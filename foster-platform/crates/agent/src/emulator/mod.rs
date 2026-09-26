mod driver;
mod fake;
mod generic_adb;
mod runtime_state;

pub use driver::{EmulatorDriver, EmulatorDriverError};
pub use fake::FakeEmulatorDriver;
pub use generic_adb::{
    CommandOutput, CommandRunner, EmulatorInstanceConfig, GenericAdbEmulatorDriver,
    SystemCommandRunner,
};

pub use runtime_state::{
    EmulatorRuntimeError, EmulatorRuntimeLease, EmulatorRuntimeRegistry, EmulatorRuntimeSnapshot,
};
