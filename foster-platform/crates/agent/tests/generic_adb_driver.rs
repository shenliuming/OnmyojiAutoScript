use std::{collections::VecDeque, sync::Arc, time::Duration};

use async_trait::async_trait;
use foster_agent::emulator::{
    CommandOutput, CommandRunner, EmulatorDriver, EmulatorDriverError, GenericAdbEmulatorDriver,
    wait_adb_online,
};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Default)]
struct FakeRunner {
    outputs: Arc<Mutex<VecDeque<CommandOutput>>>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl FakeRunner {
    fn with_outputs(outputs: Vec<CommandOutput>) -> Self {
        Self {
            outputs: Arc::new(Mutex::new(outputs.into())),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn calls(&self) -> Vec<String> {
        self.calls.lock().await.clone()
    }
}

#[async_trait]
impl CommandRunner for FakeRunner {
    async fn run(
        &self,
        program: &str,
        args: &[String],
    ) -> Result<CommandOutput, EmulatorDriverError> {
        self.calls
            .lock()
            .await
            .push(format!("{program} {}", args.join(" ")));
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
    assert_eq!(instances[0].adb_serial.as_deref(), Some("127.0.0.1:16384"));
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
    let driver =
        GenericAdbEmulatorDriver::from_json_with_runner(config_json(), "adb".into(), runner)?;

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
    let driver =
        GenericAdbEmulatorDriver::from_json_with_runner(config_json(), "adb".into(), runner)?;

    assert_eq!(driver.status("emu-01").await?, EmulatorStatus::Idle);
    assert_eq!(driver.status("emu-01").await?, EmulatorStatus::Offline);

    Ok(())
}

#[tokio::test]
async fn status_recovers_network_serial_after_adb_connect() -> anyhow::Result<()> {
    use foster_domain::EmulatorStatus;

    let runner = FakeRunner::with_outputs(vec![
        CommandOutput {
            success: false,
            stdout: Vec::new(),
            stderr: b"error: device '(null)' not found".to_vec(),
        },
        CommandOutput {
            success: true,
            stdout: b"connected to 127.0.0.1:16384\n".to_vec(),
            stderr: Vec::new(),
        },
        CommandOutput {
            success: true,
            stdout: b"device\n".to_vec(),
            stderr: Vec::new(),
        },
    ]);
    let driver = GenericAdbEmulatorDriver::from_json_with_runner(
        config_json(),
        "adb".into(),
        runner.clone(),
    )?;

    assert_eq!(driver.status("emu-01").await?, EmulatorStatus::Idle);

    let calls = runner.calls().await;
    assert_eq!(calls[0], "adb -s 127.0.0.1:16384 get-state");
    assert_eq!(calls[1], "adb connect 127.0.0.1:16384");
    assert_eq!(calls[2], "adb -s 127.0.0.1:16384 get-state");

    Ok(())
}

#[tokio::test]
async fn wait_adb_online_connects_network_serial_before_polling() -> anyhow::Result<()> {
    let runner = FakeRunner::with_outputs(vec![
        CommandOutput {
            success: false,
            stdout: Vec::new(),
            stderr: b"error: device '(null)' not found".to_vec(),
        },
        CommandOutput {
            success: true,
            stdout: b"connected to 127.0.0.1:16384\n".to_vec(),
            stderr: Vec::new(),
        },
        CommandOutput {
            success: true,
            stdout: b"device\n".to_vec(),
            stderr: Vec::new(),
        },
    ]);

    wait_adb_online(&runner, "adb", "127.0.0.1:16384", Duration::from_secs(2)).await?;

    let calls = runner.calls().await;
    assert_eq!(calls[0], "adb connect 127.0.0.1:16384");
    assert_eq!(calls[1], "adb -s 127.0.0.1:16384 get-state");

    Ok(())
}
