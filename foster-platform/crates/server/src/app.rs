use axum::{
    Json, Router,
    routing::get,
};
use serde_json::{Value, json};
use sqlx::MySqlPool;

use crate::{
    agent_gateway::{
        handler::ws_handler,
        registry::AgentRegistry,
    },
    config::AgentGatewayConfig,
};

#[derive(Clone)]
pub struct AppState {
    pub pool: MySqlPool,
    pub registry: AgentRegistry,
    pub gateway_config: AgentGatewayConfig,
}

pub fn build_app(state: AppState) -> Router {
    spawn_stale_sweeper(state.clone());

    Router::new()
        .route("/healthz", get(healthz))
        .route("/agent/ws", get(ws_handler))
        .with_state(state)
}

async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

fn spawn_stale_sweeper(state: AppState) {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(state.gateway_config.sweep_interval);

        loop {
            interval.tick().await;

            for host_id in state
                .registry
                .remove_stale(state.gateway_config.heartbeat_timeout)
            {
                let _ = sqlx::query(
                    "UPDATE host
                     SET status = 'OFFLINE'
                     WHERE id = ?",
                )
                .bind(host_id)
                .execute(&state.pool)
                .await;
            }
        }
    });
}
