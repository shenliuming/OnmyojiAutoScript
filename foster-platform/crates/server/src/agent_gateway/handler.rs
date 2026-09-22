use std::time::Instant;

use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use foster_protocol::{AgentEnvelope, AgentEvent, validate_protocol_version};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    agent_gateway::{auth::is_authorized, registry::AgentPresence},
    app::AppState,
    control_plane::repository::{
        host_exists, mark_host_offline, mark_host_online, touch_host_heartbeat,
        update_emulator_heartbeats, upsert_emulator_snapshot,
    },
    enrollment::EnrollmentService,
    foster_dispatch::FosterDispatchService,
};

pub async fn ws_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !is_authorized(&headers, &state.gateway_config.agent_token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    ws.on_upgrade(move |socket| handle_socket(socket, state))
        .into_response()
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let Some(Ok(Message::Text(text))) = socket.next().await else {
        return;
    };

    let Ok(envelope) = serde_json::from_str::<AgentEnvelope>(text.as_str()) else {
        return;
    };

    if validate_protocol_version(envelope.protocol_version).is_err() {
        return;
    }

    let AgentEvent::Hello(hello) = envelope.payload else {
        return;
    };

    let Ok(exists) = host_exists(&state.pool, hello.host_id).await else {
        return;
    };
    if !exists {
        return;
    }

    let connection_id = Uuid::new_v4();
    let now = Instant::now();
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel();

    state.registry.register_with_sender(
        AgentPresence {
            connection_id,
            agent_id: hello.agent_id,
            host_id: hello.host_id,
            connected_at: now,
            last_heartbeat_at: now,
        },
        outbound_tx,
    );

    if mark_host_online(&state.pool, hello.host_id, &hello.agent_version)
        .await
        .is_err()
    {
        state
            .registry
            .remove_if_current(hello.host_id, connection_id);
        return;
    }

    let (mut socket_sink, mut socket_stream) = socket.split();
    let mut deadline = tokio::time::Instant::now() + state.gateway_config.heartbeat_timeout;

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                break;
            }
            outbound = outbound_rx.recv() => {
                let Some(outbound) = outbound else {
                    break;
                };

                let delivery = match serde_json::to_string(&outbound.envelope) {
                    Ok(json) => socket_sink
                        .send(Message::Text(json.into()))
                        .await
                        .map_err(|error| error.to_string()),
                    Err(error) => Err(error.to_string()),
                };
                let failed = delivery.is_err();
                let _ = outbound.delivered.send(delivery);

                if failed {
                    break;
                }
            }
            next = socket_stream.next() => {
                let Some(message) = next else {
                    break;
                };
                let Ok(message) = message else {
                    break;
                };

                match message {
                    Message::Text(text) => {
                        let Ok(envelope) =
                            serde_json::from_str::<AgentEnvelope>(text.as_str())
                        else {
                            continue;
                        };

                        if validate_protocol_version(envelope.protocol_version).is_err() {
                            break;
                        }

                        match envelope.payload {
                            AgentEvent::Heartbeat(heartbeat) => {
                                if heartbeat.host_id != hello.host_id {
                                    break;
                                }

                                if !state
                                    .registry
                                    .heartbeat(hello.host_id, connection_id)
                                {
                                    break;
                                }

                                if touch_host_heartbeat(&state.pool, hello.host_id)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }

                                if update_emulator_heartbeats(
                                    &state.pool,
                                    hello.host_id,
                                    &heartbeat.emulators,
                                )
                                .await
                                .is_err()
                                {
                                    break;
                                }

                                deadline = tokio::time::Instant::now()
                                    + state.gateway_config.heartbeat_timeout;
                            }
                            AgentEvent::EmulatorSnapshot(snapshot) => {
                                if snapshot.host_id != hello.host_id {
                                    break;
                                }

                                if upsert_emulator_snapshot(
                                    &state.pool,
                                    hello.host_id,
                                    &snapshot.emulators,
                                )
                                .await
                                .is_err()
                                {
                                    break;
                                }
                            }
                            event @ (
                                AgentEvent::LoginPreparing(_)
                                | AgentEvent::LoginQrReady(_)
                                | AgentEvent::LoginQrExpired(_)
                                | AgentEvent::LoginIdentityDetected(_)
                                | AgentEvent::LoginFailed(_)
                            ) => {
                                if EnrollmentService::new(state.pool.clone())
                                    .process_agent_event(hello.host_id, &event)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            event @ (
                                AgentEvent::FosterStageChanged(_)
                                | AgentEvent::FosterSucceeded(_)
                                | AgentEvent::FosterFailed(_)
                            ) => {
                                if FosterDispatchService::new(state.pool.clone())
                                    .process_agent_event(hello.host_id, &event)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            AgentEvent::Hello(_) => break,
                            AgentEvent::Pong(_) => {}
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }

    if state
        .registry
        .remove_if_current(hello.host_id, connection_id)
    {
        let _ = mark_host_offline(&state.pool, hello.host_id).await;
    }
}
