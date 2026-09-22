use chrono::{TimeZone, Utc};
use foster_server::{
    public_portal::{PublicPortalError, PublicPortalService, QuietPeriodInput},
    scheduler::{JobGateResult, SchedulerService},
};
use sqlx::MySqlPool;

struct Fixture {
    account_id: i64,
    subscription_id: i64,
    job_id: i64,
}

async fn seed_fixture(pool: &MySqlPool) -> anyhow::Result<Fixture> {
    let account_id = sqlx::query(
        "INSERT INTO game_account(
            customer_id, character_name, server_name,
            login_status, verify_status
         )
         VALUES (92001, '角色H5', '春之樱', 'LOGGED_IN', 'VERIFIED')",
    )
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let plan_id = sqlx::query(
        "INSERT INTO foster_plan(
            plan_code, plan_name, daily_target_runs,
            interval_minutes, resource_mode, status
         )
         VALUES ('PLAN-H5', 'H5套餐', 4, 360, 'USER_FRIEND', 'ACTIVE')",
    )
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let base = Utc.with_ymd_and_hms(2026, 9, 22, 4, 0, 0).unwrap();

    let subscription_id = sqlx::query(
        "INSERT INTO foster_subscription(
            subscription_no, game_account_id, plan_id,
            resource_mode, daily_target_runs, interval_minutes,
            status, start_at, end_at, next_run_at
         )
         VALUES ('SUB-H5', ?, ?, 'USER_FRIEND', 4, 360,
                 'ACTIVE', ?, ?, ?)",
    )
    .bind(account_id)
    .bind(plan_id)
    .bind((base - chrono::Duration::days(1)).naive_utc())
    .bind((base + chrono::Duration::days(30)).naive_utc())
    .bind((base + chrono::Duration::hours(2)).naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    let job_id = sqlx::query(
        "INSERT INTO foster_job(
            job_no, subscription_id, game_account_id,
            status, scheduled_at
         )
         VALUES ('JOB-H5-PENDING', ?, ?, 'PENDING', ?)",
    )
    .bind(subscription_id)
    .bind(account_id)
    .bind(base.naive_utc())
    .execute(pool)
    .await?
    .last_insert_id() as i64;

    Ok(Fixture {
        account_id,
        subscription_id,
        job_id,
    })
}

#[sqlx::test(migrations = "../../migrations")]
async fn share_tokens_are_hashed_and_public_token_is_read_only(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool).await?;
    let service = PublicPortalService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 4, 0, 0).unwrap();

    let links = service
        .rotate_share_link(fixture.subscription_id, None)
        .await?;

    let raw_public_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM foster_share_link
         WHERE public_token_hash = ? OR control_token_hash = ?",
    )
    .bind(&links.public_token)
    .bind(&links.control_token)
    .fetch_one(&pool)
    .await?;
    assert_eq!(raw_public_count, 0);

    let status = service.load_public_status(&links.public_token, now).await?;
    assert_eq!(status.subscription_no, "SUB-H5");

    let control_with_public = service.pause(&links.public_token, "2H", now).await;
    assert!(matches!(
        control_with_public,
        Err(PublicPortalError::NotFound)
    ));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn rotated_link_revokes_old_link_and_expired_link_is_gone(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool).await?;
    let service = PublicPortalService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 4, 0, 0).unwrap();

    let old = service
        .rotate_share_link(fixture.subscription_id, None)
        .await?;
    let fresh = service
        .rotate_share_link(fixture.subscription_id, None)
        .await?;

    assert!(matches!(
        service.load_public_status(&old.public_token, now).await,
        Err(PublicPortalError::NotFound)
    ));
    assert_eq!(
        service
            .load_public_status(&fresh.public_token, now)
            .await?
            .subscription_no,
        "SUB-H5"
    );

    let expired = service
        .rotate_share_link(
            fixture.subscription_id,
            Some(now - chrono::Duration::seconds(1)),
        )
        .await?;
    assert!(matches!(
        service.load_public_status(&expired.public_token, now).await,
        Err(PublicPortalError::Expired)
    ));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn public_status_counts_only_today_success_and_hides_internal_ids(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool).await?;
    let service = PublicPortalService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 4, 0, 0).unwrap();

    sqlx::query("DELETE FROM foster_job WHERE id = ?")
        .bind(fixture.job_id)
        .execute(&pool)
        .await?;

    for (job_no, status, finished_at) in [
        (
            "JOB-H5-SUCCESS",
            "SUCCESS",
            Some(now - chrono::Duration::minutes(30)),
        ),
        (
            "JOB-H5-FAILED",
            "FAILED",
            Some(now - chrono::Duration::minutes(20)),
        ),
        (
            "JOB-H5-YESTERDAY",
            "SUCCESS",
            Some(now - chrono::Duration::hours(20)),
        ),
    ] {
        sqlx::query(
            "INSERT INTO foster_job(
                job_no, subscription_id, game_account_id,
                status, scheduled_at, finished_at
             )
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(job_no)
        .bind(fixture.subscription_id)
        .bind(fixture.account_id)
        .bind(status)
        .bind((now - chrono::Duration::hours(1)).naive_utc())
        .bind(finished_at.map(|value| value.naive_utc()))
        .execute(&pool)
        .await?;
    }

    let links = service
        .rotate_share_link(fixture.subscription_id, None)
        .await?;
    let status = service.load_public_status(&links.public_token, now).await?;

    assert_eq!(status.today_success_count, 1);
    assert_eq!(status.daily_target_runs, 4);
    assert_eq!(status.recent_jobs.len(), 3);

    let json = serde_json::to_value(&status)?;
    assert!(json.get("subscriptionId").is_none());
    assert!(json.get("gameAccountId").is_none());
    assert!(json.get("hostId").is_none());
    assert!(json.get("tokenHash").is_none());

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn pause_only_extends_until_explicitly_cleared_and_scheduler_defers(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool).await?;
    let portal = PublicPortalService::new(pool.clone());
    let scheduler = SchedulerService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 4, 0, 0).unwrap();

    let links = portal
        .rotate_share_link(fixture.subscription_id, None)
        .await?;

    let first = portal.pause(&links.control_token, "4H", now).await?;
    assert_eq!(first, now + chrono::Duration::hours(4));

    let shorter = portal
        .pause(
            &links.control_token,
            "1H",
            now + chrono::Duration::minutes(10),
        )
        .await?;
    assert_eq!(shorter, first);

    assert_eq!(
        scheduler.gate_pending_job(fixture.job_id, now).await?,
        JobGateResult::DeferredManual(first)
    );

    portal
        .clear_pause(&links.control_token, now + chrono::Duration::minutes(20))
        .await?;

    let stored_pause: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
        "SELECT manual_pause_until
         FROM foster_subscription
         WHERE id = ?",
    )
    .bind(fixture.subscription_id)
    .fetch_one(&pool)
    .await?;
    assert!(stored_pause.is_none());

    let job_status: String = sqlx::query_scalar("SELECT status FROM foster_job WHERE id = ?")
        .bind(fixture.job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(job_status, "PENDING");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn today_pause_ends_at_shanghai_midnight(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool).await?;
    let portal = PublicPortalService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 30, 0).unwrap();
    let links = portal
        .rotate_share_link(fixture.subscription_id, None)
        .await?;

    let until = portal.pause(&links.control_token, "TODAY", now).await?;

    assert_eq!(until, Utc.with_ymd_and_hms(2026, 9, 22, 16, 0, 0).unwrap());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn quiet_periods_replace_atomically_and_feed_scheduler_gate(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool).await?;
    let portal = PublicPortalService::new(pool.clone());
    let scheduler = SchedulerService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 13, 0, 0).unwrap();
    let links = portal
        .rotate_share_link(fixture.subscription_id, None)
        .await?;

    portal
        .replace_quiet_periods(
            &links.control_token,
            vec![QuietPeriodInput {
                weekday_mask: 1,
                start_time: "20:00".into(),
                end_time: "23:00".into(),
                before_buffer_minutes: 5,
                after_buffer_minutes: 5,
            }],
            now,
        )
        .await?;

    assert_eq!(
        scheduler.gate_pending_job(fixture.job_id, now).await?,
        JobGateResult::DeferredQuiet(Utc.with_ymd_and_hms(2026, 9, 21, 15, 5, 0).unwrap())
    );

    portal
        .replace_quiet_periods(
            &links.control_token,
            vec![QuietPeriodInput {
                weekday_mask: 127,
                start_time: "01:00".into(),
                end_time: "02:00".into(),
                before_buffer_minutes: 0,
                after_buffer_minutes: 0,
            }],
            now,
        )
        .await?;

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM foster_quiet_period
         WHERE game_account_id = ?",
    )
    .bind(fixture.account_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(count, 1);

    let job_status: String = sqlx::query_scalar("SELECT status FROM foster_job WHERE id = ?")
        .bind(fixture.job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(job_status, "PENDING");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn invalid_quiet_period_does_not_replace_existing_configuration(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = seed_fixture(&pool).await?;
    let portal = PublicPortalService::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 9, 22, 4, 0, 0).unwrap();
    let links = portal
        .rotate_share_link(fixture.subscription_id, None)
        .await?;

    portal
        .replace_quiet_periods(
            &links.control_token,
            vec![QuietPeriodInput {
                weekday_mask: 127,
                start_time: "20:00".into(),
                end_time: "23:00".into(),
                before_buffer_minutes: 5,
                after_buffer_minutes: 5,
            }],
            now,
        )
        .await?;

    let result = portal
        .replace_quiet_periods(
            &links.control_token,
            vec![QuietPeriodInput {
                weekday_mask: 0,
                start_time: "99:99".into(),
                end_time: "23:00".into(),
                before_buffer_minutes: 999,
                after_buffer_minutes: 0,
            }],
            now,
        )
        .await;
    assert!(matches!(result, Err(PublicPortalError::InvalidQuietPeriod)));

    let row: (u8, chrono::NaiveTime) = sqlx::query_as(
        "SELECT weekday_mask, start_time
         FROM foster_quiet_period
         WHERE game_account_id = ?",
    )
    .bind(fixture.account_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(row.0, 127);
    assert_eq!(row.1, chrono::NaiveTime::from_hms_opt(20, 0, 0).unwrap());

    Ok(())
}
