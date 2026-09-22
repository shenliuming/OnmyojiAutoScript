use std::time::Duration;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::AUTHORIZATION},
};
use chrono::{TimeZone, Utc};
use foster_server::{
    agent_gateway::registry::AgentRegistry,
    app::{AppState, build_app_with_admin_token},
    config::AgentGatewayConfig,
    resource_admin::{
        CreateResourceCycleRequest, ResourceAdminError, ResourceAdminService,
        UpsertProviderRequest,
    },
    resource_pool::{ReserveForJobResult, ResourcePoolService},
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

fn provider_body(code: &str, alias: &str, nickname: &str) -> Value {
    json!({
        "providerCode": code,
        "providerAlias": alias,
        "nickname": nickname,
        "gameUid": "provider-uid",
        "serverName": "春之樱"
    })
}

async fn create_game_account(pool: &MySqlPool, customer_id: i64) -> anyhow::Result<i64> {
    Ok(sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status
         )
         VALUES (?, 'LOGGED_IN', 'VERIFIED')",
    )
    .bind(customer_id)
    .execute(pool)
    .await?
    .last_insert_id() as i64)
}

async fn create_platform_job(
    pool: &MySqlPool,
    game_account_id: i64,
    now: chrono::DateTime<Utc>,
) -> anyhow::Result<i64> {
    let plan_id = sqlx::query(
        "INSERT INTO foster_plan(
            plan_code, plan_name, daily_target_runs,
            interval_minutes, resource_mode, resource_type, status
         )
         VALUES ('PLAN-RESOURCE-ADMIN', 'Resource Admin Plan', 4, 360,
                 'PLATFORM', 'FISH', 'ACTIVE')",
    )
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let subscription_id = sqlx::query(
        "INSERT INTO foster_subscription(
            subscription_no, game_account_id, plan_id,
            resource_mode, resource_type, daily_target_runs,
            interval_minutes, status, start_at, end_at
         )
         VALUES ('SUB-RESOURCE-ADMIN', ?, ?, 'PLATFORM', 'FISH', 4,
                 360, 'ACTIVE', ?, ?)",
    )
    .bind(game_account_id)
    .bind(plan_id)
    .bind((now - chrono::Duration::days(1)).naive_utc())
    .bind((now + chrono::Duration::days(30)).naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    Ok(sqlx::query(
        "INSERT INTO foster_job(
            job_no, subscription_id, game_account_id,
            status, scheduled_at, started_at
         )
         VALUES ('JOB-RESOURCE-ADMIN', ?, ?,
                 'SWITCHING_ACCOUNT', ?, ?)",
    )
    .bind(subscription_id)
    .bind(game_account_id)
    .bind(now.naive_utc())
    .bind(now.naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64)
}

#[sqlx::test(migrations = "../../migrations")]
async fn resource_admin_requires_configured_bearer(pool: MySqlPool) -> anyhow::Result<()> {
    let body = provider_body("P-AUTH", "资源AUTH", "资源号AUTH");

    let app = build_app_with_admin_token(test_state(pool.clone()), Some("admin-secret".into()));

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/providers")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))?,
        )
        .await?;
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

    let wrong = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/providers")
                .header("content-type", "application/json")
                .header(AUTHORIZATION, "Bearer wrong")
                .body(Body::from(body.to_string()))?,
        )
        .await?;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    let no_config = build_app_with_admin_token(test_state(pool), None)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/resource-pool")
                .header(AUTHORIZATION, "Bearer anything")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(no_config.status(), StatusCode::SERVICE_UNAVAILABLE);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn provider_upsert_is_idempotent_by_code(pool: MySqlPool) -> anyhow::Result<()> {
    let app = build_app_with_admin_token(test_state(pool.clone()), Some("admin-secret".into()));

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/providers")
                .header("content-type", "application/json")
                .header(AUTHORIZATION, "Bearer admin-secret")
                .body(Body::from(
                    provider_body("P-001", "资源A01", "旧名称").to_string(),
                ))?,
        )
        .await?;
    assert_eq!(first.status(), StatusCode::OK);
    let first_json = json_body(first).await?;
    let provider_id = first_json["id"].as_i64().unwrap();

    let second = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/providers")
                .header("content-type", "application/json")
                .header(AUTHORIZATION, "Bearer admin-secret")
                .body(Body::from(
                    provider_body("P-001", "资源A01", "新名称").to_string(),
                ))?,
        )
        .await?;
    assert_eq!(second.status(), StatusCode::OK);
    let second_json = json_body(second).await?;

    assert_eq!(second_json["id"].as_i64(), Some(provider_id));
    assert_eq!(second_json["nickname"].as_str(), Some("新名称"));

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_account")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn duplicate_provider_alias_is_conflict(pool: MySqlPool) -> anyhow::Result<()> {
    let service = ResourceAdminService::new(pool);

    service
        .upsert_provider(UpsertProviderRequest {
            provider_code: "P-ALIAS-A".into(),
            game_uid: None,
            nickname: "资源号A".into(),
            provider_alias: "公共别名".into(),
            server_name: None,
            status: None,
        })
        .await?;

    let duplicate = service
        .upsert_provider(UpsertProviderRequest {
            provider_code: "P-ALIAS-B".into(),
            game_uid: None,
            nickname: "资源号B".into(),
            provider_alias: "公共别名".into(),
            server_name: None,
            status: None,
        })
        .await;

    assert!(matches!(
        duplicate,
        Err(ResourceAdminError::ProviderAliasConflict)
    ));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn friend_binding_api_upserts_status(pool: MySqlPool) -> anyhow::Result<()> {
    let account_id = create_game_account(&pool, 92001).await?;
    let provider = ResourceAdminService::new(pool.clone())
        .upsert_provider(UpsertProviderRequest {
            provider_code: "P-BIND".into(),
            game_uid: None,
            nickname: "资源号绑定".into(),
            provider_alias: "资源绑定01".into(),
            server_name: Some("春之樱".into()),
            status: None,
        })
        .await?;

    let app = build_app_with_admin_token(test_state(pool.clone()), Some("admin-secret".into()));
    let uri = format!("/admin/accounts/{account_id}/providers/{}", provider.id);

    for status in ["VERIFIED", "SUSPECT"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(&uri)
                    .header("content-type", "application/json")
                    .header(AUTHORIZATION, "Bearer admin-secret")
                    .body(Body::from(json!({ "status": status }).to_string()))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
    }

    let row: (i64, String) = sqlx::query_as(
        "SELECT COUNT(*), MAX(status)
         FROM foster_friend_binding
         WHERE game_account_id = ?
           AND provider_account_id = ?",
    )
    .bind(account_id)
    .bind(provider.id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.0, 1);
    assert_eq!(row.1, "SUSPECT");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn cycle_api_validates_input_and_lists_pool(pool: MySqlPool) -> anyhow::Result<()> {
    let account_id = create_game_account(&pool, 92002).await?;
    let provider = ResourceAdminService::new(pool.clone())
        .upsert_provider(UpsertProviderRequest {
            provider_code: "P-CYCLE".into(),
            game_uid: None,
            nickname: "资源号周期".into(),
            provider_alias: "资源周期01".into(),
            server_name: Some("春之樱".into()),
            status: None,
        })
        .await?;
    ResourceAdminService::new(pool.clone())
        .set_friend_binding(account_id, provider.id, "VERIFIED")
        .await?;

    let app = build_app_with_admin_token(test_state(pool.clone()), Some("admin-secret".into()));
    let now = Utc.with_ymd_and_hms(2026, 9, 23, 2, 0, 0).unwrap();
    let cycle_uri = format!("/admin/providers/{}/cycles", provider.id);

    for invalid in [
        json!({
            "resourceType": "UNKNOWN",
            "resourceLevel": 6,
            "startAt": now,
            "endAt": now + chrono::Duration::hours(8),
            "slotCapacity": 2
        }),
        json!({
            "resourceType": "FISH",
            "resourceLevel": 6,
            "startAt": now,
            "endAt": now,
            "slotCapacity": 2
        }),
        json!({
            "resourceType": "FISH",
            "resourceLevel": 6,
            "startAt": now,
            "endAt": now + chrono::Duration::hours(8),
            "slotCapacity": 0
        }),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(&cycle_uri)
                    .header("content-type", "application/json")
                    .header(AUTHORIZATION, "Bearer admin-secret")
                    .body(Body::from(invalid.to_string()))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&cycle_uri)
                .header("content-type", "application/json")
                .header(AUTHORIZATION, "Bearer admin-secret")
                .body(Body::from(
                    json!({
                        "resourceType": "DOUYU",
                        "resourceLevel": 6,
                        "startAt": now,
                        "endAt": now + chrono::Duration::hours(8),
                        "slotCapacity": 2
                    })
                    .to_string(),
                ))?,
        )
        .await?;
    assert_eq!(created.status(), StatusCode::OK);
    let created_json = json_body(created).await?;
    assert_eq!(created_json["resourceType"].as_str(), Some("FISH"));
    assert_eq!(created_json["slotCapacity"].as_i64(), Some(2));
    assert_eq!(created_json["occupiedSlots"].as_i64(), Some(0));
    assert_eq!(created_json["verifiedBindingCount"].as_i64(), Some(1));

    let listed = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/resource-pool")
                .header(AUTHORIZATION, "Bearer admin-secret")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(listed.status(), StatusCode::OK);
    let listed_json = json_body(listed).await?;
    assert_eq!(listed_json["providers"].as_array().unwrap().len(), 1);
    assert_eq!(
        listed_json["providers"][0]["providerAlias"].as_str(),
        Some("资源周期01")
    );
    assert_eq!(
        listed_json["providers"][0]["cycles"][0]["verifiedBindingCount"].as_i64(),
        Some(1)
    );

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn disabled_cycle_is_not_allocatable(pool: MySqlPool) -> anyhow::Result<()> {
    let now = Utc.with_ymd_and_hms(2026, 9, 23, 2, 0, 0).unwrap();
    let account_id = create_game_account(&pool, 92003).await?;
    let service = ResourceAdminService::new(pool.clone());

    let provider = service
        .upsert_provider(UpsertProviderRequest {
            provider_code: "P-DISABLED".into(),
            game_uid: None,
            nickname: "资源号禁用".into(),
            provider_alias: "资源禁用01".into(),
            server_name: Some("春之樱".into()),
            status: None,
        })
        .await?;
    service
        .set_friend_binding(account_id, provider.id, "VERIFIED")
        .await?;

    let cycle = service
        .create_cycle(
            provider.id,
            CreateResourceCycleRequest {
                resource_type: "FISH".into(),
                resource_level: 6,
                start_at: now - chrono::Duration::minutes(5),
                end_at: now + chrono::Duration::hours(8),
                slot_capacity: 2,
            },
        )
        .await?;

    service.set_cycle_status(cycle.id, "DISABLED").await?;

    let job_id = create_platform_job(&pool, account_id, now).await?;
    let result = ResourcePoolService::new(pool.clone())
        .with_min_remaining_minutes(0)
        .with_retry_seconds(60)
        .reserve_for_job(job_id, now)
        .await?;

    assert_eq!(result, ReserveForJobResult::WaitingResource);

    let state: (String, Option<chrono::NaiveDateTime>) =
        sqlx::query_as("SELECT status, retry_after FROM foster_job WHERE id = ?")
            .bind(job_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(state.0, "WAITING_RESOURCE");
    assert_eq!(
        state.1,
        Some((now + chrono::Duration::seconds(60)).naive_utc())
    );

    Ok(())
}
