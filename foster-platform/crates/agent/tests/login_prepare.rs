use std::{collections::VecDeque, sync::Arc, time::Duration};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use foster_agent::{
    emulator::{CommandOutput, CommandRunner, EmulatorDriverError, GenericAdbEmulatorDriver},
    login::{HttpOasLoginExecutor, LoginExecutor},
    mumu::{
        AppMarketInstaller, MarketUi, MumuConfig, MumuController, MumuError, MumuInstanceInfo,
        MumuLoginPreparer,
    },
};
use foster_protocol::StartLoginCommand;

const SERIAL: &str = "127.0.0.1:16384";
const OTHER_SERIAL: &str = "127.0.0.1:16416";
const NORMAL: &str = "com.netease.onmyoji";
const FULL: &str = "com.netease.onmyoji.wyzymnqsd_cps";
const PNG: &[u8] = &[137, 80, 78, 71, 1, 2, 3];

#[derive(Clone, Default)]
struct FakeRunner {
    calls: Arc<tokio::sync::Mutex<Vec<String>>>,
    outputs: Arc<tokio::sync::Mutex<VecDeque<CommandOutput>>>,
}

impl FakeRunner {
    fn with_outputs(outputs: Vec<CommandOutput>) -> Self {
        Self {
            calls: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            outputs: Arc::new(tokio::sync::Mutex::new(outputs.into())),
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
        if args.first().map(String::as_str) == Some("connect")
            && self
                .calls
                .lock()
                .await
                .iter()
                .any(|call| call.starts_with("adb connect "))
        {
            return Ok(ok("connected\n"));
        }
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

fn packages(variants: &[&str]) -> CommandOutput {
    let stdout = variants
        .iter()
        .map(|package| format!("package:{package}"))
        .collect::<Vec<_>>()
        .join("\n");
    ok(&stdout)
}

fn bytes(stdout: &[u8]) -> CommandOutput {
    CommandOutput {
        success: true,
        stdout: stdout.to_vec(),
        stderr: Vec::new(),
    }
}

#[derive(Default)]
struct FakeController {
    calls: Arc<tokio::sync::Mutex<Vec<String>>>,
}

impl FakeController {
    fn instance(index: u32, port: u16) -> MumuInstanceInfo {
        MumuInstanceInfo {
            index,
            name: format!("MuMu-{index}"),
            adb_host_ip: Some("127.0.0.1".into()),
            adb_port: Some(port),
            is_android_started: true,
            is_process_started: true,
            player_state: Some("start_finished".into()),
        }
    }

    async fn calls(&self) -> Vec<String> {
        self.calls.lock().await.clone()
    }
}

#[async_trait]
impl MumuController for FakeController {
    async fn info_all(&self) -> Result<Vec<MumuInstanceInfo>, MumuError> {
        self.calls.lock().await.push("info_all".into());
        Ok(vec![Self::instance(0, 16384), Self::instance(1, 16416)])
    }

    async fn launch_instance(&self, index: u32) -> Result<(), MumuError> {
        self.calls.lock().await.push(format!("launch:{index}"));
        Ok(())
    }

    async fn apply_resolution(&self, index: u32) -> Result<(), MumuError> {
        self.calls.lock().await.push(format!("resolution:{index}"));
        Ok(())
    }
}

#[derive(Default)]
struct FakeMarketUi {
    calls: Arc<tokio::sync::Mutex<Vec<String>>>,
    fail_full_channel: bool,
    report_installed_package: Option<String>,
}

impl FakeMarketUi {
    async fn calls(&self) -> Vec<String> {
        self.calls.lock().await.clone()
    }
}

#[async_trait]
impl MarketUi for FakeMarketUi {
    async fn launch(&self, serial: &str) -> Result<(), InstallError> {
        self.calls.lock().await.push(format!("launch:{serial}"));
        Ok(())
    }

    async fn search(&self, serial: &str, query: &str) -> Result<(), InstallError> {
        self.calls
            .lock()
            .await
            .push(format!("search:{serial}:{query}"));
        Ok(())
    }

    async fn has_text(&self, serial: &str, text: &str) -> Result<bool, InstallError> {
        self.calls
            .lock()
            .await
            .push(format!("has_text:{serial}:{text}"));
        if text == "全渠道扫码" {
            return Ok(false);
        }
        Ok(!(self.fail_full_channel && text == "全渠道"))
    }

    async fn tap_text(&self, serial: &str, text: &str) -> Result<(), InstallError> {
        self.calls.lock().await.push(format!("tap:{serial}:{text}"));
        Ok(())
    }

    async fn package_installed(&self, serial: &str, package: &str) -> Result<bool, InstallError> {
        self.calls
            .lock()
            .await
            .push(format!("package_installed:{serial}:{package}"));
        Ok(self.report_installed_package.as_deref() == Some(package))
    }
}

use foster_agent::mumu::InstallError;

fn config_json() -> &'static str {
    r#"[
      {
        "emulatorCode": "emu-login",
        "adbSerial": "127.0.0.1:16384",
        "oasConfigName": "oas-login",
        "packageName": null,
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

fn mumu_config() -> MumuConfig {
    let mut config = MumuConfig::default();
    config.launch_settle_delay = Duration::ZERO;
    config
}

fn executor(
    runner: FakeRunner,
    controller: Arc<FakeController>,
    market: Arc<FakeMarketUi>,
) -> HttpOasLoginExecutor<FakeRunner> {
    let driver = GenericAdbEmulatorDriver::from_json_with_runner(
        config_json(),
        "adb".into(),
        runner.clone(),
    )
    .unwrap();
    let installer = AppMarketInstaller::new(
        market,
        FULL,
        Duration::from_millis(200),
        Duration::from_millis(1),
    );
    let preparer = MumuLoginPreparer::new(
        controller,
        Arc::new(runner),
        Arc::new(installer),
        mumu_config(),
        "adb",
    )
    .shared();
    HttpOasLoginExecutor::new(
        driver,
        "http://127.0.0.1:9".into(),
        Duration::from_secs(120),
        Duration::from_secs(1),
        Duration::from_millis(10),
    )
    .with_preparer(preparer)
}

fn command() -> StartLoginCommand {
    StartLoginCommand {
        session_no: "LOGIN-PREPARE".into(),
        game_account_id: 1001,
        emulator_code: "emu-login".into(),
    }
}

#[tokio::test]
async fn prepare_orders_lease_to_screenshot_and_returns_qr() {
    let runner = FakeRunner::with_outputs(vec![
        ok("connected to 127.0.0.1:16384\n"), // adb connect
        ok("device\n"),                       // wait adb online
        packages(&[NORMAL]),                  // instance 0: ordinary package present
        ok("Success\n"),                      // uninstall on instance 0
        packages(&[NORMAL]),                  // instance 1: ordinary package present
        ok("Success\n"),                      // uninstall on instance 1
        packages(&[]),                        // decide: nothing installed yet
        packages(&[FULL]),                    // verify after market install
        ok(""),                               // monkey launch of the full-channel package
        ok("12345\n"),                        // pidof
        bytes(PNG),                           // screencap
    ]);
    let controller = Arc::new(FakeController::default());
    let market = Arc::new(FakeMarketUi {
        report_installed_package: Some(FULL.into()),
        ..FakeMarketUi::default()
    });
    let executor = executor(runner.clone(), controller.clone(), market.clone());

    let prepared = executor.prepare(&command()).await.unwrap();

    let expected_calls = vec![
        format!("adb connect {SERIAL}"),
        format!("adb -s {SERIAL} get-state"),
        format!("adb -s {SERIAL} shell pm list packages"),
        format!("adb -s {SERIAL} shell pm uninstall {NORMAL}"),
        format!("adb -s {OTHER_SERIAL} shell pm list packages"),
        format!("adb -s {OTHER_SERIAL} shell pm uninstall {NORMAL}"),
        format!("adb -s {SERIAL} shell pm list packages"),
        format!("adb -s {SERIAL} shell pm list packages"),
        format!("adb -s {SERIAL} shell monkey -p {FULL} -c android.intent.category.LAUNCHER 1"),
        format!("adb -s {SERIAL} shell pidof {FULL}"),
        format!("adb -s {SERIAL} exec-out screencap -p"),
    ];
    assert_eq!(runner.calls().await, expected_calls);
    assert_eq!(
        controller.calls().await,
        vec!["info_all", "resolution:0", "info_all"]
    );
    assert_eq!(
        market.calls().await,
        vec![
            format!("launch:{SERIAL}"),
            format!("search:{SERIAL}:yys"),
            format!("tap:{SERIAL}:阴阳师"),
            format!("has_text:{SERIAL}:全渠道扫码"),
            format!("has_text:{SERIAL}:全渠道"),
            format!("tap:{SERIAL}:全渠道"),
            format!("tap:{SERIAL}:安装"),
            format!("package_installed:{SERIAL}:{FULL}"),
        ]
    );
    assert_eq!(
        prepared.qr_payload,
        format!("data:image/png;base64,{}", STANDARD.encode(PNG))
    );
}

#[tokio::test]
async fn failed_market_install_stops_before_launch() {
    let runner = FakeRunner::with_outputs(vec![
        ok("connected to 127.0.0.1:16384\n"), // adb connect
        ok("device\n"),
        packages(&[]), // instance 0 clean
        packages(&[]), // instance 1 clean
        packages(&[]), // decide: nothing installed
    ]);
    let controller = Arc::new(FakeController::default());
    let market = Arc::new(FakeMarketUi {
        fail_full_channel: true,
        ..FakeMarketUi::default()
    });
    let executor = executor(runner.clone(), controller, market);

    let error = executor.prepare(&command()).await.unwrap_err();

    assert!(error.to_string().contains("full-channel"));
    assert!(
        !runner
            .calls()
            .await
            .iter()
            .any(|call| call.contains("monkey"))
    );
}

#[tokio::test]
async fn wrong_installed_package_stops_before_launch() {
    let runner = FakeRunner::with_outputs(vec![
        ok("connected to 127.0.0.1:16384\n"), // adb connect
        ok("device\n"),
        packages(&[]),                  // instance 0 clean
        packages(&[]),                  // instance 1 clean
        packages(&[]),                  // decide: nothing installed
        packages(&["com.other.wrong"]), // verify: a wrong package, not full-channel
    ]);
    let controller = Arc::new(FakeController::default());
    let market = Arc::new(FakeMarketUi {
        report_installed_package: Some(FULL.into()),
        ..FakeMarketUi::default()
    });
    let executor = executor(runner.clone(), controller, market);

    let error = executor.prepare(&command()).await.unwrap_err();

    assert!(error.to_string().contains(FULL));
    assert!(
        !runner
            .calls()
            .await
            .iter()
            .any(|call| call.contains("monkey"))
    );
}
