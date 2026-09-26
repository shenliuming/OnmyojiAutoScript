use std::time::Duration;

use async_trait::async_trait;

use crate::emulator::{CommandRunner, ensure_adb_connected};

pub const GAME_NAME: &str = "阴阳师";
/// MuMu's `adb shell input text` cannot reliably inject Chinese on Android 15.
/// The store indexes the game under this ASCII alias and returns the full
/// channel QR-code build as the first result.
pub const GAME_SEARCH_QUERY: &str = "yys";
const FULL_CHANNEL_MARKER: &str = "全渠道扫码";
pub const FULL_CHANNEL_LABEL: &str = "全渠道";
pub const INSTALL_LABEL: &str = "安装";
const DOWNLOAD_LABEL: &str = "下载";
const CONTINUE_DOWNLOAD_LABEL: &str = "继续下载";
const DUMP_PATH: &str = "/sdcard/foster_market.xml";

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("adb command failed on {serial} ({operation})")]
    Adb {
        serial: String,
        operation: &'static str,
    },
    #[error("failed to open the MuMu app market on {serial}")]
    LaunchFailed { serial: String },
    #[error("app market search entry was not found on {serial}")]
    SearchBoxMissing { serial: String },
    #[error("UI text {text:?} was not found on {serial}")]
    TextMissing { serial: String, text: String },
    #[error("app market game entry for {game} was not found on {serial}")]
    GameEntryMissing { serial: String, game: String },
    #[error("full-channel option for {game} was not found in the app market on {serial}")]
    FullChannelOptionMissing { serial: String, game: String },
    #[error("app market install button was not found on {serial}")]
    InstallButtonMissing { serial: String },
    #[error("full-channel package did not finish installing within {seconds}s on {serial}")]
    InstallTimeout { serial: String, seconds: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketNode {
    pub text: String,
    pub resource_id: String,
    pub center: (u32, u32),
}

/// UI automation surface for the MuMu app market. Implementations must derive
/// tap coordinates from the located node itself, never from fixed positions.
#[async_trait]
pub trait MarketUi: Send + Sync + 'static {
    async fn launch(&self, serial: &str) -> Result<(), InstallError>;
    async fn focus_search(&self, _serial: &str) -> Result<(), InstallError> {
        Ok(())
    }
    async fn search(&self, serial: &str, query: &str) -> Result<(), InstallError>;
    async fn has_text(&self, serial: &str, text: &str) -> Result<bool, InstallError>;
    async fn tap_text(&self, serial: &str, text: &str) -> Result<(), InstallError>;
    async fn package_installed(&self, serial: &str, package: &str) -> Result<bool, InstallError>;
}

pub struct AppMarketInstaller {
    ui: std::sync::Arc<dyn MarketUi>,
    full_channel_package: String,
    timeout: Duration,
    poll_interval: Duration,
}

impl AppMarketInstaller {
    pub fn new(
        ui: std::sync::Arc<dyn MarketUi>,
        full_channel_package: impl Into<String>,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Self {
        Self {
            ui,
            full_channel_package: full_channel_package.into(),
            timeout,
            poll_interval,
        }
    }

    pub async fn install_full_channel(&self, serial: &str) -> Result<(), InstallError> {
        self.ui.launch(serial).await?;
        self.ui.search(serial, GAME_SEARCH_QUERY).await?;
        self.ui
            .tap_text(serial, GAME_NAME)
            .await
            .map_err(|error| match error {
                InstallError::TextMissing { serial, .. } => InstallError::GameEntryMissing {
                    serial,
                    game: GAME_NAME.into(),
                },
                other => other,
            })?;

        if self.ui.has_text(serial, FULL_CHANNEL_MARKER).await? {
            // Current MuMu 12 store builds expose this game as one result
            // whose title explicitly says it supports full-channel QR login.
            // Its channel selector contains only “网易”, and tapping that
            // selector would install the wrong ordinary package.  In this UI
            // the result's detail-page “网易 下载” button installs the
            // full-channel package identified below, so use it only after
            // checking the marker and verify the package after installation.
            self.ui
                .tap_text(serial, DOWNLOAD_LABEL)
                .await
                .map_err(|error| match error {
                    InstallError::TextMissing { serial, .. } => {
                        InstallError::InstallButtonMissing { serial }
                    }
                    other => other,
                })?;
        } else if self.ui.has_text(serial, FULL_CHANNEL_LABEL).await? {
            self.ui.tap_text(serial, FULL_CHANNEL_LABEL).await?;
            self.ui
                .tap_text(serial, INSTALL_LABEL)
                .await
                .map_err(|error| match error {
                    InstallError::TextMissing { serial, .. } => {
                        InstallError::InstallButtonMissing { serial }
                    }
                    other => other,
                })?;
        } else {
            return Err(InstallError::FullChannelOptionMissing {
                serial: serial.to_string(),
                game: GAME_NAME.into(),
            });
        }

        let deadline = tokio::time::Instant::now() + self.timeout;
        loop {
            if self
                .ui
                .package_installed(serial, &self.full_channel_package)
                .await?
            {
                return Ok(());
            }
            // MuMu's store can pause a large APK download without surfacing
            // an error.  Keep the preparation self-healing by resuming the
            // same detail-page action when the button is present; a missing
            // button simply means the download is still progressing.
            let _ = self.ui.tap_text(serial, CONTINUE_DOWNLOAD_LABEL).await;
            if tokio::time::Instant::now() >= deadline {
                return Err(InstallError::InstallTimeout {
                    serial: serial.to_string(),
                    seconds: self.timeout.as_secs(),
                });
            }
            tokio::time::sleep(self.poll_interval).await;
        }
    }
}

/// App-market automation over ADB `uiautomator dump` + `input tap`.
pub struct AdbMarketUi<R: CommandRunner> {
    runner: R,
    adb_program: String,
    market_activity: String,
}

impl<R: CommandRunner> AdbMarketUi<R> {
    pub fn new(
        runner: R,
        adb_program: impl Into<String>,
        market_activity: impl Into<String>,
    ) -> Self {
        Self {
            runner,
            adb_program: adb_program.into(),
            market_activity: market_activity.into(),
        }
    }

    async fn run_shell(
        &self,
        serial: &str,
        operation: &'static str,
        shell_args: &[&str],
    ) -> Result<Vec<u8>, InstallError> {
        ensure_adb_connected(&self.runner, &self.adb_program, serial).await;
        let mut args = vec!["-s".to_string(), serial.to_string(), "shell".to_string()];
        args.extend(shell_args.iter().map(|arg| arg.to_string()));
        let output = self
            .runner
            .run(&self.adb_program, &args)
            .await
            .map_err(|_| InstallError::Adb {
                serial: serial.to_string(),
                operation,
            })?;
        let dump_succeeded_with_quirky_exit = operation == "uiautomator dump"
            && String::from_utf8_lossy(&output.stdout).contains("UI hierchary dumped to:");
        if !output.success && !dump_succeeded_with_quirky_exit {
            return Err(InstallError::Adb {
                serial: serial.to_string(),
                operation,
            });
        }
        Ok(output.stdout)
    }

    async fn dump_nodes(&self, serial: &str) -> Result<Vec<MarketNode>, InstallError> {
        self.run_shell(
            serial,
            "uiautomator dump",
            &["uiautomator", "dump", DUMP_PATH],
        )
        .await?;
        let xml = self
            .run_shell(serial, "cat ui dump", &["cat", DUMP_PATH])
            .await?;
        Ok(parse_ui_dump(&String::from_utf8_lossy(&xml)))
    }
}

#[async_trait]
impl<R: CommandRunner> MarketUi for AdbMarketUi<R> {
    async fn launch(&self, serial: &str) -> Result<(), InstallError> {
        self.run_shell(
            serial,
            "am start app market",
            &["am", "start", "-W", "-n", &self.market_activity],
        )
        .await?;
        let mut nodes = None;
        let mut last_error = None;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            match self.dump_nodes(serial).await {
                Ok(value) => {
                    nodes = Some(value);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let nodes = nodes.ok_or_else(|| {
            last_error.unwrap_or_else(|| InstallError::Adb {
                serial: serial.to_string(),
                operation: "uiautomator dump",
            })
        })?;
        if nodes.iter().any(|node| {
            node.text
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>()
                == "跳过"
        }) {
            self.tap_text(serial, "跳").await?;
        }
        Ok(())
    }

    async fn focus_search(&self, serial: &str) -> Result<(), InstallError> {
        let node = self
            .dump_nodes(serial)
            .await?
            .into_iter()
            .find(|node| node.resource_id == "com.mumu.store:id/search_bar")
            .ok_or_else(|| InstallError::SearchBoxMissing {
                serial: serial.to_string(),
            })?;
        self.run_shell(
            serial,
            "input tap search field",
            &[
                "input",
                "tap",
                &node.center.0.to_string(),
                &node.center.1.to_string(),
            ],
        )
        .await?;
        Ok(())
    }

    async fn search(&self, serial: &str, query: &str) -> Result<(), InstallError> {
        self.focus_search(serial).await?;
        // MuMu keeps the previous query in the search field.  Android's
        // `input text` cannot replace it, and the clear icon has no text node
        // in the UI dump, so clear a bounded number of characters through the
        // focused field before entering the ASCII alias.
        let mut clear_args = vec!["input", "keyevent"];
        clear_args.extend(std::iter::repeat("67").take(64));
        self.run_shell(serial, "clear app market search text", &clear_args)
            .await?;
        self.run_shell(serial, "input search text", &["input", "text", query])
            .await?;
        self.run_shell(serial, "input search enter", &["input", "keyevent", "66"])
            .await?;
        Ok(())
    }

    async fn has_text(&self, serial: &str, text: &str) -> Result<bool, InstallError> {
        Ok(self
            .dump_nodes(serial)
            .await?
            .iter()
            .any(|node| node.text.contains(text)))
    }

    async fn tap_text(&self, serial: &str, text: &str) -> Result<(), InstallError> {
        let node = self
            .dump_nodes(serial)
            .await?
            .into_iter()
            .find(|node| node.text.contains(text))
            .ok_or_else(|| InstallError::TextMissing {
                serial: serial.to_string(),
                text: text.to_string(),
            })?;
        self.run_shell(
            serial,
            "input tap",
            &[
                "input",
                "tap",
                &node.center.0.to_string(),
                &node.center.1.to_string(),
            ],
        )
        .await?;
        Ok(())
    }

    async fn package_installed(&self, serial: &str, package: &str) -> Result<bool, InstallError> {
        let stdout = self
            .run_shell(serial, "pm list packages", &["pm", "list", "packages"])
            .await?;
        let expected = format!("package:{package}");
        Ok(String::from_utf8_lossy(&stdout)
            .lines()
            .any(|line| line.trim() == expected))
    }
}

/// Extract text-bearing nodes from a `uiautomator dump` XML hierarchy.
pub fn parse_ui_dump(xml: &str) -> Vec<MarketNode> {
    let mut nodes = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = xml[cursor..].find("<node") {
        let start = cursor + offset;
        let Some(end) = xml[start..].find('>') else {
            break;
        };
        let tag = &xml[start..start + end];
        if let (Some(text), Some(bounds)) = (attribute(tag, "text"), attribute(tag, "bounds")) {
            let resource_id = attribute(tag, "resource-id").unwrap_or_default();
            if (!text.is_empty() || resource_id == "com.mumu.store:id/search_bar")
                && let Some((x1, y1, x2, y2)) = parse_bounds(&bounds)
            {
                nodes.push(MarketNode {
                    text,
                    resource_id,
                    center: ((x1 + x2) / 2, (y1 + y2) / 2),
                });
            }
        }
        cursor = start + end;
    }
    nodes
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}=\"");
    let start = tag.find(&prefix)? + prefix.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Parse uiautomator bounds of the form `[x1,y1][x2,y2]`.
pub fn parse_bounds(value: &str) -> Option<(u32, u32, u32, u32)> {
    let value = value.trim();
    let rest = value.strip_prefix('[')?;
    let (first, rest) = rest.split_once(']')?;
    let rest = rest.strip_prefix('[')?;
    let (second, _) = rest.split_once(']')?;
    let (x1, y1) = first.split_once(',')?;
    let (x2, y2) = second.split_once(',')?;
    Some((
        x1.trim().parse().ok()?,
        y1.trim().parse().ok()?,
        x2.trim().parse().ok()?,
        y2.trim().parse().ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emulator::{CommandOutput, EmulatorDriverError};
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex as StdMutex},
    };

    const FULL: &str = "com.netease.onmyoji.wyzymnqsd_cps";
    const SERIAL: &str = "127.0.0.1:16416";

    #[derive(Clone, Default)]
    struct CallLog(Arc<StdMutex<Vec<String>>>);

    impl CallLog {
        fn push(&self, entry: impl Into<String>) {
            self.0.lock().unwrap().push(entry.into());
        }

        fn snapshot(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    struct FakeMarket {
        log: CallLog,
        screens: StdMutex<Vec<Vec<&'static str>>>,
        installed: StdMutex<bool>,
        never_installs: bool,
    }

    impl FakeMarket {
        fn new(log: CallLog, screens: Vec<Vec<&'static str>>) -> Self {
            Self {
                log,
                screens: StdMutex::new(screens),
                installed: StdMutex::new(false),
                never_installs: false,
            }
        }

        fn current_texts(&self) -> Vec<String> {
            self.screens
                .lock()
                .unwrap()
                .first()
                .map(|texts| texts.iter().map(|text| text.to_string()).collect())
                .unwrap_or_default()
        }

        /// The UI moves forward only when an action lands on it.
        fn advance(&self) {
            let mut screens = self.screens.lock().unwrap();
            if screens.len() > 1 {
                screens.remove(0);
            }
        }
    }

    #[async_trait]
    impl MarketUi for FakeMarket {
        async fn launch(&self, serial: &str) -> Result<(), InstallError> {
            self.log.push(format!("launch:{serial}"));
            Ok(())
        }

        async fn search(&self, serial: &str, query: &str) -> Result<(), InstallError> {
            self.log.push(format!("search:{serial}:{query}"));
            Ok(())
        }

        async fn has_text(&self, serial: &str, text: &str) -> Result<bool, InstallError> {
            self.log.push(format!("has_text:{serial}:{text}"));
            Ok(self
                .current_texts()
                .iter()
                .any(|value| value.contains(text)))
        }

        async fn tap_text(&self, serial: &str, text: &str) -> Result<(), InstallError> {
            self.log.push(format!("tap:{serial}:{text}"));
            if self
                .current_texts()
                .iter()
                .any(|value| value.contains(text))
            {
                if (text == INSTALL_LABEL || text == DOWNLOAD_LABEL) && !self.never_installs {
                    *self.installed.lock().unwrap() = true;
                }
                self.advance();
                Ok(())
            } else {
                Err(InstallError::TextMissing {
                    serial: serial.into(),
                    text: text.into(),
                })
            }
        }

        async fn package_installed(
            &self,
            serial: &str,
            package: &str,
        ) -> Result<bool, InstallError> {
            self.log
                .push(format!("package_installed:{serial}:{package}"));
            Ok(*self.installed.lock().unwrap() && package == FULL)
        }
    }

    fn installer(market: Arc<FakeMarket>) -> AppMarketInstaller {
        AppMarketInstaller::new(
            market,
            FULL,
            Duration::from_millis(200),
            Duration::from_millis(5),
        )
    }

    #[tokio::test]
    async fn happy_path_requires_full_channel_selection_before_install() {
        let log = CallLog::default();
        let market = Arc::new(FakeMarket::new(
            log.clone(),
            vec![
                vec!["阴阳师", "梦幻西游"],
                vec!["阴阳师", "全渠道", "渠道服"],
                vec!["阴阳师(全渠道)", "安装"],
                vec![],
            ],
        ));

        installer(market.clone())
            .install_full_channel(SERIAL)
            .await
            .unwrap();

        assert_eq!(
            log.snapshot(),
            vec![
                format!("launch:{SERIAL}"),
                format!("search:{SERIAL}:{GAME_SEARCH_QUERY}"),
                format!("tap:{SERIAL}:{GAME_NAME}"),
                format!("has_text:{SERIAL}:全渠道扫码"),
                format!("has_text:{SERIAL}:{FULL_CHANNEL_LABEL}"),
                format!("tap:{SERIAL}:{FULL_CHANNEL_LABEL}"),
                format!("tap:{SERIAL}:{INSTALL_LABEL}"),
                format!("package_installed:{SERIAL}:{FULL}"),
            ]
        );
        assert!(*market.installed.lock().unwrap());
    }

    #[tokio::test]
    async fn missing_full_channel_option_is_a_hard_failure_without_install() {
        let log = CallLog::default();
        let market = Arc::new(FakeMarket::new(
            log.clone(),
            vec![vec!["阴阳师"], vec!["阴阳师", "官服", "安装"], vec![]],
        ));

        let error = installer(market)
            .install_full_channel(SERIAL)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            InstallError::FullChannelOptionMissing { ref serial, .. } if serial == SERIAL
        ));
        assert!(
            !log.snapshot()
                .contains(&format!("tap:{SERIAL}:{INSTALL_LABEL}"))
        );
    }

    #[tokio::test]
    async fn current_mumu_result_uses_full_channel_title_marker() {
        let log = CallLog::default();
        let market = Arc::new(FakeMarket::new(
            log.clone(),
            vec![
                vec!["阴阳师（支持官服安卓/iOS账密+全渠道扫码）"],
                vec!["阴阳师（支持官服安卓/iOS账密+全渠道扫码）", "网易 下载"],
                vec![],
            ],
        ));

        installer(market)
            .install_full_channel(SERIAL)
            .await
            .unwrap();

        assert_eq!(
            log.snapshot(),
            vec![
                format!("launch:{SERIAL}"),
                format!("search:{SERIAL}:{GAME_SEARCH_QUERY}"),
                format!("tap:{SERIAL}:{GAME_NAME}"),
                format!("has_text:{SERIAL}:全渠道扫码"),
                format!("tap:{SERIAL}:下载"),
                format!("package_installed:{SERIAL}:{FULL}"),
            ]
        );
    }

    #[tokio::test]
    async fn missing_game_entry_reports_game_entry_missing() {
        let log = CallLog::default();
        let market = Arc::new(FakeMarket::new(log, vec![vec!["王者荣耀", "安装"], vec![]]));

        let error = installer(market)
            .install_full_channel(SERIAL)
            .await
            .unwrap_err();

        assert!(matches!(error, InstallError::GameEntryMissing { .. }));
    }

    #[tokio::test]
    async fn package_never_appearing_times_out() {
        let log = CallLog::default();
        let mut market = FakeMarket::new(
            log,
            vec![
                vec!["阴阳师"],
                vec!["阴阳师", "全渠道", "安装"],
                vec!["正在安装"],
            ],
        );
        market.never_installs = true;
        let market = Arc::new(market);
        let installer = AppMarketInstaller::new(
            market,
            FULL,
            Duration::from_millis(40),
            Duration::from_millis(5),
        );

        let error = installer.install_full_channel(SERIAL).await.unwrap_err();

        assert!(matches!(error, InstallError::InstallTimeout { .. }));
    }

    #[test]
    fn parses_text_nodes_with_bounds_centers() {
        let xml = r#"<hierarchy><node text="" bounds="[0,0][1280,720]"><node text="阴阳师" bounds="[100,200][300,260]"/><node text="全渠道" content-desc="x" bounds="[40,40][80,80]"/></node></hierarchy>"#;
        let nodes = parse_ui_dump(xml);
        assert_eq!(
            nodes,
            vec![
                MarketNode {
                    text: "阴阳师".into(),
                    resource_id: String::new(),
                    center: (200, 230)
                },
                MarketNode {
                    text: "全渠道".into(),
                    resource_id: String::new(),
                    center: (60, 60)
                },
            ]
        );
    }

    #[test]
    fn parse_bounds_rejects_malformed_values() {
        assert_eq!(parse_bounds("[0,0][10,20]"), Some((0, 0, 10, 20)));
        assert_eq!(parse_bounds("[0,0][10,20] extra"), Some((0, 0, 10, 20)));
        for bad in ["0,0][10,20]", "[0,0]", "[a,b][c,d]", ""] {
            assert_eq!(parse_bounds(bad), None, "expected rejection of {bad:?}");
        }
    }

    #[derive(Clone, Default)]
    struct RecordingRunner {
        calls: Arc<StdMutex<Vec<String>>>,
        outputs: Arc<StdMutex<VecDeque<CommandOutput>>>,
    }

    #[async_trait]
    impl CommandRunner for RecordingRunner {
        async fn run(
            &self,
            program: &str,
            args: &[String],
        ) -> Result<CommandOutput, EmulatorDriverError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{program} {}", args.join(" ")));
            self.outputs
                .lock()
                .unwrap()
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

    #[tokio::test]
    async fn adb_market_ui_taps_derived_centers_and_checks_exact_package() {
        let dump_xml = r#"<?xml version="1.0"?><hierarchy><node text="搜 索" resource-id="com.mumu.store:id/search_bar" bounds="[600,40][680,90]"/><node text="阴阳师" bounds="[100,300][500,360]"/></hierarchy>"#;
        let runner = RecordingRunner {
            outputs: Arc::new(StdMutex::new(
                [
                    ok("connected"), // connect before am start
                    ok(""),          // am start
                    ok("connected"), // connect before startup overlay dump
                    ok(""),          // uiautomator dump (startup overlay)
                    ok("connected"), // connect before startup overlay dump cat
                    ok(dump_xml),    // cat dump (startup overlay)
                    ok("connected"), // connect before search dump
                    ok(""),          // uiautomator dump (search box)
                    ok("connected"), // connect before search dump cat
                    ok(dump_xml),    // cat dump (search box)
                    ok("connected"), // connect before search tap
                    ok(""),          // input tap (search box center)
                    ok("connected"), // connect before clearing search text
                    ok(""),          // clear previous search text
                    ok("connected"), // connect before input text
                    ok(""),          // input text
                    ok("connected"), // connect before input enter
                    ok(""),          // input keyevent
                    ok("connected"), // connect before results dump
                    ok(""),          // uiautomator dump (results)
                    ok("connected"), // connect before results dump cat
                    ok(dump_xml),    // cat dump (results)
                    ok("connected"), // connect before game tap
                    ok(""),          // input tap game entry center
                    ok("connected"), // connect before full package check
                    ok(&format!("package:{FULL}\npackage:com.other")),
                    ok("connected"), // connect before ordinary package check
                    ok(&format!("package:com.other")),
                ]
                .into(),
            )),
            ..RecordingRunner::default()
        };

        let ui = AdbMarketUi::new(runner.clone(), "adb", "com.mumu.store/.MainActivity");

        ui.launch(SERIAL).await.unwrap();
        ui.search(SERIAL, GAME_SEARCH_QUERY).await.unwrap();
        ui.tap_text(SERIAL, GAME_NAME).await.unwrap();
        assert!(ui.package_installed(SERIAL, FULL).await.unwrap());
        assert!(
            !ui.package_installed(SERIAL, "com.netease.onmyoji")
                .await
                .unwrap()
        );

        let calls = runner.calls.lock().unwrap().clone();
        assert_eq!(
            calls[1],
            format!("adb -s {SERIAL} shell am start -W -n com.mumu.store/.MainActivity")
        );
        // Search-box tap must use the node's own center, not a fixed coordinate.
        assert_eq!(calls[11], format!("adb -s {SERIAL} shell input tap 640 65"));
        assert_eq!(
            calls[15],
            format!("adb -s {SERIAL} shell input text {GAME_SEARCH_QUERY}")
        );
        assert_eq!(
            calls[17],
            format!("adb -s {SERIAL} shell input keyevent 66")
        );
        assert_eq!(
            calls[23],
            format!("adb -s {SERIAL} shell input tap 300 330")
        );
        assert_eq!(calls[25], format!("adb -s {SERIAL} shell pm list packages"));
        assert_eq!(calls[27], format!("adb -s {SERIAL} shell pm list packages"));
    }
}
