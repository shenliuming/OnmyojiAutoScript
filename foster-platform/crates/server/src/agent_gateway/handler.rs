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
use futures_util::StreamExt;
use uuid::Uuid;

use crate::{
    agent_gateway::{auth::is_authorized, registry::AgentPresence},
    app::AppState,
    control_plane::repository::{
        host_exists, mark_host_offline, mark_host_online, touch_host_heartbeat,
        upsert_emulator_snapshot,
    },
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

    state.registry.register(AgentPresence {
        connection_id,
        agent_id: hello.agent_id,
        host_id: hello.host_id,
        connected_at: now,
        last_heartbeat_at: now,
    });

    if mark_host_online(&state.pool, hello.host_id, &hello.agent_version)
        .await
        .is_err()
    {
        state
            .registry
            .remove_if_current(hello.host_id, connection_id);
        return;
    }

    let mut deadline = tokio::time::Instant::now() + state.gateway_config.heartbeat_timeout;

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                break;
            }
            next = socket.next() => {
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
                            AgentEvent::Hello(_) => break,
                            AgentEvent::Pong(_) => {}
                            AgentEvent::LoginPreparing(_)
                            | AgentEvent::LoginQrReady(_)
                            | AgentEvent::LoginQrExpired(_)
                            | AgentEvent::LoginIdentityDetected(_)
                            | AgentEvent::LoginFailed(_) => {}
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
