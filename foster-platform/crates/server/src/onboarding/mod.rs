mod admin_page;
mod api;
mod repository;
mod service;

pub use admin_page::admin_page;
pub(crate) use api::authorize_admin;
pub use api::{AdminAuthConfig, admin_onboard};
pub use service::{
    OnboardCustomerRequest, OnboardCustomerResult, OnboardingError, OnboardingService,
};
