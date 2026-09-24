use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Deserializer};
use thiserror::Error;
use tokio::{process::Command, time::timeout};

const DEFAULT_CLI_PATH: &str = r"C:\Program Files\Netease\MuMu\nx_main\mumu-cli.exe";
const FULL_CHANNEL_PACKAGE: &str = "com.netease.onmyoji.wyzymnqsd_cps";

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
            full_channel_package: FULL_CHANNEL_PACKAGE.into(),
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
        if config.full_channel_package != FULL_CHANNEL_PACKAGE {
            return Err(MumuError::InvalidConfig("FOSTER_FULL_CHANNEL_PACKAGE"));
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
    if let Some(code) = value.get("errcode") {
        let code = code.as_i64().ok_or(MumuError::InvalidOutput)?;
        if code != 0 {
            return Err(MumuError::Protocol(code));
        }
    }
    if let Some(data) = value.get_mut("data") {
        value = data.take();
    } else if let Some(players) = value.get_mut("players") {
        value = players.take();
    }
    let items = match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(map) if map.contains_key("index") => {
            vec![serde_json::Value::Object(map)]
        }
        serde_json::Value::Object(map) => map.into_values().collect(),
        _ => return Err(MumuError::InvalidOutput),
    };
    let mut instances: Vec<MumuInstanceInfo> =
        serde_json::from_value(serde_json::Value::Array(items))
            .map_err(|_| MumuError::InvalidOutput)?;
    instances.sort_by_key(|instance| instance.index);
    Ok(instances)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn parses_local_mumu_keyed_running_and_stopped_instances() {
        // Redacted local `mumu-cli info --vmindex all` output: volatile IDs, handles,
        // timestamps, and disk usage are replaced while preserving field types.
        let json = r#"{
            "0": {"adb_host_ip":"127.0.0.1","adb_port":16384,"android_version":"15.0",
                  "created_timestamp":0,"disk_size_bytes":0,"error_code":0,"hyperv_enabled":true,
                  "index":"0","info_source":"rpc","is_android_started":true,"is_main":false,
                  "is_process_started":true,"launch_err_code":0,"launch_err_msg":"","launch_time":0,
                  "main_wnd":"REDACTED","name":"MuMu安卓设备","pid":0,"player_state":"start_finished",
                  "render_wnd":"REDACTED","vt_enabled":true},
            "1": {"adb_host_ip":"127.0.0.1","adb_port":16416,"android_version":"15.0",
                  "created_timestamp":0,"disk_size_bytes":0,"error_code":0,"hyperv_enabled":true,
                  "index":"1","info_source":"rpc","is_android_started":true,"is_main":false,
                  "is_process_started":true,"name":"MuMu安卓设备-1","pid":0,
                  "player_state":"start_finished","render_wnd":"REDACTED","vt_enabled":true},
            "2": {"android_version":"15.0","created_timestamp":0,"disk_size_bytes":0,
                  "index":"2","info_source":"rpc","is_android_started":false,"is_main":false,
                  "is_process_started":false,"name":"MuMu安卓设备-2"},
            "3": {"android_version":"15.0","created_timestamp":0,"disk_size_bytes":0,
                  "index":"3","is_android_started":false,"is_main":false,
                  "is_process_started":false,"name":"MuMu安卓设备-3"}
        }"#;
        let instances = parse_info_all(json.as_bytes()).unwrap();
        assert_eq!(instances.len(), 4);
        assert_eq!(instances[0].index, 0);
        assert_eq!(instances[0].adb_host_ip.as_deref(), Some("127.0.0.1"));
        assert_eq!(instances[0].adb_port, Some(16384));
        assert!(instances[0].is_android_started);
        assert!(instances[0].is_process_started);
        assert_eq!(instances[0].player_state.as_deref(), Some("start_finished"));
        assert_eq!(instances[1].index, 1);
        assert_eq!(instances[1].adb_port, Some(16416));
        assert_eq!(instances[2].index, 2);
        assert_eq!(instances[2].name, "MuMu安卓设备-2");
        assert_eq!(instances[2].adb_host_ip, None);
        assert_eq!(instances[2].adb_port, None);
        assert!(!instances[2].is_android_started);
        assert_eq!(instances[3].index, 3);
        assert_eq!(instances[3].player_state, None);
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
    fn rejects_protocol_errors() {
        assert_eq!(
            parse_info_all(br#"{"errcode":9,"errmsg":"secret-value"}"#),
            Err(MumuError::Protocol(9))
        );
    }

    #[test]
    fn rejects_invalid_errcode_types_even_with_valid_instance_data() {
        for code in ["\"0\"", "null", "false", "0.0"] {
            let json = format!(
                "{{\"errcode\":{code},\"data\":[{{\"index\":\"0\",\"name\":\"MuMu\",\"is_android_started\":true,\"is_process_started\":true}}]}}"
            );
            assert_eq!(
                parse_info_all(json.as_bytes()),
                Err(MumuError::InvalidOutput)
            );
        }
    }

    #[test]
    fn rejects_any_package_override_other_than_approved_full_channel() {
        for package in ["com.netease.onmyoji", "com.example.other"] {
            let result = MumuConfig::from_lookup(|name| {
                (name == "FOSTER_FULL_CHANNEL_PACKAGE").then(|| package.into())
            });
            assert!(matches!(
                result,
                Err(MumuError::InvalidConfig("FOSTER_FULL_CHANNEL_PACKAGE"))
            ));
        }
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
