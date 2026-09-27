use std::{
    collections::HashMap,
    sync::Arc,
    sync::Mutex as StdMutex,
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
    emulator::{EmulatorDriver, EmulatorDriverError, EmulatorLeaseManager},
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
    lease_manager: EmulatorLeaseManager,
    login_tasks: Arc<StdMutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
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
            lease_manager: EmulatorLeaseManager::default(),
            login_tasks: Arc::new(StdMutex::new(HashMap::new())),
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
                self.handle_start_login(command_id, command)?;
            }
            ServerCommand::CancelLogin(command) => {
                if let Some(task) = self.login_tasks.lock().unwrap().remove(&command.session_no) {
                    task.abort();
                }
                if let Some(executor) = &self.login_executor {
                    executor.cancel(&command.session_no).await?;
                }
                self.command_journal.interrupt_login(&command.session_no)?;
            }
            ServerCommand::SelectLoginPlatform(command) => {
                let Some(executor) = self.login_executor.clone() else {
                    self.outbox.push(AgentEvent::LoginFailed(LoginFailed {
                        session_no: command.session_no,
                        code: "LOGIN_EXECUTOR_NOT_CONFIGURED".to_string(),
                        message: "login executor is not configured".to_string(),
                    }));
                    return Ok(());
                };
                let outbox = self.outbox.clone();
                tokio::spawn(async move {
                    let session_no = command.session_no.clone();
                    match executor.select_platform(&command).await {
                        Ok(()) => {
                            outbox.push(AgentEvent::LoginPlatformSelected(
                                foster_protocol::LoginPlatformSelected {
                                    session_no,
                                    platform: command.platform,
                                },
                            ));
                        }
                        Err(error) => {
                            outbox.push(AgentEvent::LoginSelectionRejected(
                                foster_protocol::LoginSelectionRejected {
                                    session_no,
                                    action: "PLATFORM".to_string(),
                                    message: error.to_string(),
                                },
                            ));
                        }
                    }
                });
            }
            ServerCommand::SelectLoginIdentity(command) => {
                let Some(executor) = self.login_executor.clone() else {
                    self.outbox.push(AgentEvent::LoginFailed(LoginFailed {
                        session_no: command.session_no,
                        code: "LOGIN_EXECUTOR_NOT_CONFIGURED".to_string(),
                        message: "login executor is not configured".to_string(),
                    }));
                    return Ok(());
                };
                let outbox = self.outbox.clone();
                let journal = self.command_journal.clone();
                let login_tasks = self.login_tasks.clone();
                tokio::spawn(async move {
                    let session_no = command.session_no.clone();
                    match executor.select_identity(&command).await {
                        Ok(identity) => {
                            let event = AgentEvent::LoginIdentityDetected(LoginIdentityDetected {
                                session_no: session_no.clone(),
                                masked_account: identity.masked_account,
                                character_name: identity.character_name,
                                server_name: identity.server_name,
                                game_uid: None,
                            });
                            let _ = journal.finish_login(&session_no, event.clone());
                            if let Some(task) = login_tasks.lock().unwrap().remove(&session_no) {
                                task.abort();
                            }
                            outbox.push(event);
                        }
                        Err(error) => {
                            outbox.push(AgentEvent::LoginSelectionRejected(
                                foster_protocol::LoginSelectionRejected {
                                    session_no,
                                    action: "IDENTITY".to_string(),
                                    message: error.to_string(),
                                },
                            ));
                        }
                    }
                });
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
        let lease_manager = self.lease_manager.clone();

        tokio::spawn(async move {
            let job_id = command.job_id;
            let attempt = command.attempt;
            let _lease = match lease_manager.try_acquire(&command.emulator_code).await {
                Ok(lease) => lease,
                Err(error) => {
                    let event = AgentEvent::FosterFailed(foster_protocol::FosterFailed {
                        job_id,
                        attempt,
                        failed_at: chrono::Utc::now(),
                        error_code: foster_domain::FosterErrorCode::EmulatorOffline,
                        message: error.to_string(),
                        screenshot_url: None,
                    });
                    let _ = journal.finish(command_id, event.clone());
                    outbox.push(event);
                    return;
                }
            };

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

    fn handle_start_login(
        &self,
        command_id: Uuid,
        command: foster_protocol::StartLoginCommand,
    ) -> Result<(), AgentRuntimeError> {
        match self
            .command_journal
            .begin_login(command_id, &command.session_no)?
        {
            CommandDecision::AlreadyRunning | CommandDecision::Interrupted => return Ok(()),
            CommandDecision::ReplayFinished(event) => {
                self.outbox.push(event);
                return Ok(());
            }
            CommandDecision::StartNew => {}
        }

        let Some(executor) = self.login_executor.clone() else {
            let event = AgentEvent::LoginFailed(LoginFailed {
                session_no: command.session_no,
                code: "LOGIN_EXECUTOR_NOT_CONFIGURED".to_string(),
                message: "login executor is not configured".to_string(),
            });
            self.command_journal.finish(command_id, event.clone())?;
            self.outbox.push(event);
            return Ok(());
        };

        let journal = self.command_journal.clone();
        let outbox = self.outbox.clone();
        let lease_manager = self.lease_manager.clone();
        let login_tasks = self.login_tasks.clone();
        let session_no = command.session_no.clone();
        let task_key = session_no.clone();

        let mut tasks = self.login_tasks.lock().unwrap();
        let task = tokio::spawn(async move {
            outbox.push(AgentEvent::LoginPreparing(LoginPreparing {
                session_no: command.session_no.clone(),
            }));

            let _lease = match lease_manager.try_acquire(&command.emulator_code).await {
                Ok(lease) => lease,
                Err(error) => {
                    let event = AgentEvent::LoginFailed(LoginFailed {
                        session_no: command.session_no,
                        code: "LOGIN_LEASE_FAILED".to_string(),
                        message: error.to_string(),
                    });
                    let _ = journal.finish(command_id, event.clone());
                    outbox.push(event);
                    login_tasks.lock().unwrap().remove(&session_no);
                    return;
                }
            };

            let prepared = match executor.prepare(&command).await {
                Ok(prepared) => prepared,
                Err(error) => {
                    let event = AgentEvent::LoginFailed(LoginFailed {
                        session_no: command.session_no,
                        code: "LOGIN_PREPARE_FAILED".to_string(),
                        message: error.to_string(),
                    });
                    let _ = journal.finish(command_id, event.clone());
                    outbox.push(event);
                    login_tasks.lock().unwrap().remove(&session_no);
                    return;
                }
            };

            let qr_ttl = match chrono::Duration::from_std(prepared.qr_ttl) {
                Ok(value) => value,
                Err(error) => {
                    let event = AgentEvent::LoginFailed(LoginFailed {
                        session_no: command.session_no,
                        code: "LOGIN_QR_TTL_INVALID".to_string(),
                        message: error.to_string(),
                    });
                    let _ = journal.finish(command_id, event.clone());
                    outbox.push(event);
                    login_tasks.lock().unwrap().remove(&session_no);
                    return;
                }
            };

            outbox.push(AgentEvent::LoginQrReady(LoginQrReady {
                session_no: command.session_no.clone(),
                qr_payload: prepared.qr_payload,
                expires_at: chrono::Utc::now() + qr_ttl,
            }));

            let identity =
                match tokio::time::timeout(prepared.qr_ttl, executor.wait_identity(&command)).await
                {
                    Ok(Ok(identity)) => identity,
                    result => {
                        let event = AgentEvent::LoginFailed(LoginFailed {
                            session_no: command.session_no,
                            code: "LOGIN_IDENTITY_FAILED".to_string(),
                            message: match result {
                                Ok(Err(error)) => error.to_string(),
                                Err(_) => {
                                    "login QR expired before identity was detected".to_string()
                                }
                                Ok(Ok(_)) => unreachable!(),
                            },
                        });
                        let _ = journal.finish(command_id, event.clone());
                        outbox.push(event);
                        login_tasks.lock().unwrap().remove(&session_no);
                        return;
                    }
                };

            let event = AgentEvent::LoginIdentityDetected(LoginIdentityDetected {
                session_no: command.session_no,
                masked_account: identity.masked_account,
                character_name: identity.character_name,
                server_name: identity.server_name,
                game_uid: identity.game_uid,
            });
            let _ = journal.finish(command_id, event.clone());
            outbox.push(event);
            login_tasks.lock().unwrap().remove(&session_no);
        });
        tasks.insert(task_key, task);

        Ok(())
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

impl<D: EmulatorDriver> Drop for AgentRuntime<D> {
    fn drop(&mut self) {
        for (_, task) in self.login_tasks.lock().unwrap().drain() {
            task.abort();
        }
    }
}
