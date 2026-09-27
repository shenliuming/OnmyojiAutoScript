use std::{collections::VecDeque, sync::Arc, time::Duration};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use foster_agent::{
    emulator::{CommandOutput, CommandRunner, EmulatorDriver, EmulatorDriverError, GenericAdbEmulatorDriver},
    login::{HttpOasLoginExecutor, LoginExecutor},
};
use foster_protocol::{LoginPlatform, StartLoginCommand};
use tokio::sync::Mutex;

const SERIAL: &str = "emulator-5554";
const FULL: &str = "com.netease.onmyoji.wyzymnqsd_cps";
const PNG: &[u8] = &[137, 80, 78, 71, 1, 2, 3];

#[derive(Clone, Default)]
struct FakeRunner {
    calls: Arc<Mutex<Vec<String>>>,
    outputs: Arc<Mutex<VecDeque<CommandOutput>>>,
}

impl FakeRunner {
    fn with_outputs(outputs: Vec<CommandOutput>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            outputs: Arc::new(Mutex::new(outputs.into())),
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
            .ok_or_else(|| EmulatorDriverError::Message("no scripted output".into()))
    }
}

fn ok(stdout: &str) -> CommandOutput {
    CommandOutput {
        success: true,
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
    }
}

fn bytes(stdout: &[u8]) -> CommandOutput {
    CommandOutput {
        success: true,
        stdout: stdout.to_vec(),
        stderr: Vec::new(),
    }
}

fn config_json() -> &'static str {
    r#"[
      {
        "emulatorCode": "ld-00",
        "driverType": "LDPLAYER",
        "adbSerial": "emulator-5554",
        "oasConfigName": "oas-ld-00",
        "packageName": "com.netease.onmyoji.wyzymnqsd_cps",
        "startProgram": "ldconsole.exe",
        "startArgs": ["launch", "--index", "0"],
        "stopProgram": "ldconsole.exe",
        "stopArgs": ["quit", "--index", "0"],
        "loginPrepareProgram": null,
        "loginPrepareArgs": [],
        "loginPrepareDelayMs": 0
      }
    ]"#
}

fn command() -> StartLoginCommand {
    StartLoginCommand {
        session_no: "LOGIN-LDPLAYER".into(),
        game_account_id: 1001,
        emulator_code: "ld-00".into(),
        platform: LoginPlatform::Android,
        character_name: "角色A".into(),
        game_uid: "10001".into(),
    }
}

#[tokio::test]
async fn prepare_uses_ldconsole_then_adb_then_screenshot() -> anyhow::Result<()> {
    let runner = FakeRunner::with_outputs(vec![
        ok(""),          // ldconsole launch
        ok("device\n"), // adb get-state
        ok("Events injected: 1\n"), // adb monkey
        bytes(PNG),      // screencap
    ]);
    let driver = GenericAdbEmulatorDriver::from_json_with_runner(
        config_json(),
        "adb".into(),
        runner.clone(),
    )?;
    let executor = HttpOasLoginExecutor::new(
        driver,
        "http://127.0.0.1:9".into(),
        Duration::from_secs(120),
        Duration::from_secs(1),
        Duration::from_millis(10),
    );

    let prepared = executor.prepare(&command()).await?;

    assert_eq!(
        runner.calls().await,
        vec![
            "ldconsole.exe launch --index 0",
            format!("adb -s {SERIAL} get-state"),
            format!("adb -s {SERIAL} shell monkey -p {FULL} -c android.intent.category.LAUNCHER 1"),
            format!("adb -s {SERIAL} exec-out screencap -p"),
        ]
    );
    assert_eq!(
        prepared.qr_payload,
        format!("data:image/png;base64,{}", STANDARD.encode(PNG))
    );
    Ok(())
}

#[tokio::test]
async fn descriptor_reports_ldplayer_and_stop_uses_ldconsole() -> anyhow::Result<()> {
    let runner = FakeRunner::with_outputs(vec![ok("")]);
    let driver = GenericAdbEmulatorDriver::from_json_with_runner(
        config_json(),
        "adb".into(),
        runner.clone(),
    )?;

    let instances = driver.list_instances().await?;
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].driver_type, "LDPLAYER");
    assert_eq!(instances[0].adb_serial.as_deref(), Some(SERIAL));

    driver.stop("ld-00").await?;
    assert_eq!(runner.calls().await, vec!["ldconsole.exe quit --index 0"]);
    Ok(())
}
