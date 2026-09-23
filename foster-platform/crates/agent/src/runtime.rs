use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use foster_domain::EmulatorStatus;
use foster_protocol::{
    AgentEnvelope, AgentEvent, AgentHello, EmulatorHeartbeat, EmulatorSnapshot, Heartbeat,
    LoginFailed, LoginIdentityDetected, LoginPreparing, LoginQrReady, PROTOCOL_VERSION, Pong,
    ServerCommand, ServerEnvelope, validate_protocol_version,
};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::{
    command_journal::{CommandDecision, CommandJournal, CommandJournalError},
    config::AgentConfig,
    emulator::{EmulatorDriver, EmulatorDriverError},
    foster::{FosterExecutor, FosterExecutorError, events_for_execution},
    login::{LoginExecutor, LoginExecutorError},
    outbox::AgentEventOutbox,
    ws::{AgentWebSocket, WsClientError, connect},
};

#[derive(Debug, thiserror::Error)]
pub enum AgentRuntimeError {
    #[error(transparent)]
    WebSocket(#[from] WsClientError),
    #[error(transparent)]
    ProtocolWebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error(transparent)]
    Serialize(#[from] serde_json::Error),
    #[error(transparent)]
    Driver(#[from] EmulatorDriverError),
    #[error(transparent)]
    LoginExecutor(#[from] LoginExecutorError),
    #[error(transparent)]
    FosterExecutor(#[from] FosterExecutorError),
    #[error(transparent)]
    CommandJournal(#[from] CommandJournalError),
}

pub struct AgentRuntime<D: EmulatorDriver> {
    config: AgentConfig,
    driver: Arc<D>,
    login_executor: Option<Arc<dyn LoginExecutor>>,
    foster_executor: Option<Arc<dyn FosterExecutor>>,
    command_journal: CommandJournal,
    outbox: AgentEventOutbox,
}

impl<D: EmulatorDriver> AgentRuntime<D> {
    pub fn new(config: AgentConfig, driver: D) -> Self {
        Self {
            config,
            driver: Arc::new(driver),
            login_executor: None,
            foster_executor: None,
            command_journal: CommandJournal::in_memory(),
            outbox: AgentEventOutbox::default(),
        }
    }

    pub fn with_login_executor<E: LoginExecutor>(mut self, executor: E) -> Self {
        self.login_executor = Some(Arc::new(executor));
        self
    }

    pub fn with_foster_executor<E: FosterExecutor>(mut self, executor: E) -> Self {
        self.foster_executor = Some(Arc::new(executor));
        self
    }

    pub fn with_command_journal(mut self, command_journal: CommandJournal) -> Self {
        self.command_journal = command_journal;
        self
    }

    pub async fn run(self) -> Result<(), AgentRuntimeError> {
        let mut reconnect_index = 0usize;

        loop {
            let connected_at = Instant::now();
            let _ = self.run_connection().await;

            if connected_at.elapsed() >= self.config.heartbeat_interval {
                reconnect_index = 0;
            }

            let delay = self.reconnect_delay(reconnect_index);
            tokio::time::sleep(delay).await;

            reconnect_index = reconnect_index
                .saturating_add(1)
                .min(self.config.reconnect_delays.len().saturating_sub(1));
        }
    }

    async fn run_connection(&self) -> Result<(), AgentRuntimeError> {
        let mut socket = connect(&self.config).await?;

        self.send_hello(&mut socket).await?;
        self.send_snapshot(&mut socket).await?;
        self.send_heartbeat(&mut socket).await?;

        for event in self.command_journal.finished_events()? {
            self.outbox.push(event);
        }

        let mut heartbeat = tokio::time::interval(self.config.heartbeat_interval);
        heartbeat.tick().await;

        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    self.send_heartbeat(&mut socket).await?;
                }
                event = self.outbox.recv() => {
                    if let Err(error) = self.send_event(&mut socket, event.clone()).await {
                        self.outbox.requeue_front(event);
                        return Err(error);
                    }
                }
                incoming = socket.next() => {
                    let Some(message) = incoming else {
                        return Ok(());
                    };
                    let message = message?;

                    match message {
                        Message::Text(text) => {
                            self.handle_server_message(&mut socket, text.as_str()).await?;
                        }
                        Message::Close(_) => return Ok(()),
                        Message::Ping(payload) => {
                            socket.send(Message::Pong(payload)).await?;
                        }
                        Message::Binary(_) | Message::Pong(_) | Message::Frame(_) => {}
                    }
                }
            }
        }
    }

    async fn handle_server_message(
        &self,
        socket: &mut AgentWebSocket,
        text: &str,
    ) -> Result<(), AgentRuntimeError> {
        let envelope: ServerEnvelope = serde_json::from_str(text)?;

        if validate_protocol_version(envelope.protocol_version).is_err() {
            return Ok(());
        }

        let command_id = envelope.command_id;

        match envelope.payload {
            ServerCommand::Ping(command) => {
                self.send_event(
                    socket,
                    AgentEvent::Pong(Pong {
                        nonce: command.nonce,
                    }),
                )
                .await?;
            }
            ServerCommand::RefreshEmulators(_) => {
                self.send_snapshot(socket).await?;
            }
            ServerCommand::StartLogin(command) => {
                self.spawn_login_execution(command);
            }
            ServerCommand::CancelLogin(command) => {
                if let Some(executor) = &self.login_executor {
                    executor.cancel(&command.session_no).await?;
                }
            }
            ServerCommand::ExecuteFoster(command) => {
                self.handle_execute_foster(command_id, command)?;
            }
        }

        Ok(())
    }

    fn handle_execute_foster(
        &self,
        command_id: Uuid,
        command: foster_protocol::ExecuteFosterCommand,
    ) -> Result<(), AgentRuntimeError> {
        match self
            .command_journal
            .begin_foster(command_id, command.job_id, command.attempt)?
        {
            CommandDecision::AlreadyRunning | CommandDecision::Interrupted => return Ok(()),
            CommandDecision::ReplayFinished(event) => {
                self.outbox.push(event);
                return Ok(());
            }
            CommandDecision::StartNew => {}
        }

        let Some(executor) = self.foster_executor.clone() else {
            let event = AgentEvent::FosterFailed(foster_protocol::FosterFailed {
                job_id: command.job_id,
                attempt: command.attempt,
                failed_at: chrono::Utc::now(),
                error_code: foster_domain::FosterErrorCode::EmulatorOffline,
                message: "foster executor is not configured".to_string(),
                screenshot_url: None,
            });
            self.command_journal.finish(command_id, event.clone())?;
            self.outbox.push(event);
            return Ok(());
        };

        let journal = self.command_journal.clone();
        let outbox = self.outbox.clone();

        tokio::spawn(async move {
            let job_id = command.job_id;
            let attempt = command.attempt;

            let execution = match executor.execute(&command).await {
                Ok(execution) => execution,
                Err(error) => {
                    let event = AgentEvent::FosterFailed(foster_protocol::FosterFailed {
                        job_id,
                        attempt,
                        failed_at: chrono::Utc::now(),
                        error_code: foster_domain::FosterErrorCode::NetworkError,
                        message: format!("foster executor failed: {error}"),
                        screenshot_url: None,
                    });
                    let _ = journal.finish(command_id, event.clone());
                    outbox.push(event);
                    return;
                }
            };

            for event in events_for_execution(job_id, attempt, execution) {
                if matches!(
                    event,
                    AgentEvent::FosterSucceeded(_) | AgentEvent::FosterFailed(_)
                ) {
                    let _ = journal.finish(command_id, event.clone());
                }
                outbox.push(event);
            }
        });

        Ok(())
    }

    fn spawn_login_execution(
        &self,
        command: foster_protocol::StartLoginCommand,
    ) {
        let event_tx = self.outbox.clone();
        let Some(executor) = self.login_executor.clone() else {
            event_tx.push(AgentEvent::LoginFailed(LoginFailed {
                session_no: command.session_no,
                code: "LOGIN_EXECUTOR_NOT_CONFIGURED".to_string(),
                message: "login executor is not configured".to_string(),
            }));
            return;
        };

        tokio::spawn(async move {
            event_tx.push(AgentEvent::LoginPreparing(LoginPreparing {
                session_no: command.session_no.clone(),
            }));

            let prepared = match executor.prepare(&command).await {
                Ok(prepared) => prepared,
                Err(error) => {
                    event_tx.push(AgentEvent::LoginFailed(LoginFailed {
                        session_no: command.session_no,
                        code: "LOGIN_PREPARE_FAILED".to_string(),
                        message: error.to_string(),
                    }));
                    return;
                }
            };

            let qr_ttl = match chrono::Duration::from_std(prepared.qr_ttl) {
                Ok(value) => value,
                Err(error) => {
                    event_tx.push(AgentEvent::LoginFailed(LoginFailed {
                        session_no: command.session_no,
                        code: "LOGIN_QR_TTL_INVALID".to_string(),
                        message: error.to_string(),
                    }));
                    return;
                }
            };

            event_tx.push(AgentEvent::LoginQrReady(LoginQrReady {
                session_no: command.session_no.clone(),
                qr_payload: prepared.qr_payload,
                expires_at: chrono::Utc::now() + qr_ttl,
            }));

            let identity = match executor.wait_identity(&command).await {
                Ok(identity) => identity,
                Err(error) => {
                    event_tx.push(AgentEvent::LoginFailed(LoginFailed {
                        session_no: command.session_no,
                        code: "LOGIN_IDENTITY_FAILED".to_string(),
                        message: error.to_string(),
                    }));
                    return;
                }
            };

            event_tx.push(AgentEvent::LoginIdentityDetected(LoginIdentityDetected {
                session_no: command.session_no,
                masked_account: identity.masked_account,
                character_name: identity.character_name,
                server_name: identity.server_name,
                game_uid: identity.game_uid,
            }));
        });
    }

    async fn send_hello(&self, socket: &mut AgentWebSocket) -> Result<(), AgentRuntimeError> {
        let hostname = std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "unknown-host".to_string());

        self.send_event(
            socket,
            AgentEvent::Hello(AgentHello {
                agent_id: self.config.agent_id.clone(),
                host_id: self.config.host_id,
                agent_version: env!("CARGO_PKG_VERSION").to_string(),
                hostname,
                os_version: std::env::consts::OS.to_string(),
                capabilities: vec![
                    "EMULATOR_DISCOVERY".to_string(),
                    "LOGIN_EXECUTION".to_string(),
                    "FOSTER_EXECUTION".to_string(),
                    "COMMAND_JOURNAL_V1".to_string(),
                ],
                command_states: self.command_journal.states()?,
            }),
        )
        .await
    }

    async fn send_snapshot(&self, socket: &mut AgentWebSocket) -> Result<(), AgentRuntimeError> {
        let emulators = self.driver.list_instances().await?;

        self.send_event(
            socket,
            AgentEvent::EmulatorSnapshot(EmulatorSnapshot {
                host_id: self.config.host_id,
                emulators,
            }),
        )
        .await
    }

    async fn send_heartbeat(&self, socket: &mut AgentWebSocket) -> Result<(), AgentRuntimeError> {
        let mut emulators = Vec::new();
        for emulator in self.driver.list_instances().await? {
            let status = self
                .driver
                .status(&emulator.emulator_code)
                .await
                .unwrap_or(EmulatorStatus::Error);
            emulators.push(EmulatorHeartbeat {
                emulator_code: emulator.emulator_code,
                status,
            });
        }

        self.send_event(
            socket,
            AgentEvent::Heartbeat(Heartbeat {
                host_id: self.config.host_id,
                emulators,
            }),
        )
        .await
    }

    async fn send_event(
        &self,
        socket: &mut AgentWebSocket,
        payload: AgentEvent,
    ) -> Result<(), AgentRuntimeError> {
        let envelope = AgentEnvelope {
            protocol_version: PROTOCOL_VERSION,
            event_id: Uuid::new_v4(),
            sent_at: chrono::Utc::now(),
            payload,
        };

        let text = serde_json::to_string(&envelope)?;
        socket.send(Message::Text(text.into())).await?;
        Ok(())
    }

    fn reconnect_delay(&self, index: usize) -> Duration {
        self.config
            .reconnect_delays
            .get(index)
            .copied()
            .or_else(|| self.config.reconnect_delays.last().copied())
            .unwrap_or_else(|| Duration::from_secs(1))
    }
}
