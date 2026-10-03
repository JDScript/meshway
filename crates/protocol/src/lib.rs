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
pub use configuration::{
    CompareConfigurationRequest, ConfigurationAppliedResponse, ConfigurationComparison,
    ConfigurationPreviewResponse, ConfigurationRequest, ConfigurationResponse,
    ConfigurationSchemaResponse, ConfigurationSourceResponse, TransportCatalogResponse,
};
pub use events::StreamEvent;
pub use history::{ActivityListResponse, ActivityResponse, ConfigurationRevisionList};
pub use management::{
    AccountResponse, AccountSessionsResponse, ChangeEmailRequest, ChangeMemberRoleRequest,
    ChangePasswordRequest, CreateJoinLinkRequest, JoinLinkCreated, JoinLinkCredentialRequest,
    JoinLinkInspection, JoinLinkListResponse, JoinLinkRedeemed, JoinLinkResponse,
    MemberMutationResponse, MemberRemovedResponse, PasswordResetInspectRequest,
    PasswordResetInspection, PasswordResetRedeemRequest, RemovalImpactResponse,
    RemoveMemberRequest,
};

pub use auth::{
    AuthOptionsResponse, AuthSessionResponse, LoginRequest, PasswordLengthUnit, PasswordRules,
    RegisterRequest, SessionIssuedResponse, SessionMetadata, UserView,
};
pub use meshway_model::{
    ActivityId, JoinLinkId, MembershipId, MembershipRole, NetworkId, Revision, SessionId, Version,
};
pub use networks::{
    CreateNetworkRequest, MemberActions, MemberSummary, NetworkActions, NetworkListResponse,
    NetworkMembersResponse, NetworkPageQuery, NetworkResponse, NetworkSummary,
    RenameNetworkRequest,
};
pub use problem::{ConfigurationValidationParams, EmptyParams, ErrorResponse, Problem};
pub use secret::WireSecret;
