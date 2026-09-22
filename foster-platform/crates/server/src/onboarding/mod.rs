mod api;
mod h5;
mod repository;
mod service;

pub use api::{AdminAuthConfig, admin_onboard};
pub use h5::{login_page, service_page};
pub use service::{
    OnboardCustomerRequest, OnboardCustomerResult, OnboardingError, OnboardingService,
};
