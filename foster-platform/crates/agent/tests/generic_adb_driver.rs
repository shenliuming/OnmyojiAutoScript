use std::{
    collections::VecDeque,
    sync::Arc,
};

use async_trait::async_trait;
use foster_agent::emulator::{
    CommandOutput, CommandRunner, EmulatorDriver, EmulatorDriverError,
    GenericAdbEmulatorDriver,
};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Default)]
struct FakeRunner {
    outputs: Arc<Mutex<VecDeque<CommandOutput>>>,
}

impl FakeRunner {
    fn with_outputs(outputs: Vec<CommandOutput>) -> Self {
        Self {
            outputs: Arc::new(Mutex::new(outputs.into())),
        }
    }
}

#[async_trait]
impl CommandRunner for FakeRunner {
    async fn run(
        &self,
        _program: &str,
        _args: &[String],
    ) -> Result<CommandOutput, EmulatorDriverError> {
        self.outputs
            .lock()
            .await
            .pop_front()
            .ok_or_else(|| EmulatorDriverError::Message("no fake output".into()))
    }
}

fn config_json() -> &'static str {
    r#"[
      {
        "emulatorCode": "emu-01",
        "adbSerial": "127.0.0.1:16384",
        "oasConfigName": "oas-01",
        "packageName": "com.netease.onmyoji",
        "startProgram": null,
        "startArgs": [],
        "stopProgram": null,
        "stopArgs": [],
        "loginPrepareProgram": null,
        "loginPrepareArgs": [],
        "loginPrepareDelayMs": 0
      }
    ]"#
}

#[tokio::test]
async fn static_inventory_exposes_emulator_descriptor() -> anyhow::Result<()> {
    let driver = GenericAdbEmulatorDriver::from_json_with_runner(
        config_json(),
        "adb".into(),
        FakeRunner::default(),
    )?;

    let instances = driver.list_instances().await?;

    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].emulator_code, "emu-01");
    assert_eq!(instances[0].driver_type, "ADB");
    assert_eq!(
        instances[0].adb_serial.as_deref(),
        Some("127.0.0.1:16384")
    );
    assert_eq!(
        driver.oas_config_map().get("emu-01").map(String::as_str),
        Some("oas-01")
    );

    Ok(())
}

#[tokio::test]
async fn screenshot_returns_raw_png_bytes() -> anyhow::Result<()> {
    let png = vec![137, 80, 78, 71, 13, 10, 26, 10, 1, 2, 3];
    let runner = FakeRunner::with_outputs(vec![CommandOutput {
        success: true,
        stdout: png.clone(),
        stderr: Vec::new(),
    }]);
    let driver = GenericAdbEmulatorDriver::from_json_with_runner(
        config_json(),
        "adb".into(),
        runner,
    )?;

    let bytes = driver.screenshot("emu-01").await?;

    assert_eq!(bytes, png);
    Ok(())
}

#[tokio::test]
async fn unknown_emulator_is_rejected() -> anyhow::Result<()> {
    let driver = GenericAdbEmulatorDriver::from_json_with_runner(
        config_json(),
        "adb".into(),
        FakeRunner::default(),
    )?;

    let result = driver.screenshot("missing").await;

    assert!(matches!(
        result,
        Err(EmulatorDriverError::UnknownInstance(value))
            if value == "missing"
    ));

    Ok(())
}

#[test]
fn duplicate_emulator_code_is_rejected() {
    let duplicate = r#"[
      {
        "emulatorCode": "emu-01",
        "adbSerial": "a",
        "oasConfigName": "oas-a"
      },
      {
        "emulatorCode": "emu-01",
        "adbSerial": "b",
        "oasConfigName": "oas-b"
      }
    ]"#;

    let result = GenericAdbEmulatorDriver::from_json_with_runner(
        duplicate,
        "adb".into(),
        FakeRunner::default(),
    );

    assert!(result.is_err());
}


#[tokio::test]
async fn adb_state_controls_emulator_health() -> anyhow::Result<()> {
    use foster_domain::EmulatorStatus;

    let runner = FakeRunner::with_outputs(vec![
        CommandOutput {
            success: true,
            stdout: b"device\n".to_vec(),
            stderr: Vec::new(),
        },
        CommandOutput {
            success: false,
            stdout: Vec::new(),
            stderr: b"offline".to_vec(),
        },
    ]);
    let driver = GenericAdbEmulatorDriver::from_json_with_runner(
        config_json(),
        "adb".into(),
        runner,
    )?;

    assert_eq!(
        driver.status("emu-01").await?,
        EmulatorStatus::Idle
    );
    assert_eq!(
        driver.status("emu-01").await?,
        EmulatorStatus::Offline
    );

    Ok(())
}
