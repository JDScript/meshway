//! Meshway HTTP and event contracts.
//!
//! Public envelopes reference the shared model and configuration types. Route
//! declarations live with their actual handlers in the Controller application.

#![forbid(unsafe_code)]

mod auth;
pub mod configuration;
pub mod enrollment;
pub mod events;
pub mod execution;
pub mod history;
pub mod live;
pub mod management;
mod management_rules;
mod networks;
pub mod node;
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
pub use enrollment::{
    ApproveEnrollmentRequest, CreateEnrollmentKeyRequest, CreateEnrollmentRequest,
    EnrollmentDeviceProof, EnrollmentIntent, EnrollmentKeyResponse, EnrollmentKeysResponse,
    EnrollmentPollResponse, EnrollmentRequestView, EnrollmentResponse, KeyEnrollmentRequest,
    RedeemEnrollmentRequest,
};
pub use events::{NodeEvent, StreamEvent, UserEvent};
pub use execution::{
    AcquireInstanceRequest, CancelDemandRequest, DemandRequest, DemandResponse, InstanceLease,
    InstanceRequest, NodeConfigurationRequest, NodeConfigurationResponse, PublicationProgress,
    ReceiptRequest, RoutingSelectionRequest, RoutingSelectionResponse,
};
pub use history::{ActivityListResponse, ActivityResponse, ConfigurationRevisionList};
pub use live::{
    ConfigurationApplyProgress, NodeConfigurationView, NodeEntitiesView, UserConfigurationView,
    UserEntitiesView,
};
pub use management::{
    AccountResponse, AccountSessionsResponse, ChangeEmailRequest, ChangeMemberRoleRequest,
    ChangePasswordRequest, CreateJoinLinkRequest, JoinLinkCreated, JoinLinkCredentialRequest,
    JoinLinkInspection, JoinLinkListResponse, JoinLinkRedeemed, JoinLinkResponse,
    MemberMutationResponse, MemberRemovedResponse, PasswordResetInspectRequest,
    PasswordResetInspection, PasswordResetRedeemRequest, RemovalImpactResponse,
    RemoveMemberRequest,
};
pub use node::{
    DeleteNodeRequest, NodeChallenge, NodeChallengeRequest, NodeInventoryResponse,
    NodeInventoryView, NodeKeyMetadata, NodeRotationRequest, NodeSessionRequest,
    NodeSessionResponse, RevokeNodeKeyRequest,
};
pub use observations::{HistoryAccepted, HistoryPage, HistoryQuery, HistoryReport};
pub use resources::{RuleSetList, RuleSetView};
pub use state::{StateAccepted, StateQuery, StateReport, StateView};

pub use auth::{
    AuthOptionsResponse, AuthSessionResponse, LoginRequest, PasswordLengthUnit, PasswordRules,
    RegisterRequest, SessionIssuedResponse, SessionMetadata, UserView,
};
pub use meshway_model::{
    ActivityId, EnrollmentKeyId, EnrollmentRequestId, JoinLinkId, MembershipId, MembershipRole,
    NetworkId, NodeId, PublicationId, Revision, SessionId, Version,
};
pub use networks::{
    CreateNetworkRequest, MemberActions, MemberSummary, NetworkActions, NetworkListResponse,
    NetworkMembersResponse, NetworkPageQuery, NetworkResponse, NetworkSummary,
    RenameNetworkRequest,
};
pub use problem::{ConfigurationValidationParams, EmptyParams, ErrorResponse, Problem};
pub use resources::{RuleSetList, RuleSetView};
pub use secret::WireSecret;
