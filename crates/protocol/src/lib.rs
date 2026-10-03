//! Meshway HTTP and event contracts.
//!
//! Public envelopes reference the shared model and configuration types. Route
//! declarations live with their actual handlers in the Controller application.

#![forbid(unsafe_code)]

mod auth;
pub mod configuration;
pub mod events;
pub mod history;
pub mod management;
mod management_rules;
mod networks;
mod object;
pub mod observations;
mod problem;
pub mod resources;
mod secret;
pub use management_rules::{NetworkNameRules, NetworkNameTrim};
pub mod state;
pub use events::StreamEvent;

pub use auth::{
    AuthOptionsResponse, AuthSessionResponse, LoginRequest, PasswordLengthUnit, PasswordRules,
    RegisterRequest, SessionIssuedResponse, SessionMetadata, UserView,
};
pub use meshway_model::{MembershipId, MembershipRole, NetworkId, Revision, Version};
pub use networks::{
    CreateNetworkRequest, MemberActions, MemberSummary, NetworkActions, NetworkListResponse,
    NetworkMembersResponse, NetworkPageQuery, NetworkResponse, NetworkSummary,
    RenameNetworkRequest,
};
pub use problem::{ConfigurationValidationParams, EmptyParams, ErrorResponse, Problem};
pub use secret::WireSecret;
