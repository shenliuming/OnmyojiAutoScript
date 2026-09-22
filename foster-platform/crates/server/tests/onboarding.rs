use std::time::Duration;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::AUTHORIZATION},
};
use foster_server::{
    agent_gateway::registry::AgentRegistry,
    app::{AppState, build_app_with_admin_token},
    config::AgentGatewayConfig,
    onboarding::{OnboardCustomerRequest, OnboardingError, OnboardingService},
};
use serde_json::Value;
use sqlx::MySqlPool;
use tower::ServiceExt;

async fn seed_emulator_capacity(pool: &MySqlPool) -> anyhow::Result<()> {
    let host_id = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('host-onboard', 'host-onboard', 'ONLINE')",
    )
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status
         )
         VALUES (?, 'emu-onboard', 'FAKE', 5, 'IDLE')",
    )
    .bind(host_id)
    .execute(pool)
    .await?;

    Ok(())
}

fn request(plan_code: &str) -> OnboardCustomerRequest {
    OnboardCustomerRequest {
        customer_id: 99001,
        plan_code: plan_code.to_string(),
        service_days: 30,
        login_ttl_minutes: 15,
    }
}

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

#[sqlx::test(migrations = "../../migrations")]
async fn default_mvp_plans_exist(pool: MySqlPool) -> anyhow::Result<()> {
    let plans: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT plan_code, resource_mode, resource_type
         FROM foster_plan
         WHERE plan_code IN (
            'BASIC_AUTO_FOSTER',
            'PLATFORM_FISH',
            'PLATFORM_TAIKO_JADE'
         )
         ORDER BY plan_code",
    )
    .fetch_all(&pool)
    .await?;

    assert_eq!(plans.len(), 3);
    assert!(
        plans.iter().any(|row| {
            row.0 == "BASIC_AUTO_FOSTER" && row.1 == "USER_FRIEND" && row.2.is_none()
        })
    );
    assert!(plans.iter().any(|row| {
        row.0 == "PLATFORM_FISH" && row.1 == "PLATFORM" && row.2.as_deref() == Some("FISH")
    }));
    assert!(plans.iter().any(|row| {
        row.0 == "PLATFORM_TAIKO_JADE"
            && row.1 == "PLATFORM"
            && row.2.as_deref() == Some("TAIKO_JADE")
    }));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn onboarding_creates_pending_subscription_login_and_share_link(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    seed_emulator_capacity(&pool).await?;

    let result = OnboardingService::new(pool.clone())
        .onboard(request("BASIC_AUTO_FOSTER"), &AgentRegistry::default())
        .await?;

    assert!(result.login_url.starts_with("/login/"));
    assert!(result.login_url.contains("#control="));
    assert!(result.service_url.starts_with("/service/"));
    assert!(result.service_url.contains("#control="));
    assert_eq!(result.login_dispatch_status, "WAITING_EMULATOR");

    let subscription: (String, String, String, i32, i32) = sqlx::query_as(
        "SELECT
            subscription_no,
            status,
            resource_mode,
            daily_target_runs,
            interval_minutes
         FROM foster_subscription
         WHERE subscription_no = ?",
    )
    .bind(&result.subscription_no)
    .fetch_one(&pool)
    .await?;

    assert_eq!(subscription.0, result.subscription_no);
    assert_eq!(subscription.1, "PENDING_LOGIN");
    assert_eq!(subscription.2, "USER_FRIEND");
    assert_eq!(subscription.3, 4);
    assert_eq!(subscription.4, 360);

    let login_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM login_session")
        .fetch_one(&pool)
        .await?;
    let share_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM foster_share_link")
        .fetch_one(&pool)
        .await?;
    assert_eq!(login_count, 1);
    assert_eq!(share_count, 1);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn unknown_plan_is_rejected_without_orphan_account(pool: MySqlPool) -> anyhow::Result<()> {
    seed_emulator_capacity(&pool).await?;

    let result = OnboardingService::new(pool.clone())
        .onboard(request("DOES_NOT_EXIST"), &AgentRegistry::default())
        .await;

    assert!(matches!(result, Err(OnboardingError::UnknownPlan)));

    let account_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM game_account")
        .fetch_one(&pool)
        .await?;
    let subscription_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM foster_subscription")
        .fetch_one(&pool)
        .await?;
    assert_eq!(account_count, 0);
    assert_eq!(subscription_count, 0);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn no_emulator_capacity_is_rejected_without_orphans(pool: MySqlPool) -> anyhow::Result<()> {
    let result = OnboardingService::new(pool.clone())
        .onboard(request("BASIC_AUTO_FOSTER"), &AgentRegistry::default())
        .await;

    assert!(matches!(result, Err(OnboardingError::NoCapacity)));

    for table in [
        "login_session",
        "emulator_account_binding",
        "foster_share_link",
        "foster_subscription",
        "game_account",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await?;
        assert_eq!(count, 0, "orphan rows remain in {table}");
    }

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn admin_onboard_requires_configured_bearer_token(pool: MySqlPool) -> anyhow::Result<()> {
    seed_emulator_capacity(&pool).await?;

    let body = serde_json::json!({
        "customerId": 99002,
        "planCode": "BASIC_AUTO_FOSTER",
        "serviceDays": 30,
        "loginTtlMinutes": 15
    });

    let app = build_app_with_admin_token(test_state(pool.clone()), Some("admin-secret".into()));

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/onboard")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))?,
        )
        .await?;
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

    let wrong = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/onboard")
                .header("content-type", "application/json")
                .header(AUTHORIZATION, "Bearer wrong")
                .body(Body::from(body.to_string()))?,
        )
        .await?;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    let ok = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/onboard")
                .header("content-type", "application/json")
                .header(AUTHORIZATION, "Bearer admin-secret")
                .body(Body::from(body.to_string()))?,
        )
        .await?;
    assert_eq!(ok.status(), StatusCode::OK);
    let json = json_body(ok).await?;
    assert!(json["subscriptionNo"].as_str().is_some());
    assert!(json["loginUrl"].as_str().unwrap().starts_with("/login/"));
    assert!(
        json["serviceUrl"]
            .as_str()
            .unwrap()
            .starts_with("/service/")
    );

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn missing_server_admin_token_fails_closed(pool: MySqlPool) -> anyhow::Result<()> {
    seed_emulator_capacity(&pool).await?;
    let app = build_app_with_admin_token(test_state(pool), None);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/onboard")
                .header("content-type", "application/json")
                .header(AUTHORIZATION, "Bearer anything")
                .body(Body::from(
                    serde_json::json!({
                        "customerId": 99003,
                        "planCode": "BASIC_AUTO_FOSTER",
                        "serviceDays": 30,
                        "loginTtlMinutes": 15
                    })
                    .to_string(),
                ))?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn confirmed_onboarding_login_activates_subscription(pool: MySqlPool) -> anyhow::Result<()> {
    seed_emulator_capacity(&pool).await?;

    let result = OnboardingService::new(pool.clone())
        .onboard(request("BASIC_AUTO_FOSTER"), &AgentRegistry::default())
        .await?;

    let control_token = result
        .login_url
        .split("#control=")
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("missing login control token"))?
        .to_string();

    sqlx::query(
        "UPDATE login_session
         SET status = 'VERIFYING_ACCOUNT',
             detected_character_name = '角色首单',
             detected_server_name = '春之樱'
         WHERE game_account_id = (
             SELECT game_account_id
             FROM foster_subscription
             WHERE subscription_no = ?
         )",
    )
    .bind(&result.subscription_no)
    .execute(&pool)
    .await?;

    foster_server::enrollment::EnrollmentService::new(pool.clone())
        .confirm_login_session(&control_token)
        .await?;

    let row: (String, Option<chrono::NaiveDateTime>) = sqlx::query_as(
        "SELECT status, next_run_at
         FROM foster_subscription
         WHERE subscription_no = ?",
    )
    .bind(&result.subscription_no)
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.0, "ACTIVE");
    assert!(row.1.is_some());

    Ok(())
}
