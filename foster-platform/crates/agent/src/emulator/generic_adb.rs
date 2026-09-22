use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use foster_domain::EmulatorStatus;
use foster_protocol::EmulatorDescriptor;
use serde::Deserialize;
use tokio::process::Command;

use super::{EmulatorDriver, EmulatorDriverError};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmulatorInstanceConfig {
    pub emulator_code: String,
    pub adb_serial: String,
    pub oas_config_name: String,
    pub package_name: Option<String>,
    pub start_program: Option<String>,
    #[serde(default)]
    pub start_args: Vec<String>,
    pub stop_program: Option<String>,
    #[serde(default)]
    pub stop_args: Vec<String>,
    pub login_prepare_program: Option<String>,
    #[serde(default)]
    pub login_prepare_args: Vec<String>,
    pub login_prepare_delay_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[async_trait]
pub trait CommandRunner: Send + Sync + Clone + 'static {
    async fn run(
        &self,
        program: &str,
        args: &[String],
    ) -> Result<CommandOutput, EmulatorDriverError>;
}

#[derive(Debug, Clone, Default)]
pub struct SystemCommandRunner;

#[async_trait]
impl CommandRunner for SystemCommandRunner {
    async fn run(
        &self,
        program: &str,
        args: &[String],
    ) -> Result<CommandOutput, EmulatorDriverError> {
        let output = Command::new(program)
            .args(args)
            .output()
            .await
            .map_err(|error| {
                EmulatorDriverError::Message(format!(
                    "failed to execute {program}: {error}"
                ))
            })?;

        Ok(CommandOutput {
            success: output.status.success(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

#[derive(Debug, Clone)]
pub struct GenericAdbEmulatorDriver<R = SystemCommandRunner>
where
    R: CommandRunner,
{
    adb_program: String,
    instances: Arc<HashMap<String, EmulatorInstanceConfig>>,
    runner: R,
}

impl GenericAdbEmulatorDriver<SystemCommandRunner> {
    pub fn from_json(
        json: &str,
        adb_program: String,
    ) -> Result<Self, EmulatorDriverError> {
        Self::from_json_with_runner(json, adb_program, SystemCommandRunner)
    }
}

impl<R> GenericAdbEmulatorDriver<R>
where
    R: CommandRunner,
{
    pub fn from_json_with_runner(
        json: &str,
        adb_program: String,
        runner: R,
    ) -> Result<Self, EmulatorDriverError> {
        let configs: Vec<EmulatorInstanceConfig> =
            serde_json::from_str(json).map_err(|error| {
                EmulatorDriverError::Message(format!(
                    "invalid FOSTER_EMULATORS_JSON: {error}"
                ))
            })?;

        if configs.is_empty() {
            return Err(EmulatorDriverError::Message(
                "FOSTER_EMULATORS_JSON must contain at least one emulator".to_string(),
            ));
        }

        let mut instances = HashMap::new();
        for config in configs {
            if config.emulator_code.trim().is_empty()
                || config.adb_serial.trim().is_empty()
                || config.oas_config_name.trim().is_empty()
            {
                return Err(EmulatorDriverError::Message(
                    "emulatorCode, adbSerial and oasConfigName are required".to_string(),
                ));
            }

            if instances
                .insert(config.emulator_code.clone(), config)
                .is_some()
            {
                return Err(EmulatorDriverError::Message(
                    "duplicate emulatorCode in FOSTER_EMULATORS_JSON".to_string(),
                ));
            }
        }

        Ok(Self {
            adb_program,
            instances: Arc::new(instances),
            runner,
        })
    }

    pub fn oas_config_map(&self) -> HashMap<String, String> {
        self.instances
            .iter()
            .map(|(code, config)| {
                (code.clone(), config.oas_config_name.clone())
            })
            .collect()
    }

    pub fn instance_config(
        &self,
        emulator_code: &str,
    ) -> Result<EmulatorInstanceConfig, EmulatorDriverError> {
        self.instances
            .get(emulator_code)
            .cloned()
            .ok_or_else(|| {
                EmulatorDriverError::UnknownInstance(emulator_code.to_string())
            })
    }

    pub async fn prepare_login_screen(
        &self,
        emulator_code: &str,
    ) -> Result<Vec<u8>, EmulatorDriverError> {
        self.start(emulator_code).await?;
        self.launch_game(emulator_code).await?;

        let config = self.instance_config(emulator_code)?;
        if let Some(program) = config.login_prepare_program.as_deref() {
            let output = self.runner.run(program, &config.login_prepare_args).await?;
            if !output.success {
                return Err(command_failure(
                    "login prepare command",
                    program,
                    &output.stderr,
                ));
            }
        }

        let delay = Duration::from_millis(
            config.login_prepare_delay_ms.unwrap_or(3_000),
        );
        tokio::time::sleep(delay).await;

        self.screenshot(emulator_code).await
    }

    pub async fn launch_game(
        &self,
        emulator_code: &str,
    ) -> Result<(), EmulatorDriverError> {
        let config = self.instance_config(emulator_code)?;
        let Some(package_name) = config.package_name else {
            return Ok(());
        };

        let args = vec![
            "-s".to_string(),
            config.adb_serial,
            "shell".to_string(),
            "monkey".to_string(),
            "-p".to_string(),
            package_name,
            "-c".to_string(),
            "android.intent.category.LAUNCHER".to_string(),
            "1".to_string(),
        ];
        let output = self.runner.run(&self.adb_program, &args).await?;
        if !output.success {
            return Err(command_failure(
                "launch game",
                &self.adb_program,
                &output.stderr,
            ));
        }

        Ok(())
    }

    async fn wait_for_adb(
        &self,
        serial: &str,
    ) -> Result<(), EmulatorDriverError> {
        for _ in 0..20 {
            let args = vec![
                "-s".to_string(),
                serial.to_string(),
                "get-state".to_string(),
            ];
            let output = self.runner.run(&self.adb_program, &args).await?;
            if output.success
                && String::from_utf8_lossy(&output.stdout).trim() == "device"
            {
                return Ok(());
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        Err(EmulatorDriverError::Message(format!(
            "adb device {serial} did not become ready"
        )))
    }
}

#[async_trait]
impl<R> EmulatorDriver for GenericAdbEmulatorDriver<R>
where
    R: CommandRunner,
{
    async fn list_instances(
        &self,
    ) -> Result<Vec<EmulatorDescriptor>, EmulatorDriverError> {
        let mut values = self
            .instances
            .values()
            .map(|config| EmulatorDescriptor {
                emulator_code: config.emulator_code.clone(),
                driver_type: "ADB".to_string(),
                adb_serial: Some(config.adb_serial.clone()),
            })
            .collect::<Vec<_>>();
        values.sort_by(|left, right| {
            left.emulator_code.cmp(&right.emulator_code)
        });
        Ok(values)
    }

    async fn start(
        &self,
        instance_id: &str,
    ) -> Result<(), EmulatorDriverError> {
        let config = self.instance_config(instance_id)?;

        if let Some(program) = config.start_program.as_deref() {
            let output = self.runner.run(program, &config.start_args).await?;
            if !output.success {
                return Err(command_failure(
                    "emulator start command",
                    program,
                    &output.stderr,
                ));
            }
        }

        self.wait_for_adb(&config.adb_serial).await
    }

    async fn stop(
        &self,
        instance_id: &str,
    ) -> Result<(), EmulatorDriverError> {
        let config = self.instance_config(instance_id)?;

        if let Some(program) = config.stop_program.as_deref() {
            let output = self.runner.run(program, &config.stop_args).await?;
            if !output.success {
                return Err(command_failure(
                    "emulator stop command",
                    program,
                    &output.stderr,
                ));
            }
        }

        Ok(())
    }

    async fn adb_serial(
        &self,
        instance_id: &str,
    ) -> Result<Option<String>, EmulatorDriverError> {
        Ok(Some(self.instance_config(instance_id)?.adb_serial))
    }

    async fn status(
        &self,
        instance_id: &str,
    ) -> Result<EmulatorStatus, EmulatorDriverError> {
        let config = self.instance_config(instance_id)?;
        let args = vec![
            "-s".to_string(),
            config.adb_serial,
            "get-state".to_string(),
        ];

        match self.runner.run(&self.adb_program, &args).await {
            Ok(output)
                if output.success
                    && String::from_utf8_lossy(&output.stdout).trim() == "device" =>
            {
                Ok(EmulatorStatus::Idle)
            }
            Ok(_) => Ok(EmulatorStatus::Offline),
            Err(_) => Ok(EmulatorStatus::Offline),
        }
    }

    async fn screenshot(
        &self,
        instance_id: &str,
    ) -> Result<Vec<u8>, EmulatorDriverError> {
        let config = self.instance_config(instance_id)?;
        let args = vec![
            "-s".to_string(),
            config.adb_serial,
            "exec-out".to_string(),
            "screencap".to_string(),
            "-p".to_string(),
        ];
        let output = self.runner.run(&self.adb_program, &args).await?;
        if !output.success || output.stdout.is_empty() {
            return Err(command_failure(
                "adb screenshot",
                &self.adb_program,
                &output.stderr,
            ));
        }

        Ok(output.stdout)
    }
}

fn command_failure(
    operation: &str,
    program: &str,
    stderr: &[u8],
) -> EmulatorDriverError {
    let stderr = String::from_utf8_lossy(stderr);
    EmulatorDriverError::Message(format!(
        "{operation} failed via {program}: {}",
        stderr.trim()
    ))
}
