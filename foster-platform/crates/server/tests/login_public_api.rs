use std::time::Duration;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use foster_server::{
    agent_gateway::registry::AgentRegistry,
    app::{AppState, build_app},
    config::AgentGatewayConfig,
    enrollment::EnrollmentService,
};
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::MySqlPool;
use tower::ServiceExt;

async fn seed_fixture(
    pool: &MySqlPool,
) -> anyhow::Result<(EnrollmentService, String, String, String)> {
    let host = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('host-public', 'host-public', 'ONLINE')",
    )
    .execute(pool)
    .await?;
    let host_id = host.last_insert_id() as i64;

    sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status
         )
         VALUES (?, 'emu-public', 'FAKE', 5, 'IDLE')",
    )
    .bind(host_id)
    .execute(pool)
    .await?;

    let account = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status
         )
         VALUES (8001, 'PENDING', 'PENDING')",
    )
    .execute(pool)
    .await?;
    let account_id = account.last_insert_id() as i64;

    let service = EnrollmentService::new(pool.clone());
    let created = service
        .create_login_session(account_id, Duration::from_secs(900))
        .await?;

    Ok((
        service,
        created.session_no,
        created.public_token,
        created.control_token,
    ))
}

fn test_state(pool: MySqlPool) -> AppState {
    AppState {
        pool,
        registry: AgentRegistry::default(),
        gateway_config: AgentGatewayConfig {
            agent_token: "test-token".into(),
            heartbeat_timeout: Duration::from_secs(45),
            sweep_interval: Duration::from_secs(5),
        },
    }
}

async fn json_body(response: axum::response::Response) -> anyhow::Result<Value> {
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[sqlx::test(migrations = "../../migrations")]
async fn valid_public_token_returns_login_status(pool: MySqlPool) -> anyhow::Result<()> {
    let (_service, session_no, public_token, _control_token) = seed_fixture(&pool).await?;
    let app = build_app(test_state(pool.clone()));

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/public/login/{public_token}"))
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await?;
    assert_eq!(body["sessionNo"], session_no);
    assert_eq!(body["status"], "CREATED");
    assert!(body.get("expiresAt").is_some());

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn invalid_public_token_returns_404(pool: MySqlPool) -> anyhow::Result<()> {
    seed_fixture(&pool).await?;
    let app = build_app(test_state(pool));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/public/login/not-a-valid-token")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn expired_login_session_returns_410(pool: MySqlPool) -> anyhow::Result<()> {
    let (_service, session_no, public_token, _control_token) = seed_fixture(&pool).await?;

    sqlx::query(
        "UPDATE login_session
         SET expires_at = DATE_SUB(NOW(3), INTERVAL 1 SECOND)
         WHERE session_no = ?",
    )
    .bind(session_no)
    .execute(&pool)
    .await?;

    let app = build_app(test_state(pool));
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/public/login/{public_token}"))
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::GONE);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn expired_qr_payload_is_not_exposed(pool: MySqlPool) -> anyhow::Result<()> {
    let (_service, session_no, public_token, _control_token) = seed_fixture(&pool).await?;

    sqlx::query(
        "UPDATE login_session
         SET status = 'QR_READY',
             qr_payload = 'expired-secret-qr',
             qr_expires_at = DATE_SUB(NOW(3), INTERVAL 1 SECOND)
         WHERE session_no = ?",
    )
    .bind(session_no)
    .execute(&pool)
    .await?;

    let app = build_app(test_state(pool));
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/public/login/{public_token}"))
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await?;
    assert!(body.get("qrPayload").is_none() || body["qrPayload"].is_null());

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn public_status_never_exposes_masked_account_or_uid(pool: MySqlPool) -> anyhow::Result<()> {
    let (_service, session_no, public_token, _control_token) = seed_fixture(&pool).await?;

    sqlx::query(
        "UPDATE login_session
         SET status = 'VERIFYING_ACCOUNT',
             detected_masked_account = '138****5678',
             detected_character_name = '角色A',
             detected_server_name = '春之樱',
             detected_game_uid = '10001'
         WHERE session_no = ?",
    )
    .bind(session_no)
    .execute(&pool)
    .await?;

    let app = build_app(test_state(pool));
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/public/login/{public_token}"))
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await?;
    let object = body.as_object().expect("public response must be an object");

    assert!(!object.contains_key("maskedAccount"));
    assert!(!object.contains_key("gameUid"));
    assert_eq!(body["characterName"], "角色A");
    assert_eq!(body["serverName"], "春之樱");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn sse_immediately_emits_current_login_status(pool: MySqlPool) -> anyhow::Result<()> {
    let (_service, _session_no, public_token, _control_token) = seed_fixture(&pool).await?;
    let app = build_app(test_state(pool));

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/public/login/{public_token}/events"))
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );

    let mut body = response.into_body();
    let frame = tokio::time::timeout(Duration::from_millis(500), body.frame())
        .await?
        .ok_or_else(|| anyhow::anyhow!("SSE body ended before initial state"))??;
    let data = frame
        .into_data()
        .map_err(|_| anyhow::anyhow!("expected SSE data frame"))?;
    let text = String::from_utf8(data.to_vec())?;

    assert!(text.contains("event: login_status"));
    assert!(text.contains("\"status\":\"CREATED\""));

    Ok(())
}


#[sqlx::test(migrations = "../../migrations")]
async fn start_login_persists_user_target_before_dispatch(pool: MySqlPool) -> anyhow::Result<()> {
    let (_service, session_no, _public_token, control_token) = seed_fixture(&pool).await?;
    let app = build_app(test_state(pool.clone()));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/public/login/{control_token}/start"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "platform": "ANDROID",
                        "characterName": "角色A",
                        "gameUid": "10001"
                    })
                    .to_string(),
                ))?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await?;
    assert_eq!(body["status"], "WAITING_EMULATOR");

    let target: (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT a.platform, a.character_name, a.game_uid
         FROM game_account a
         JOIN login_session ls ON ls.game_account_id = a.id
         WHERE ls.session_no = ?",
    )
    .bind(&session_no)
    .fetch_one(&pool)
    .await?;

    assert_eq!(target.0.as_deref(), Some("ANDROID"));
    assert_eq!(target.1.as_deref(), Some("角色A"));
    assert_eq!(target.2.as_deref(), Some("10001"));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn start_login_rejects_unknown_platform(pool: MySqlPool) -> anyhow::Result<()> {
    let (_service, _session_no, _public_token, control_token) = seed_fixture(&pool).await?;
    let app = build_app(test_state(pool));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/public/login/{control_token}/start"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "platform": "WINDOWS",
                        "characterName": "角色A",
                        "gameUid": "10001"
                    })
                    .to_string(),
                ))?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    Ok(())
}
