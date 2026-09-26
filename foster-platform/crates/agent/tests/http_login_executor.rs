use std::{collections::VecDeque, sync::Arc, time::Duration};

use async_trait::async_trait;
use axum::{Json, Router, routing::post};
use foster_agent::{
    emulator::{CommandOutput, CommandRunner, EmulatorDriverError, GenericAdbEmulatorDriver},
    login::{HttpOasLoginExecutor, LoginExecutor},
};
use foster_protocol::StartLoginCommand;
use serde_json::json;
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

fn command() -> StartLoginCommand {
    StartLoginCommand {
        session_no: "LOGIN-REAL".into(),
        game_account_id: 1001,
        emulator_code: "emu-login".into(),
        platform: "ANDROID".into(),
        character_name: "角色A".into(),
        game_uid: "10001".into(),
    }
}

#[tokio::test]
async fn prepare_returns_png_data_url() -> anyhow::Result<()> {
    let runner = FakeRunner::with_outputs(vec![
        CommandOutput {
            success: true,
            stdout: b"device
"
            .to_vec(),
            stderr: Vec::new(),
        },
        CommandOutput {
            success: true,
            stdout: vec![137, 80, 78, 71, 1, 2, 3],
            stderr: Vec::new(),
        },
    ]);
    let driver =
        GenericAdbEmulatorDriver::from_json_with_runner(config_json(), "adb".into(), runner)?;
    let executor = HttpOasLoginExecutor::new(
        driver,
        "http://127.0.0.1:9".into(),
        Duration::from_secs(120),
        Duration::from_secs(1),
        Duration::from_millis(10),
    );

    let prepared = executor.prepare(&command()).await?;

    assert_eq!(prepared.qr_ttl, Duration::from_secs(120));
    assert!(prepared.qr_payload.starts_with("data:image/png;base64,"));
    Ok(())
}

#[tokio::test]
async fn wait_identity_accepts_unique_character_and_server() -> anyhow::Result<()> {
    let app = Router::new().route(
        "/login/detect",
        post(|| async {
            Json(json!({
                "ready": true,
                "ambiguous": false,
                "message": "game identity detected",
                "masked_account": "138****5678",
                "character_name": "角色A",
                "server_name": "春之樱",
                "game_uid": null
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("test server");
    });

    let driver = GenericAdbEmulatorDriver::from_json_with_runner(
        config_json(),
        "adb".into(),
        FakeRunner::default(),
    )?;
    let executor = HttpOasLoginExecutor::new(
        driver,
        format!("http://{addr}"),
        Duration::from_secs(120),
        Duration::from_secs(1),
        Duration::from_millis(10),
    );

    let identity = executor.wait_identity(&command()).await?;

    assert_eq!(identity.masked_account.as_deref(), Some("138****5678"));
    assert_eq!(identity.character_name.as_deref(), Some("角色A"));
    assert_eq!(identity.server_name.as_deref(), Some("春之樱"));

    server.abort();
    Ok(())
}

#[tokio::test]
async fn cancel_stops_identity_polling() -> anyhow::Result<()> {
    let driver = GenericAdbEmulatorDriver::from_json_with_runner(
        config_json(),
        "adb".into(),
        FakeRunner::default(),
    )?;
    let executor = HttpOasLoginExecutor::new(
        driver,
        "http://127.0.0.1:9".into(),
        Duration::from_secs(120),
        Duration::from_secs(30),
        Duration::from_secs(1),
    );
    let command = command();

    executor.cancel(&command.session_no).await?;
    let result = executor.wait_identity(&command).await;

    assert!(result.is_err());
    Ok(())
}
