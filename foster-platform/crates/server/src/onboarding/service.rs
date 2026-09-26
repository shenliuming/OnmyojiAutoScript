use std::time::Duration;

use chrono::Utc;
use serde::Serialize;
use sqlx::MySqlPool;
use uuid::Uuid;

use crate::{
    agent_gateway::registry::AgentRegistry,
    control_plane::AllocationError,
    enrollment::{DispatchLoginResult, EnrollmentError, EnrollmentService},
    public_portal::{PublicPortalError, PublicPortalService},
};

use super::repository::{
    NewPendingSubscription, cleanup_failed_onboarding, insert_pending_account,
    insert_pending_subscription, load_active_plan,
};

#[derive(Debug, thiserror::Error)]
pub enum OnboardingError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Enrollment(#[from] EnrollmentError),
    #[error(transparent)]
    Portal(#[from] PublicPortalError),
    #[error("unknown or inactive foster plan")]
    UnknownPlan,
    #[error("service days must be positive")]
    InvalidServiceDays,
    #[error("login ttl minutes must be between 1 and 1440")]
    InvalidLoginTtl,
    #[error("no emulator capacity is available")]
    NoCapacity,
}

#[derive(Debug, Clone)]
pub struct OnboardCustomerRequest {
    pub customer_id: i64,
    pub plan_code: String,
    pub service_days: i32,
    pub login_ttl_minutes: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardCustomerResult {
    pub subscription_no: String,
    pub login_url: String,
    pub service_url: String,
    pub login_dispatch_status: String,
}

#[derive(Clone)]
pub struct OnboardingService {
    pool: MySqlPool,
}

impl OnboardingService {
    pub fn new(pool: MySqlPool) -> Self {
        Self { pool }
    }

    pub async fn onboard(
        &self,
        request: OnboardCustomerRequest,
        registry: &AgentRegistry,
    ) -> Result<OnboardCustomerResult, OnboardingError> {
        if request.service_days <= 0 {
            return Err(OnboardingError::InvalidServiceDays);
        }
        if !(1..=1440).contains(&request.login_ttl_minutes) {
            return Err(OnboardingError::InvalidLoginTtl);
        }

        let plan = load_active_plan(&self.pool, &request.plan_code)
            .await?
            .ok_or(OnboardingError::UnknownPlan)?;

        let now = Utc::now();
        let end_at = now + chrono::Duration::days(i64::from(request.service_days));
        let subscription_no = format!("SUB-{}", Uuid::new_v4());

        let mut tx = self.pool.begin().await?;
        let game_account_id = insert_pending_account(&mut tx, request.customer_id).await?;
        let subscription_id = insert_pending_subscription(
            &mut tx,
            NewPendingSubscription {
                subscription_no: &subscription_no,
                game_account_id,
                plan: &plan,
                start_at: now,
                end_at,
            },
        )
        .await?;
        tx.commit().await?;

        let result = self
            .finish_onboarding(
                game_account_id,
                subscription_id,
                &subscription_no,
                end_at,
                request.login_ttl_minutes,
                registry,
            )
            .await;

        if result.is_err() {
            let _ = cleanup_failed_onboarding(&self.pool, game_account_id, subscription_id).await;
        }

        result
    }

    async fn finish_onboarding(
        &self,
        game_account_id: i64,
        subscription_id: i64,
        subscription_no: &str,
        service_end_at: chrono::DateTime<Utc>,
        login_ttl_minutes: i32,
        registry: &AgentRegistry,
    ) -> Result<OnboardCustomerResult, OnboardingError> {
        let login = EnrollmentService::new(self.pool.clone())
            .create_login_session(
                game_account_id,
                Duration::from_secs((login_ttl_minutes as u64) * 60),
            )
            .await
            .map_err(map_enrollment_error)?;

        let share = PublicPortalService::new(self.pool.clone())
            .rotate_share_link(subscription_id, Some(service_end_at))
            .await?;

        let _ = registry;

        Ok(OnboardCustomerResult {
            subscription_no: subscription_no.to_string(),
            login_url: format!(
                "/login/{}#control={}",
                login.public_token, login.control_token
            ),
            service_url: format!(
                "/service/{}#control={}",
                share.public_token, share.control_token
            ),
            login_dispatch_status: "AWAITING_USER_INPUT".to_string(),
        })
    }
}

fn map_enrollment_error(error: EnrollmentError) -> OnboardingError {
    match error {
        EnrollmentError::Allocation(AllocationError::NoCapacity) => OnboardingError::NoCapacity,
        other => OnboardingError::Enrollment(other),
    }
}

fn dispatch_status_name(status: DispatchLoginResult) -> &'static str {
    match status {
        DispatchLoginResult::Dispatched => "DISPATCHED",
        DispatchLoginResult::WaitingEmulator => "WAITING_EMULATOR",
        DispatchLoginResult::AlreadyDispatched => "ALREADY_DISPATCHED",
    }
}
