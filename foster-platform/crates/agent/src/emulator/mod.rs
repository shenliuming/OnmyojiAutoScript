mod driver;
mod fake;

pub use driver::{EmulatorDriver, EmulatorDriverError};
pub use fake::FakeEmulatorDriver;
