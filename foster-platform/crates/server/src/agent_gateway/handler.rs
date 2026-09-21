use std::time::Instant;

use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use foster_protocol::{
    AgentEnvelope, AgentEvent, validate_protocol_version,
};
use futures_util::StreamExt;
use uuid::Uuid;

use crate::{
    agent_gateway::{
        auth::is_authorized,
        registry::AgentPresence,
    },
    app::AppState,
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

    let Ok(exists) = host_exists(&state, hello.host_id).await else {
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

    if mark_host_online(&state, hello.host_id, &hello.agent_version)
        .await
        .is_err()
    {
        state
            .registry
            .remove_if_current(hello.host_id, connection_id);
        return;
    }

    let mut deadline =
        tokio::time::Instant::now() + state.gateway_config.heartbeat_timeout;

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

                        if let AgentEvent::Heartbeat(heartbeat) = envelope.payload {
                            if heartbeat.host_id != hello.host_id {
                                break;
                            }

                            if state
                                .registry
                                .heartbeat(hello.host_id, connection_id)
                            {
                                deadline = tokio::time::Instant::now()
                                    + state.gateway_config.heartbeat_timeout;
                            } else {
                                break;
                            }
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
        let _ = mark_host_offline(&state, hello.host_id).await;
    }
}

async fn host_exists(state: &AppState, host_id: i64) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM host WHERE id = ?",
    )
    .bind(host_id)
    .fetch_one(&state.pool)
    .await?;

    Ok(count > 0)
}

async fn mark_host_online(
    state: &AppState,
    host_id: i64,
    agent_version: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE host
         SET status = 'ONLINE',
             agent_version = ?,
             last_heartbeat_at = NOW(3)
         WHERE id = ?",
    )
    .bind(agent_version)
    .bind(host_id)
    .execute(&state.pool)
    .await?;

    Ok(())
}

async fn mark_host_offline(
    state: &AppState,
    host_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE host
         SET status = 'OFFLINE'
         WHERE id = ?",
    )
    .bind(host_id)
    .execute(&state.pool)
    .await?;

    Ok(())
}
