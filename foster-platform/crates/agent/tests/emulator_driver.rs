use foster_agent::emulator::{
    EmulatorDriver, EmulatorDriverError, FakeEmulatorDriver,
};
use foster_protocol::EmulatorDescriptor;

#[tokio::test]
async fn fake_driver_reports_configured_instances() {
    let driver = FakeEmulatorDriver::new(vec![EmulatorDescriptor {
        emulator_code: "emu-01".into(),
        driver_type: "FAKE".into(),
        adb_serial: Some("127.0.0.1:5555".into()),
    }]);

    let instances = driver.list_instances().await.unwrap();

    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].emulator_code, "emu-01");
}

#[tokio::test]
async fn fake_driver_rejects_unknown_instance() {
    let driver = FakeEmulatorDriver::new(Vec::new());

    let result = driver.start("missing").await;

    assert!(matches!(
        result,
        Err(EmulatorDriverError::UnknownInstance(value))
            if value == "missing"
    ));
}
