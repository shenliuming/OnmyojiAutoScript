use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Deserializer};
use thiserror::Error;
use tokio::{process::Command, time::timeout};

const DEFAULT_CLI_PATH: &str = r"C:\Program Files\Netease\MuMu\nx_main\mumu-cli.exe";

#[derive(Debug, Clone)]
pub struct MumuConfig {
    pub cli_path: PathBuf,
    pub app_market_package: String,
    pub app_market_activity: String,
    pub normal_package: String,
    pub full_channel_package: String,
    pub resolution_width: u32,
    pub resolution_height: u32,
    pub command_timeout: Duration,
}

impl Default for MumuConfig {
    fn default() -> Self {
        Self {
            cli_path: DEFAULT_CLI_PATH.into(),
            app_market_package: "com.mumu.store".into(),
            app_market_activity: "com.mumu.store/.MainActivity".into(),
            normal_package: "com.netease.onmyoji".into(),
            full_channel_package: "com.netease.onmyoji.wyzymnqsd_cps".into(),
            resolution_width: 1280,
            resolution_height: 720,
            command_timeout: Duration::from_secs(30),
        }
    }
}

impl MumuConfig {
    pub fn from_env() -> Result<Self, MumuError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(mut get: impl FnMut(&str) -> Option<String>) -> Result<Self, MumuError> {
        let mut config = Self::default();
        if let Some(value) = get("FOSTER_MUMU_CLI_PATH") {
            config.cli_path = PathBuf::from(nonempty(value, "FOSTER_MUMU_CLI_PATH")?);
        }
        for (name, target) in [
            (
                "FOSTER_MUMU_APP_MARKET_PACKAGE",
                &mut config.app_market_package,
            ),
            (
                "FOSTER_MUMU_APP_MARKET_ACTIVITY",
                &mut config.app_market_activity,
            ),
            ("FOSTER_NORMAL_PACKAGE", &mut config.normal_package),
            (
                "FOSTER_FULL_CHANNEL_PACKAGE",
                &mut config.full_channel_package,
            ),
        ] {
            if let Some(value) = get(name) {
                *target = nonempty(value, name)?;
            }
        }
        config.resolution_width =
            positive_u32(&mut get, "FOSTER_RESOLUTION_WIDTH", config.resolution_width)?;
        config.resolution_height = positive_u32(
            &mut get,
            "FOSTER_RESOLUTION_HEIGHT",
            config.resolution_height,
        )?;
        let seconds = positive_u64(
            &mut get,
            "FOSTER_MUMU_COMMAND_TIMEOUT_SECONDS",
            config.command_timeout.as_secs(),
        )?;
        config.command_timeout = Duration::from_secs(seconds);
        Ok(config)
    }
}

fn nonempty(value: String, name: &'static str) -> Result<String, MumuError> {
    if value.trim().is_empty() {
        return Err(MumuError::InvalidConfig(name));
    }
    Ok(value)
}

fn positive_u32(
    get: &mut impl FnMut(&str) -> Option<String>,
    name: &'static str,
    default: u32,
) -> Result<u32, MumuError> {
    match get(name) {
        Some(value) => value
            .parse::<u32>()
            .ok()
            .filter(|number| *number > 0)
            .ok_or(MumuError::InvalidConfig(name)),
        None => Ok(default),
    }
}

fn positive_u64(
    get: &mut impl FnMut(&str) -> Option<String>,
    name: &'static str,
    default: u64,
) -> Result<u64, MumuError> {
    match get(name) {
        Some(value) => value
            .parse::<u64>()
            .ok()
            .filter(|number| *number > 0)
            .ok_or(MumuError::InvalidConfig(name)),
        None => Ok(default),
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MumuError {
    #[error("invalid MuMu configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("MuMu CLI could not start ({0:?})")]
    Spawn(std::io::ErrorKind),
    #[error("MuMu CLI command timed out")]
    Timeout,
    #[error("MuMu CLI command failed with exit code {code:?}")]
    NonZeroExit { code: Option<i32> },
    #[error("MuMu CLI returned malformed JSON")]
    InvalidJson,
    #[error("MuMu CLI returned invalid instance information")]
    InvalidOutput,
    #[error("MuMu CLI returned error code {0}")]
    Protocol(i64),
}

#[derive(Clone)]
pub struct CommandOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
}

impl std::fmt::Debug for CommandOutput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CommandOutput")
            .field("stdout_bytes", &self.stdout.len())
            .field("stderr_bytes", &self.stderr.len())
            .field("exit_code", &self.exit_code)
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MumuInstanceInfo {
    #[serde(deserialize_with = "deserialize_index")]
    pub index: u32,
    pub name: String,
    #[serde(default)]
    pub adb_host_ip: Option<String>,
    #[serde(default)]
    pub adb_port: Option<u16>,
    pub is_android_started: bool,
    pub is_process_started: bool,
    #[serde(default)]
    pub player_state: Option<String>,
}

fn deserialize_index<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Index {
        Text(String),
        Number(u32),
    }
    match Index::deserialize(deserializer)? {
        Index::Text(value) => value.parse().map_err(serde::de::Error::custom),
        Index::Number(value) => Ok(value),
    }
}

#[derive(Debug, Clone)]
pub struct MumuCli {
    config: MumuConfig,
}

impl MumuCli {
    pub fn new(config: MumuConfig) -> Self {
        Self { config }
    }

    pub async fn run(&self, args: &[&str]) -> Result<CommandOutput, MumuError> {
        let mut command = Command::new(&self.config.cli_path);
        command.args(args).kill_on_drop(true);
        let output = timeout(self.config.command_timeout, command.output())
            .await
            .map_err(|_| MumuError::Timeout)?
            .map_err(|error| MumuError::Spawn(error.kind()))?;
        if !output.status.success() {
            return Err(MumuError::NonZeroExit {
                code: output.status.code(),
            });
        }
        Ok(CommandOutput {
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code: output.status.code(),
        })
    }

    pub async fn info_all(&self) -> Result<Vec<MumuInstanceInfo>, MumuError> {
        let output = self.run(&["info", "--vmindex", "all"]).await?;
        parse_info_all(&output.stdout)
    }
}

pub fn parse_info_all(output: &[u8]) -> Result<Vec<MumuInstanceInfo>, MumuError> {
    let mut value: serde_json::Value =
        serde_json::from_slice(output).map_err(|_| MumuError::InvalidJson)?;
    if let Some(code) = value.get("errcode").and_then(|value| value.as_i64()) {
        if code != 0 {
            return Err(MumuError::Protocol(code));
        }
    }
    if let Some(data) = value.get_mut("data") {
        value = data.take();
    } else if let Some(players) = value.get_mut("players") {
        value = players.take();
    }
    if value.is_object() && value.get("index").is_some() {
        value = serde_json::Value::Array(vec![value]);
    }
    serde_json::from_value(value).map_err(|_| MumuError::InvalidOutput)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn parses_running_and_stopped_instances() {
        let json = br#"[
            {"index":"0","name":"MuMu 0","adb_host_ip":"127.0.0.1","adb_port":16384,
             "is_android_started":true,"is_process_started":true,"player_state":"running"},
            {"index":"2","name":"MuMu 2","is_android_started":false,"is_process_started":false}
        ]"#;
        let instances = parse_info_all(json).unwrap();
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].index, 0);
        assert_eq!(instances[0].adb_host_ip.as_deref(), Some("127.0.0.1"));
        assert_eq!(instances[0].adb_port, Some(16384));
        assert!(instances[0].is_android_started);
        assert!(instances[0].is_process_started);
        assert_eq!(instances[0].player_state.as_deref(), Some("running"));
        assert_eq!(instances[1].index, 2);
        assert_eq!(instances[1].name, "MuMu 2");
        assert_eq!(instances[1].adb_host_ip, None);
        assert_eq!(instances[1].adb_port, None);
    }

    #[test]
    fn rejects_malformed_or_incomplete_json_without_echoing_it() {
        for json in [
            br#"{"token":"secret-value""#.as_slice(),
            br#"[{"index":"0"}]"#,
        ] {
            let error = parse_info_all(json).unwrap_err();
            assert!(!error.to_string().contains("secret-value"));
        }
    }

    #[test]
    fn parses_wrapped_instances_and_rejects_protocol_errors() {
        let json = br#"{"errcode":0,"data":[{"index":1,"name":"MuMu 1","is_android_started":false,"is_process_started":true}]}"#;
        assert_eq!(parse_info_all(json).unwrap()[0].index, 1);
        assert_eq!(
            parse_info_all(br#"{"errcode":9,"errmsg":"secret-value"}"#),
            Err(MumuError::Protocol(9))
        );
    }

    #[test]
    fn output_debug_redacts_command_streams() {
        let output = CommandOutput {
            stdout: b"secret-value".to_vec(),
            stderr: b"qr-image".to_vec(),
            exit_code: Some(0),
        };
        let debug = format!("{output:?}");
        assert!(!debug.contains("secret-value"));
        assert!(!debug.contains("qr-image"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn nonzero_exit_reports_code_without_output() {
        let cli = MumuCli::new(MumuConfig {
            cli_path: "cmd.exe".into(),
            command_timeout: Duration::from_secs(5),
            ..MumuConfig::default()
        });
        let error = cli
            .run(&["/C", "echo secret-value & exit /B 7"])
            .await
            .unwrap_err();
        assert!(matches!(error, MumuError::NonZeroExit { code: Some(7) }));
        assert!(!error.to_string().contains("secret-value"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn timeout_reports_timeout_without_command_args() {
        let cli = MumuCli::new(MumuConfig {
            cli_path: "powershell.exe".into(),
            command_timeout: Duration::from_millis(20),
            ..MumuConfig::default()
        });
        let error = cli
            .run(&[
                "-NoProfile",
                "-Command",
                "Start-Sleep -Seconds 2; echo secret-value",
            ])
            .await
            .unwrap_err();
        assert!(matches!(error, MumuError::Timeout));
        assert!(!error.to_string().contains("secret-value"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn nonzero_exit_reports_code_without_output() {
        let cli = MumuCli::new(MumuConfig {
            cli_path: "/bin/sh".into(),
            command_timeout: Duration::from_secs(5),
            ..MumuConfig::default()
        });
        let error = cli
            .run(&["-c", "echo secret-value; exit 7"])
            .await
            .unwrap_err();
        assert!(matches!(error, MumuError::NonZeroExit { code: Some(7) }));
        assert!(!error.to_string().contains("secret-value"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_reports_timeout_without_command_args() {
        let cli = MumuCli::new(MumuConfig {
            cli_path: "/bin/sh".into(),
            command_timeout: Duration::from_millis(20),
            ..MumuConfig::default()
        });
        let error = cli
            .run(&["-c", "sleep 1; echo secret-value"])
            .await
            .unwrap_err();
        assert!(matches!(error, MumuError::Timeout));
        assert!(!error.to_string().contains("secret-value"));
    }

    #[test]
    fn environment_overrides_defaults_and_rejects_invalid_dimensions() {
        let config = MumuConfig::from_lookup(|name| match name {
            "FOSTER_RESOLUTION_WIDTH" => Some("1280".into()),
            "FOSTER_RESOLUTION_HEIGHT" => Some("720".into()),
            "FOSTER_MUMU_COMMAND_TIMEOUT_SECONDS" => Some("45".into()),
            "FOSTER_FULL_CHANNEL_PACKAGE" => Some("com.netease.onmyoji.wyzymnqsd_cps".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(config.resolution_width, 1280);
        assert_eq!(config.resolution_height, 720);
        assert_eq!(config.command_timeout, Duration::from_secs(45));
        assert_eq!(config.app_market_package, "com.mumu.store");
        assert_eq!(config.app_market_activity, "com.mumu.store/.MainActivity");
        assert_eq!(config.normal_package, "com.netease.onmyoji");
        assert_eq!(
            config.full_channel_package,
            "com.netease.onmyoji.wyzymnqsd_cps"
        );
        assert!(
            MumuConfig::from_lookup(|name| (name == "FOSTER_RESOLUTION_WIDTH").then(|| "0".into()))
                .is_err()
        );
    }
}
