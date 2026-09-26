use std::time::Duration;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::AUTHORIZATION},
};
use foster_server::{
    agent_gateway::registry::AgentRegistry,
    app::{AppState, build_app_with_admin_token},
    config::AgentGatewayConfig,
};
use serde_json::{Value, json};
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

async fn json_body(response: axum::response::Response) -> anyhow::Result<Value> {
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn host_body(code: &str, hostname: &str) -> Value {
    json!({
        "hostCode": code,
        "hostname": hostname
    })
}

#[sqlx::test(migrations = "../../migrations")]
async fn readyz_returns_ready_when_database_is_available(pool: MySqlPool) -> anyhow::Result<()> {
    let app = build_app_with_admin_token(test_state(pool), Some("admin-secret".into()));

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/readyz")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await?;
    assert_eq!(body["status"].as_str(), Some("ready"));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn host_admin_requires_bearer_token(pool: MySqlPool) -> anyhow::Result<()> {
    let app = build_app_with_admin_token(test_state(pool), Some("admin-secret".into()));

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/hosts")
                .header("content-type", "application/json")
                .body(Body::from(host_body("HOST-01", "win-01").to_string()))?,
        )
        .await?;
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

    let wrong = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/hosts")
                .header(AUTHORIZATION, "Bearer wrong")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn host_bootstrap_is_idempotent_by_host_code(pool: MySqlPool) -> anyhow::Result<()> {
    let app = build_app_with_admin_token(test_state(pool.clone()), Some("admin-secret".into()));

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/hosts")
                .header("content-type", "application/json")
                .header(AUTHORIZATION, "Bearer admin-secret")
                .body(Body::from(host_body("HOST-01", "win-old").to_string()))?,
        )
        .await?;
    assert_eq!(first.status(), StatusCode::OK);
    let first_body = json_body(first).await?;
    let host_id = first_body["id"].as_i64().unwrap();

    let second = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/hosts")
                .header("content-type", "application/json")
                .header(AUTHORIZATION, "Bearer admin-secret")
                .body(Body::from(host_body("HOST-01", "win-new").to_string()))?,
        )
        .await?;
    assert_eq!(second.status(), StatusCode::OK);
    let second_body = json_body(second).await?;

    assert_eq!(second_body["id"].as_i64(), Some(host_id));
    assert_eq!(second_body["hostname"].as_str(), Some("win-new"));
    assert_eq!(second_body["status"].as_str(), Some("OFFLINE"));

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM host WHERE host_code = 'HOST-01'")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn host_bootstrap_cannot_force_online(pool: MySqlPool) -> anyhow::Result<()> {
    let app = build_app_with_admin_token(test_state(pool), Some("admin-secret".into()));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/hosts")
                .header("content-type", "application/json")
                .header(AUTHORIZATION, "Bearer admin-secret")
                .body(Body::from(
                    json!({
                        "hostCode": "HOST-ONLINE",
                        "hostname": "win-online",
                        "status": "ONLINE"
                    })
                    .to_string(),
                ))?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn host_bootstrap_preserves_runtime_presence_fields(pool: MySqlPool) -> anyhow::Result<()> {
    let result = sqlx::query(
        "INSERT INTO host(
            host_code, hostname, status, agent_version, last_heartbeat_at
         )
         VALUES ('HOST-RUNTIME', 'old-name', 'ONLINE', '1.2.3', NOW(3))",
    )
    .execute(&pool)
    .await?;
    let host_id = result.last_insert_id() as i64;

    let before: chrono::NaiveDateTime =
        sqlx::query_scalar("SELECT last_heartbeat_at FROM host WHERE id = ?")
            .bind(host_id)
            .fetch_one(&pool)
            .await?;

    let app = build_app_with_admin_token(test_state(pool.clone()), Some("admin-secret".into()));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/hosts")
                .header("content-type", "application/json")
                .header(AUTHORIZATION, "Bearer admin-secret")
                .body(Body::from(
                    json!({
                        "hostCode": "HOST-RUNTIME",
                        "hostname": "new-name",
                        "status": "MAINTENANCE"
                    })
                    .to_string(),
                ))?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await?;
    assert_eq!(body["hostname"].as_str(), Some("new-name"));
    assert_eq!(body["status"].as_str(), Some("ONLINE"));
    assert_eq!(body["agentVersion"].as_str(), Some("1.2.3"));

    let after: chrono::NaiveDateTime =
        sqlx::query_scalar("SELECT last_heartbeat_at FROM host WHERE id = ?")
            .bind(host_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(after, before);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn host_list_reports_emulator_capacity_and_bindings(pool: MySqlPool) -> anyhow::Result<()> {
    let host_id = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('HOST-CAP', 'win-cap', 'ONLINE')",
    )
    .execute(&pool)
    .await?
    .last_insert_id() as i64;

    let emu_a = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type, max_account_count, status
         )
         VALUES (?, 'emu-cap-a', 'ADB', 3, 'IDLE')",
    )
    .bind(host_id)
    .execute(&pool)
    .await?
    .last_insert_id() as i64;

    let emu_b = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type, max_account_count, status
         )
         VALUES (?, 'emu-cap-b', 'ADB', 5, 'OFFLINE')",
    )
    .bind(host_id)
    .execute(&pool)
    .await?
    .last_insert_id() as i64;

    let account_a = sqlx::query(
        "INSERT INTO game_account(customer_id, login_status, verify_status)
         VALUES (10001, 'LOGGED_IN', 'VERIFIED')",
    )
    .execute(&pool)
    .await?
    .last_insert_id() as i64;
    let account_b = sqlx::query(
        "INSERT INTO game_account(customer_id, login_status, verify_status)
         VALUES (10002, 'PENDING', 'PENDING')",
    )
    .execute(&pool)
    .await?
    .last_insert_id() as i64;

    sqlx::query(
        "INSERT INTO emulator_account_binding(
            emulator_id, game_account_id, slot_no, status
         )
         VALUES (?, ?, 1, 'ACTIVE'), (?, ?, 1, 'PENDING')",
    )
    .bind(emu_a)
    .bind(account_a)
    .bind(emu_b)
    .bind(account_b)
    .execute(&pool)
    .await?;

    let app = build_app_with_admin_token(test_state(pool), Some("admin-secret".into()));
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/hosts")
                .header(AUTHORIZATION, "Bearer admin-secret")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await?;
    let host = &body.as_array().unwrap()[0];

    assert_eq!(host["totalEmulators"].as_i64(), Some(2));
    assert_eq!(host["onlineEmulators"].as_i64(), Some(1));
    assert_eq!(host["configuredCapacity"].as_i64(), Some(8));
    assert_eq!(host["boundAccounts"].as_i64(), Some(2));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn emulator_capacity_can_be_configured_without_sql(pool: MySqlPool) -> anyhow::Result<()> {
    let host_id = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('HOST-EMU-CAP', 'win-emu-cap', 'ONLINE')",
    )
    .execute(&pool)
    .await?
    .last_insert_id() as i64;

    let emulator_id = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type, max_account_count,
            status, lifecycle_status, occupancy_status, activity_type
         )
         VALUES (
            ?, 'emu-admin-cap', 'ADB', 5,
            'IDLE', 'READY', 'IDLE', 'NONE'
         )",
    )
    .bind(host_id)
    .execute(&pool)
    .await?
    .last_insert_id() as i64;

    let app = build_app_with_admin_token(test_state(pool.clone()), Some("admin-secret".into()));

    let updated = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/admin/emulators/{emulator_id}/capacity"))
                .header("content-type", "application/json")
                .header(AUTHORIZATION, "Bearer admin-secret")
                .body(Body::from(json!({ "maxAccountCount": 3 }).to_string()))?,
        )
        .await?;

    assert_eq!(updated.status(), StatusCode::OK);
    let updated_body = json_body(updated).await?;
    assert_eq!(updated_body["maxAccountCount"].as_i64(), Some(3));

    let stored: i32 =
        sqlx::query_scalar("SELECT max_account_count FROM emulator_instance WHERE id = ?")
            .bind(emulator_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(stored, 3);

    let listed = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/admin/hosts/{host_id}/emulators"))
                .header(AUTHORIZATION, "Bearer admin-secret")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(listed.status(), StatusCode::OK);
    let listed_body = json_body(listed).await?;
    let emulator = &listed_body.as_array().unwrap()[0];
    assert_eq!(emulator["id"].as_i64(), Some(emulator_id));
    assert_eq!(emulator["emulatorCode"].as_str(), Some("emu-admin-cap"));
    assert_eq!(emulator["maxAccountCount"].as_i64(), Some(3));
    assert_eq!(emulator["lifecycleStatus"].as_str(), Some("READY"));
    assert_eq!(emulator["occupancyStatus"].as_str(), Some("IDLE"));
    assert_eq!(emulator["activityType"].as_str(), Some("NONE"));
    assert_eq!(emulator["available"].as_bool(), Some(true));
    assert_eq!(emulator["boundAccounts"].as_i64(), Some(0));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn emulator_capacity_rejects_invalid_values(pool: MySqlPool) -> anyhow::Result<()> {
    let host_id = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('HOST-EMU-INVALID', 'win-emu-invalid', 'OFFLINE')",
    )
    .execute(&pool)
    .await?
    .last_insert_id() as i64;

    let emulator_id = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type, max_account_count, status
         )
         VALUES (?, 'emu-admin-invalid', 'ADB', 5, 'OFFLINE')",
    )
    .bind(host_id)
    .execute(&pool)
    .await?
    .last_insert_id() as i64;

    let app = build_app_with_admin_token(test_state(pool), Some("admin-secret".into()));

    for invalid in [0, 101] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/admin/emulators/{emulator_id}/capacity"))
                    .header("content-type", "application/json")
                    .header(AUTHORIZATION, "Bearer admin-secret")
                    .body(Body::from(
                        json!({ "maxAccountCount": invalid }).to_string(),
                    ))?,
            )
            .await?;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    Ok(())
}
