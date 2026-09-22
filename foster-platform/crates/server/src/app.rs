use axum::{
    Extension, Json, Router,
    routing::{get, post},
};
use serde_json::{Value, json};
use sqlx::MySqlPool;

use crate::{
    agent_gateway::{handler::ws_handler, registry::AgentRegistry},
    config::AgentGatewayConfig,
    enrollment::{
        public_api::{confirm_login, get_public_login},
        sse::login_status_events,
    },
    foster_dispatch::FosterDispatchService,
    onboarding::{
        AdminAuthConfig, admin_onboard, login_page, service_page,
    },
    public_portal::{clear_pause, get_service_status, pause_service, replace_quiet_periods},
    resource_pool::ResourcePoolService,
    scheduler::SchedulerService,
};

#[derive(Clone)]
pub struct AppState {
    pub pool: MySqlPool,
    pub registry: AgentRegistry,
    pub gateway_config: AgentGatewayConfig,
}

pub fn build_app(state: AppState) -> Router {
    let admin_token = std::env::var("FOSTER_ADMIN_TOKEN").ok();
    build_app_with_admin_token(state, admin_token)
}

pub fn build_app_with_admin_token(
    state: AppState,
    admin_token: Option<String>,
) -> Router {
    spawn_stale_sweeper(state.clone());
    spawn_foster_scheduler(state.clone());

    Router::new()
        .route("/healthz", get(healthz))
        .route("/agent/ws", get(ws_handler))
        .route("/admin/onboard", post(admin_onboard))
        .route("/login/{public_token}", get(login_page))
        .route("/service/{public_token}", get(service_page))
        .route("/public/login/{public_token}", get(get_public_login))
        .route(
            "/public/login/{public_token}/events",
            get(login_status_events),
        )
        .route("/public/login/{control_token}/confirm", post(confirm_login))
        .route("/r/{public_token}", get(get_service_status))
        .route(
            "/r/{control_token}/pause",
            post(pause_service).delete(clear_pause),
        )
        .route(
            "/r/{control_token}/quiet-periods",
            axum::routing::put(replace_quiet_periods),
        )
        .layer(Extension(AdminAuthConfig { token: admin_token }))
        .with_state(state)
}

async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

fn spawn_stale_sweeper(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(state.gateway_config.sweep_interval);

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

fn spawn_foster_scheduler(state: AppState) {
    tokio::spawn(async move {
        let interval = std::env::var("FOSTER_SCHEDULER_INTERVAL_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(std::time::Duration::from_secs)
            .unwrap_or_else(|| std::time::Duration::from_secs(15));

        loop {
            tokio::time::sleep(interval).await;

            let now = chrono::Utc::now();
            if let Err(error) = ResourcePoolService::new(state.pool.clone()).reap(now).await {
                tracing::warn!(error = %error, "resource pool reap failed");
            }

            let scheduler = SchedulerService::new(state.pool.clone());
            let report = match scheduler.run_once(now).await {
                Ok(report) => report,
                Err(error) => {
                    tracing::warn!(error = %error, "foster scheduler tick failed");
                    continue;
                }
            };

            let dispatcher = FosterDispatchService::new(state.pool.clone());
            for job_id in report.claimed_job_ids {
                if let Err(error) = dispatcher.dispatch_job(job_id, &state.registry).await {
                    tracing::warn!(
                        job_id,
                        error = %error,
                        "foster job dispatch failed"
                    );
                }
            }
        }
    });
}
