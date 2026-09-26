use std::time::Duration;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use foster_server::{
    agent_gateway::registry::AgentRegistry,
    app::{AppState, build_app_with_admin_token},
    config::AgentGatewayConfig,
};
use sqlx::MySqlPool;
use tower::ServiceExt;

fn test_state(pool: MySqlPool) -> AppState {
    AppState {
        pool,
        registry: AgentRegistry::default(),
        gateway_config: AgentGatewayConfig {
            agent_token: "agent-test".into(),
            heartbeat_timeout: Duration::from_secs(45),
            sweep_interval: Duration::from_secs(5),
        },
    }
}

async fn text_body(response: axum::response::Response) -> anyhow::Result<String> {
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(String::from_utf8(bytes.to_vec())?)
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_h5_is_static_and_reads_control_token_from_fragment(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let app = build_app_with_admin_token(test_state(pool), Some("admin".into()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/login/public-login-token")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let html = text_body(response).await?;
    assert!(html.contains("location.hash"));
    assert!(html.contains("/public/login/"));
    assert!(html.contains("Android"));
    assert!(html.contains("iOS"));
    assert!(html.contains("characterName"));
    assert!(html.contains("gameUid"));
    assert!(html.contains("/start"));
    assert!(!html.contains("public-login-token"));
    assert!(!html.contains("admin"));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn service_h5_is_read_only_without_fragment_and_exposes_no_server_token(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let app = build_app_with_admin_token(test_state(pool), Some("server-admin-secret".into()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/service/public-service-token")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let html = text_body(response).await?;

    assert!(html.contains("location.hash"));
    assert!(html.contains("只读链接"));
    assert!(html.contains("quietPeriods"));
    assert!(!html.contains("public-service-token"));
    assert!(!html.contains("server-admin-secret"));

    Ok(())
}
