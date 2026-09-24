use std::sync::Arc;
use std::time::Duration;

use crate::emulator::{CommandRunner, SystemCommandRunner, query_installed_packages, uninstall_package, wait_adb_online};

use super::{
    AppMarketInstaller, InstallError, MumuConfig, MumuController, MumuError, MumuInstanceInfo,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstance {
    pub mumu_index: u32,
    pub adb_serial: String,
    pub oas_config_name: String,
    pub full_channel_package: String,
    pub launch_wait: Duration,
    pub settle_delay: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    #[error("MuMu instance discovery failed: {0}")]
    Discovery(#[from] MumuError),
    #[error("MuMu instance for serial {0} was not found")]
    InstanceNotFound(String),
    #[error("MuMu control failed for instance {index}: {source}")]
    Controller { index: u32, source: MumuError },
    #[error("adb command failed on {serial} ({operation})")]
    Adb {
        serial: String,
        operation: &'static str,
    },
    #[error("failed to remove ordinary package {package} on {serial}; its local data is left untouched")]
    NormalPackageRemoval { serial: String, package: String },
    #[error("ordinary package {package} is still present on {serial}")]
    NormalPackageStillPresent { serial: String, package: String },
    #[error("full-channel package {package} was not available on {serial} after installation")]
    PackageVerification { serial: String, package: String },
    #[error(transparent)]
    Install(#[from] InstallError),
}

/// Prepares one MuMu instance for full-channel QR login. The ordinary package
/// is uninstalled from every discovered instance, and the full-channel package
/// is the only package this preparer may ever validate for launch.
pub struct MumuLoginPreparer<C, R = SystemCommandRunner>
where
    C: MumuController + ?Sized,
    R: CommandRunner,
{
    controller: Arc<C>,
    runner: Arc<R>,
    market: Arc<AppMarketInstaller>,
    config: MumuConfig,
    adb_program: String,
}

impl<C, R> MumuLoginPreparer<C, R>
where
    C: MumuController + ?Sized,
    R: CommandRunner,
{
    pub fn new(
        controller: Arc<C>,
        runner: Arc<R>,
        market: Arc<AppMarketInstaller>,
        config: MumuConfig,
        adb_program: impl Into<String>,
    ) -> Self
    where
        C: MumuController,
    {
        Self {
            controller,
            runner,
            market,
            config,
            adb_program: adb_program.into(),
        }
    }

    /// Wrap a concrete preparer for storage behind the controller trait object.
    pub fn shared(self) -> Arc<MumuLoginPreparer<dyn MumuController, R>>
    where
        C: MumuController + 'static + Sized,
    {
        Arc::new(MumuLoginPreparer {
            controller: self.controller,
            runner: self.runner,
            market: self.market,
            config: self.config,
            adb_program: self.adb_program,
        })
    }

    pub async fn prepare(
        &self,
        adb_serial: &str,
        oas_config_name: &str,
    ) -> Result<PreparedInstance, PrepareError> {
        let normal = self.config.normal_package.clone();
        let full_channel = self.config.full_channel_package().to_string();

        let mut instances = self.controller.info_all().await?;
        instances.sort_by_key(|instance| instance.index);
        let selected = instances
            .iter()
            .find(|instance| serial_of(instance).as_deref() == Some(adb_serial))
            .cloned()
            .ok_or_else(|| PrepareError::InstanceNotFound(adb_serial.to_string()))?;
        let index = selected.index;

        if !is_ready(&selected) {
            self.controller
                .launch_instance(index)
                .await
                .map_err(|source| PrepareError::Controller { index, source })?;
        }
        wait_adb_online(
            self.runner.as_ref(),
            &self.adb_program,
            adb_serial,
            self.config.launch_wait_timeout,
        )
        .await
        .map_err(|_| PrepareError::Adb {
            serial: adb_serial.to_string(),
            operation: "wait for adb online",
        })?;
        self.controller
            .apply_resolution(index)
            .await
            .map_err(|source| PrepareError::Controller { index, source })?;

        // Ordinary-package cleanup runs on every discovered instance. Instances
        // without a reachable ADB endpoint are skipped and reported; failing to
        // uninstall a reachable one is a hard error.
        let snapshot = self.controller.info_all().await?;
        let mut snapshot = snapshot;
        snapshot.sort_by_key(|instance| instance.index);
        for instance in &snapshot {
            let Some(serial) = serial_of(instance) else {
                tracing::warn!(
                    mumu_index = instance.index,
                    "skipping ordinary-package cleanup: instance has no ADB endpoint"
                );
                continue;
            };
            let packages = query_installed_packages(
                self.runner.as_ref(),
                &self.adb_program,
                &serial,
            )
            .await
            .map_err(|_| PrepareError::Adb {
                serial: serial.clone(),
                operation: "list installed packages",
            })?;
            if packages.iter().any(|package| *package == normal) {
                uninstall_package(self.runner.as_ref(), &self.adb_program, &serial, &normal)
                    .await
                    .map_err(|_| PrepareError::NormalPackageRemoval {
                        serial: serial.clone(),
                        package: normal.clone(),
                    })?;
            }
        }

        let packages = query_installed_packages(self.runner.as_ref(), &self.adb_program, adb_serial)
            .await
            .map_err(|_| PrepareError::Adb {
                serial: adb_serial.to_string(),
                operation: "list installed packages",
            })?;
        if packages.iter().any(|package| *package == normal) {
            return Err(PrepareError::NormalPackageStillPresent {
                serial: adb_serial.to_string(),
                package: normal,
            });
        }
        if !packages.iter().any(|package| *package == full_channel) {
            self.market.install_full_channel(adb_serial).await?;
        }

        let packages = query_installed_packages(self.runner.as_ref(), &self.adb_program, adb_serial)
            .await
            .map_err(|_| PrepareError::Adb {
                serial: adb_serial.to_string(),
                operation: "verify installed packages",
            })?;
        if packages.iter().any(|package| *package == self.config.normal_package) {
            return Err(PrepareError::NormalPackageStillPresent {
                serial: adb_serial.to_string(),
                package: self.config.normal_package.clone(),
            });
        }
        if !packages.iter().any(|package| *package == full_channel) {
            return Err(PrepareError::PackageVerification {
                serial: adb_serial.to_string(),
                package: full_channel,
            });
        }

        Ok(PreparedInstance {
            mumu_index: index,
            adb_serial: adb_serial.to_string(),
            oas_config_name: oas_config_name.to_string(),
            full_channel_package: full_channel,
            launch_wait: self.config.launch_wait_timeout,
            settle_delay: self.config.launch_settle_delay,
        })
    }
}

fn is_ready(instance: &MumuInstanceInfo) -> bool {
    instance.is_android_started
        && instance.is_process_started
        && instance.player_state.as_deref() == Some("start_finished")
}

fn serial_of(instance: &MumuInstanceInfo) -> Option<String> {
    let host = instance.adb_host_ip.as_ref()?;
    let port = instance.adb_port?;
    if host.trim().is_empty() || port == 0 {
        return None;
    }
    Some(format!("{host}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emulator::CommandOutput;
    use crate::mumu::app_market::{FULL_CHANNEL_LABEL, GAME_NAME, INSTALL_LABEL};
    use crate::mumu::MarketUi;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::Mutex as StdMutex;

    const SERIAL: &str = "127.0.0.1:16384";
    const OTHER_SERIAL: &str = "127.0.0.1:16416";
    const NORMAL: &str = "com.netease.onmyoji";
    const FULL: &str = "com.netease.onmyoji.wyzymnqsd_cps";

    fn ready_instance(index: u32, serial_host_port: u16) -> MumuInstanceInfo {
        MumuInstanceInfo {
            index,
            name: format!("MuMu-{index}"),
            adb_host_ip: Some("127.0.0.1".into()),
            adb_port: Some(serial_host_port),
            is_android_started: true,
            is_process_started: true,
            player_state: Some("start_finished".into()),
        }
    }

    fn stopped_instance(index: u32, serial_host_port: u16) -> MumuInstanceInfo {
        MumuInstanceInfo {
            is_android_started: false,
            is_process_started: false,
            player_state: None,
            ..ready_instance(index, serial_host_port)
        }
    }

    #[derive(Default)]
    struct FakeController {
        instances: StdMutex<Vec<MumuInstanceInfo>>,
        calls: StdMutex<Vec<String>>,
        launch_marks_ready: bool,
    }

    impl FakeController {
        fn new(instances: Vec<MumuInstanceInfo>) -> Self {
            Self {
                instances: StdMutex::new(instances),
                calls: StdMutex::new(Vec::new()),
                launch_marks_ready: true,
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl MumuController for FakeController {
        async fn info_all(&self) -> Result<Vec<MumuInstanceInfo>, MumuError> {
            self.calls.lock().unwrap().push("info_all".into());
            Ok(self.instances.lock().unwrap().clone())
        }

        async fn launch_instance(&self, index: u32) -> Result<(), MumuError> {
            self.calls.lock().unwrap().push(format!("launch:{index}"));
            if self.launch_marks_ready {
                for instance in self.instances.lock().unwrap().iter_mut() {
                    if instance.index == index {
                        instance.is_android_started = true;
                        instance.is_process_started = true;
                        instance.player_state = Some("start_finished".into());
                    }
                }
            }
            Ok(())
        }

        async fn apply_resolution(&self, index: u32) -> Result<(), MumuError> {
            self.calls.lock().unwrap().push(format!("resolution:{index}"));
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct FakeRunner {
        calls: Arc<StdMutex<Vec<String>>>,
        outputs: Arc<StdMutex<VecDeque<CommandOutput>>>,
    }

    impl FakeRunner {
        fn with_outputs(outputs: Vec<CommandOutput>) -> Self {
            Self {
                calls: Arc::new(StdMutex::new(Vec::new())),
                outputs: Arc::new(StdMutex::new(outputs.into())),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl CommandRunner for FakeRunner {
        async fn run(
            &self,
            program: &str,
            args: &[String],
        ) -> Result<crate::emulator::CommandOutput, crate::emulator::EmulatorDriverError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{program} {}", args.join(" ")));
            self.outputs
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| crate::emulator::EmulatorDriverError::Message("no output".into()))
        }
    }

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            success: true,
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    fn failed() -> CommandOutput {
        CommandOutput {
            success: false,
            stdout: Vec::new(),
            stderr: b"failure".to_vec(),
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

    #[derive(Default)]
    struct FakeMarketUi {
        calls: StdMutex<Vec<String>>,
        fail_full_channel: bool,
        report_installed_package: Option<String>,
    }

    impl FakeMarketUi {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl MarketUi for FakeMarketUi {
        async fn launch(&self, serial: &str) -> Result<(), InstallError> {
            self.calls.lock().unwrap().push(format!("launch:{serial}"));
            Ok(())
        }

        async fn search(&self, serial: &str, query: &str) -> Result<(), InstallError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("search:{serial}:{query}"));
            Ok(())
        }

        async fn has_text(&self, serial: &str, text: &str) -> Result<bool, InstallError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("has_text:{serial}:{text}"));
            Ok(!(self.fail_full_channel && text == FULL_CHANNEL_LABEL))
        }

        async fn tap_text(&self, serial: &str, text: &str) -> Result<(), InstallError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("tap:{serial}:{text}"));
            Ok(())
        }

        async fn package_installed(&self, serial: &str, package: &str) -> Result<bool, InstallError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("package_installed:{serial}:{package}"));
            Ok(self.report_installed_package.as_deref() == Some(package))
        }
    }

    fn preparer(
        controller: Arc<FakeController>,
        runner: FakeRunner,
        market: Arc<FakeMarketUi>,
    ) -> MumuLoginPreparer<FakeController, FakeRunner> {
        let installer = AppMarketInstaller::new(
            market,
            FULL,
            Duration::from_secs(1),
            Duration::from_millis(1),
        );
        MumuLoginPreparer::new(
            controller,
            Arc::new(runner),
            Arc::new(installer),
            MumuConfig::default(),
            "adb",
        )
    }

    #[tokio::test]
    async fn prepares_resolution_cleanup_and_full_channel_install_in_order() {
        let controller = Arc::new(FakeController::new(vec![
            ready_instance(0, 16384),
            ready_instance(1, 16416),
        ]));
        let runner = FakeRunner::with_outputs(vec![
            ok("device\n"),                          // wait adb online
            packages(&[NORMAL]),                     // instance 0 packages
            ok("Success\n"),                         // uninstall instance 0
            packages(&[NORMAL]),                     // instance 1 packages
            ok("Success\n"),                         // uninstall instance 1
            packages(&[]),                           // decide: nothing installed
            packages(&[FULL]),                       // verify after install
        ]);
        let market = Arc::new(FakeMarketUi {
            report_installed_package: Some(FULL.into()),
            ..FakeMarketUi::default()
        });

        let prepared = preparer(controller.clone(), runner.clone(), market.clone())
            .prepare(SERIAL, "oas-01")
            .await
            .unwrap();

        assert_eq!(prepared.mumu_index, 0);
        assert_eq!(prepared.adb_serial, SERIAL);
        assert_eq!(prepared.oas_config_name, "oas-01");
        assert_eq!(prepared.full_channel_package, FULL);

        assert_eq!(
            controller.calls(),
            vec!["info_all", "resolution:0", "info_all"]
        );
        assert_eq!(
            runner.calls(),
            vec![
                format!("adb -s {SERIAL} get-state"),
                format!("adb -s {SERIAL} shell pm list packages"),
                format!("adb -s {SERIAL} shell pm uninstall {NORMAL}"),
                format!("adb -s {OTHER_SERIAL} shell pm list packages"),
                format!("adb -s {OTHER_SERIAL} shell pm uninstall {NORMAL}"),
                format!("adb -s {SERIAL} shell pm list packages"),
                format!("adb -s {SERIAL} shell pm list packages"),
            ]
        );
        assert_eq!(
            market.calls(),
            vec![
                format!("launch:{SERIAL}"),
                format!("search:{SERIAL}:{GAME_NAME}"),
                format!("tap:{SERIAL}:{GAME_NAME}"),
                format!("has_text:{SERIAL}:{FULL_CHANNEL_LABEL}"),
                format!("tap:{SERIAL}:{FULL_CHANNEL_LABEL}"),
                format!("tap:{SERIAL}:{INSTALL_LABEL}"),
                format!("package_installed:{SERIAL}:{FULL}"),
            ]
        );
    }

    #[tokio::test]
    async fn skips_market_install_when_full_channel_already_present() {
        let controller = Arc::new(FakeController::new(vec![
            ready_instance(0, 16384),
            ready_instance(1, 16416),
        ]));
        let runner = FakeRunner::with_outputs(vec![
            ok("device\n"),
            packages(&[]),   // instance 0 clean
            packages(&[]),   // instance 1 clean
            packages(&[FULL]), // decide: already installed
            packages(&[FULL]), // verify
        ]);
        let market = Arc::new(FakeMarketUi::default());

        preparer(controller, runner.clone(), market.clone())
            .prepare(SERIAL, "oas-01")
            .await
            .unwrap();

        assert!(market.calls().is_empty());
        assert_eq!(runner.calls().len(), 5);
    }

    #[tokio::test]
    async fn fails_when_ordinary_package_cannot_be_removed() {
        let controller = Arc::new(FakeController::new(vec![ready_instance(0, 16384)]));
        let runner = FakeRunner::with_outputs(vec![
            ok("device\n"),
            packages(&[NORMAL]),
            failed(), // uninstall fails
        ]);

        let error = preparer(controller, runner, Arc::new(FakeMarketUi::default()))
            .prepare(SERIAL, "oas-01")
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            PrepareError::NormalPackageRemoval { ref serial, ref package }
                if serial == SERIAL && package == NORMAL
        ));
    }

    #[tokio::test]
    async fn fails_when_ordinary_package_survives_uninstall() {
        let controller = Arc::new(FakeController::new(vec![ready_instance(0, 16384)]));
        let runner = FakeRunner::with_outputs(vec![
            ok("device\n"),
            packages(&[NORMAL]),
            ok("Success\n"),
            packages(&[NORMAL]), // decide: still present
        ]);

        let error = preparer(controller, runner, Arc::new(FakeMarketUi::default()))
            .prepare(SERIAL, "oas-01")
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            PrepareError::NormalPackageStillPresent { .. }
        ));
    }

    #[tokio::test]
    async fn fails_when_market_installs_the_wrong_package() {
        let controller = Arc::new(FakeController::new(vec![ready_instance(0, 16384)]));
        let runner = FakeRunner::with_outputs(vec![
            ok("device\n"),
            packages(&[]),                    // instance clean
            packages(&[]),                    // decide: no full-channel yet
            packages(&["com.other.wrong"]),   // verify: a wrong package, not full-channel
        ]);
        let market = Arc::new(FakeMarketUi {
            report_installed_package: Some(FULL.into()),
            ..FakeMarketUi::default()
        });

        let error = preparer(controller, runner, market)
            .prepare(SERIAL, "oas-01")
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            PrepareError::PackageVerification { ref package, .. } if package == FULL
        ));
    }

    #[tokio::test]
    async fn fails_when_market_has_no_full_channel_option() {
        let controller = Arc::new(FakeController::new(vec![ready_instance(0, 16384)]));
        let runner = FakeRunner::with_outputs(vec![
            ok("device\n"),
            packages(&[]), // clean
            packages(&[]), // decide: nothing installed
        ]);
        let market = Arc::new(FakeMarketUi {
            fail_full_channel: true,
            ..FakeMarketUi::default()
        });

        let error = preparer(controller, runner, market)
            .prepare(SERIAL, "oas-01")
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            PrepareError::Install(InstallError::FullChannelOptionMissing { .. })
        ));
    }

    #[tokio::test]
    async fn launches_stopped_instance_before_preparing() {
        let controller = Arc::new(FakeController::new(vec![stopped_instance(2, 16384)]));
        let runner = FakeRunner::with_outputs(vec![
            ok("device\n"),
            packages(&[]),     // clean
            packages(&[FULL]), // decide: already installed
            packages(&[FULL]), // verify
        ]);

        let prepared = preparer(controller.clone(), runner, Arc::new(FakeMarketUi::default()))
            .prepare(SERIAL, "oas-02")
            .await
            .unwrap();

        assert_eq!(prepared.mumu_index, 2);
        let calls = controller.calls();
        assert!(calls.contains(&"launch:2".to_string()));
        assert!(calls.contains(&"resolution:2".to_string()));
        assert!(calls.iter().position(|call| call == "launch:2")
            < calls.iter().position(|call| call == "resolution:2"));
    }

    #[tokio::test]
    async fn fails_when_serial_is_not_among_discovered_instances() {
        let controller = Arc::new(FakeController::new(vec![ready_instance(0, 16384)]));
        let runner = FakeRunner::with_outputs(vec![]);

        let error = preparer(controller, runner, Arc::new(FakeMarketUi::default()))
            .prepare("127.0.0.1:9999", "oas-01")
            .await
            .unwrap_err();

        assert!(matches!(error, PrepareError::InstanceNotFound(_)));
    }
}
