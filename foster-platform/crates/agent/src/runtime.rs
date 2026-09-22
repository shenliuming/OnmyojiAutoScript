use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use foster_domain::EmulatorStatus;
use foster_protocol::{
    AgentEnvelope, AgentEvent, AgentHello, EmulatorHeartbeat, EmulatorSnapshot, Heartbeat,
    PROTOCOL_VERSION, Pong, ServerCommand, ServerEnvelope, validate_protocol_version,
};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::{
    config::AgentConfig,
    emulator::{EmulatorDriver, EmulatorDriverError},
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
}

pub struct AgentRuntime<D: EmulatorDriver> {
    config: AgentConfig,
    driver: Arc<D>,
}

impl<D: EmulatorDriver> AgentRuntime<D> {
    pub fn new(config: AgentConfig, driver: D) -> Self {
        Self {
            config,
            driver: Arc::new(driver),
        }
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

        let mut heartbeat = tokio::time::interval(self.config.heartbeat_interval);
        heartbeat.tick().await;

        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    self.send_heartbeat(&mut socket).await?;
                }
                incoming = socket.next() => {
                    let Some(message) = incoming else {
                        return Ok(());
                    };
                    let message = message?;

                    match message {
                        Message::Text(text) => {
                            self.handle_server_message(&mut socket, text.as_str())
                                .await?;
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
            ServerCommand::StartLogin(_) | ServerCommand::CancelLogin(_) => {}
        }

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
                capabilities: vec!["EMULATOR_DISCOVERY".to_string()],
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
        let emulators = self
            .driver
            .list_instances()
            .await?
            .into_iter()
            .map(|emulator| EmulatorHeartbeat {
                emulator_code: emulator.emulator_code,
                status: EmulatorStatus::Idle,
            })
            .collect();

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
