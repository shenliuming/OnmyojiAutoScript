use std::time::Duration;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::CONTENT_TYPE},
};
use foster_server::{
    agent_gateway::registry::AgentRegistry,
    app::{AppState, build_app},
    config::AgentGatewayConfig,
    enrollment::EnrollmentService,
};
use serde_json::{Value, json};
use sqlx::MySqlPool;
use tower::ServiceExt;

struct Fixture {
    account_id: i64,
    emulator_id: i64,
    binding_id: i64,
    session_no: String,
    public_token: String,
    control_token: String,
}

async fn seed_fixture(pool: &MySqlPool, suffix: &str) -> anyhow::Result<Fixture> {
    let host = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES (?, ?, 'ONLINE')",
    )
    .bind(format!("host-activation-{suffix}"))
    .bind(format!("host-activation-{suffix}"))
    .execute(pool)
    .await?;
    let host_id = host.last_insert_id() as i64;

    let emulator = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status, lifecycle_status
         )
         VALUES (?, ?, 'FAKE', 5, 'IDLE', 'READY')",
    )
    .bind(host_id)
    .bind(format!("emu-activation-{suffix}"))
    .execute(pool)
    .await?;
    let emulator_id = emulator.last_insert_id() as i64;

    let account = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status
         )
         VALUES (?, 'PENDING', 'PENDING')",
    )
    .bind(9000_i64 + host_id)
    .execute(pool)
    .await?;
    let account_id = account.last_insert_id() as i64;

    let created = EnrollmentService::new(pool.clone())
        .create_login_session(account_id, Duration::from_secs(900))
        .await?;

    Ok(Fixture {
        account_id,
        emulator_id,
        binding_id: created.binding_id,
        session_no: created.session_no,
        public_token: created.public_token,
        control_token: created.control_token,
    })
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

async fn prepare_detected_identity(
    pool: &MySqlPool,
    session_no: &str,
    masked_account: Option<&str>,
    character_name: Option<&str>,
    server_name: Option<&str>,
    game_uid: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE login_session
         SET status = 'VERIFYING_ACCOUNT',
             detected_masked_account = ?,
             detected_character_name = ?,
             detected_server_name = ?,
             detected_game_uid = ?
         WHERE session_no = ?",
    )
    .bind(masked_account)
    .bind(character_name)
    .bind(server_name)
    .bind(game_uid)
    .bind(session_no)
    .execute(pool)
    .await?;

    Ok(())
}

async fn post_confirm(app: axum::Router, token: &str) -> anyhow::Result<axum::response::Response> {
    Ok(app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/public/login/{token}/confirm"))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "confirmed": true }).to_string()))?,
        )
        .await?)
}

async fn json_body(response: axum::response::Response) -> anyhow::Result<Value> {
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[sqlx::test(migrations = "../../migrations")]
async fn first_enrollment_activates_binding_and_account(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "first").await?;
    prepare_detected_identity(
        &pool,
        &fixture.session_no,
        Some("138****5678"),
        Some("角色A"),
        Some("春之樱"),
        Some("10001"),
    )
    .await?;

    let response =
        post_confirm(build_app(test_state(pool.clone())), &fixture.control_token).await?;
    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await?;
    assert_eq!(body["status"], "SUCCESS");
    assert_eq!(body["sessionNo"], fixture.session_no);

    let session_status: String =
        sqlx::query_scalar("SELECT status FROM login_session WHERE session_no = ?")
            .bind(&fixture.session_no)
            .fetch_one(&pool)
            .await?;
    assert_eq!(session_status, "SUCCESS");

    let binding_status: String =
        sqlx::query_scalar("SELECT status FROM emulator_account_binding WHERE id = ?")
            .bind(fixture.binding_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(binding_status, "ACTIVE");

    let account: (Option<i64>, String, String) = sqlx::query_as(
        "SELECT active_emulator_id, login_status, verify_status
         FROM game_account
         WHERE id = ?",
    )
    .bind(fixture.account_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(account.0, Some(fixture.emulator_id));
    assert_eq!(account.1, "LOGGED_IN");
    assert_eq!(account.2, "VERIFIED");

    let identity_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM game_account_identity
         WHERE game_account_id = ?
           AND source = 'LOGIN_ENROLLMENT'
           AND enabled = 1",
    )
    .bind(fixture.account_id)
    .fetch_one(&pool)
    .await?;
    assert!(identity_count >= 3);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn existing_trusted_identity_match_activates(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "existing").await?;

    for (identity_type, value) in [("CHARACTER_NAME", "角色A"), ("SERVER_NAME", "春之樱")] {
        sqlx::query(
            "INSERT INTO game_account_identity(
                game_account_id, identity_type, identity_value,
                normalized_value, source, confidence, enabled
             )
             VALUES (?, ?, ?, ?, 'MANUAL', 100, 1)",
        )
        .bind(fixture.account_id)
        .bind(identity_type)
        .bind(value)
        .bind(value)
        .execute(&pool)
        .await?;
    }

    prepare_detected_identity(
        &pool,
        &fixture.session_no,
        Some("138****5678"),
        Some("角色A"),
        Some("春之樱"),
        None,
    )
    .await?;

    let response =
        post_confirm(build_app(test_state(pool.clone())), &fixture.control_token).await?;
    assert_eq!(response.status(), StatusCode::OK);

    let status: String =
        sqlx::query_scalar("SELECT status FROM login_session WHERE session_no = ?")
            .bind(&fixture.session_no)
            .fetch_one(&pool)
            .await?;
    assert_eq!(status, "SUCCESS");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn uid_conflict_rejects_activation(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "uid-conflict").await?;

    sqlx::query(
        "INSERT INTO game_account_identity(
            game_account_id, identity_type, identity_value,
            normalized_value, source, confidence, enabled
         )
         VALUES (?, 'GAME_UID', '10001', '10001', 'LOGIN_ENROLLMENT', 100, 1)",
    )
    .bind(fixture.account_id)
    .execute(&pool)
    .await?;

    prepare_detected_identity(
        &pool,
        &fixture.session_no,
        Some("138****5678"),
        Some("角色A"),
        Some("春之樱"),
        Some("99999"),
    )
    .await?;

    let response =
        post_confirm(build_app(test_state(pool.clone())), &fixture.control_token).await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let binding_status: String =
        sqlx::query_scalar("SELECT status FROM emulator_account_binding WHERE id = ?")
            .bind(fixture.binding_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(binding_status, "PENDING");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn masked_only_identity_cannot_activate(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "masked-only").await?;
    prepare_detected_identity(
        &pool,
        &fixture.session_no,
        Some("138****5678"),
        None,
        None,
        None,
    )
    .await?;

    let response =
        post_confirm(build_app(test_state(pool.clone())), &fixture.control_token).await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let status: String =
        sqlx::query_scalar("SELECT status FROM login_session WHERE session_no = ?")
            .bind(&fixture.session_no)
            .fetch_one(&pool)
            .await?;
    assert_eq!(status, "VERIFYING_ACCOUNT");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn read_only_public_token_cannot_confirm(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "public-token").await?;
    prepare_detected_identity(
        &pool,
        &fixture.session_no,
        None,
        Some("角色A"),
        Some("春之樱"),
        None,
    )
    .await?;

    let response = post_confirm(build_app(test_state(pool)), &fixture.public_token).await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_confirm_is_idempotent(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "concurrent").await?;
    prepare_detected_identity(
        &pool,
        &fixture.session_no,
        None,
        Some("角色A"),
        Some("春之樱"),
        Some("10001"),
    )
    .await?;

    let app = build_app(test_state(pool.clone()));
    let left = post_confirm(app.clone(), &fixture.control_token);
    let right = post_confirm(app, &fixture.control_token);

    let (left, right) = tokio::join!(left, right);
    let left = left?;
    let right = right?;

    assert_eq!(left.status(), StatusCode::OK);
    assert_eq!(right.status(), StatusCode::OK);

    let active_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM emulator_account_binding
         WHERE game_account_id = ?
           AND status = 'ACTIVE'",
    )
    .bind(fixture.account_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(active_count, 1);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn session_binding_mismatch_rejects_activation(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool, "mismatch").await?;
    let other = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status
         )
         VALUES (99999, 'PENDING', 'PENDING')",
    )
    .execute(&pool)
    .await?;
    let other_account_id = other.last_insert_id() as i64;

    sqlx::query(
        "UPDATE emulator_account_binding
         SET game_account_id = ?
         WHERE id = ?",
    )
    .bind(other_account_id)
    .bind(fixture.binding_id)
    .execute(&pool)
    .await?;

    prepare_detected_identity(
        &pool,
        &fixture.session_no,
        None,
        Some("角色A"),
        Some("春之樱"),
        None,
    )
    .await?;

    let response =
        post_confirm(build_app(test_state(pool.clone())), &fixture.control_token).await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let session_status: String =
        sqlx::query_scalar("SELECT status FROM login_session WHERE session_no = ?")
            .bind(&fixture.session_no)
            .fetch_one(&pool)
            .await?;
    assert_eq!(session_status, "VERIFYING_ACCOUNT");

    Ok(())
}
