//! Meshway HTTP and event contracts, and the local Node control protocol
//! ([`ipc`]) between `meshway` and `meshwayd`.
//!
//! Public envelopes reference the shared model and configuration types. Route
//! declarations live with their actual handlers in the Controller application.

#![forbid(unsafe_code)]

mod auth;
mod challenge_request;
pub mod configuration;
pub mod connectivity;
pub mod enrollment;
pub mod events;
pub mod execution;
pub mod history;
pub mod ipc;
pub mod live;
pub mod management;
mod management_rules;
pub mod netmap;
mod networks;
pub mod node;
pub mod node_observations;
mod object;
pub mod observations;
mod problem;
pub mod resources;
mod secret;
pub use management_rules::{NetworkNameRules, NetworkNameTrim};
pub mod state;
pub use challenge_request::{
    NodeChallengeIntent, SignedNodeChallengeRequest, challenge_request_transcript,
};
pub use configuration::{
    CompareConfigurationRequest, ConfigurationAppliedResponse, ConfigurationComparison,
    ConfigurationPreviewResponse, ConfigurationRequest, ConfigurationResponse,
    ConfigurationSchemaResponse, ConfigurationSourceResponse, EvaluateConfigurationRequest,
    EvaluationConfiguration, EvaluationResponse, EvaluationTarget, TransportCatalogResponse,
};
pub use connectivity::{
    AcceptUdpTraversalResponse, CancelUdpTraversalResponse, CandidateReference,
    ConnectivityLimitExceeded, PrepareUdpTraversalRequest, PrepareUdpTraversalResponse,
    QueryUdpTraversalRequest, QueryUdpTraversalResponse, SocketReference, StunServer, StunServers,
    UdpTraversalAttempt, UdpTraversalAttemptRequest, UdpTraversalAttempts, UdpTraversalOffer,
    UdpTraversalParticipant, UdpTraversalPhase, UdpTraversalRequest, UdpTraversalRequestIds,
    UdpTraversalRequests, UdpTraversalUnavailableReason, candidate_observation_digest,
};
pub use enrollment::{
    ApproveEnrollmentRequest, CreateEnrollmentKeyRequest, CreateEnrollmentRequest,
    DeviceAuthorizationView, EnrollmentDeviceProof, EnrollmentIntent, EnrollmentKeyResponse,
    EnrollmentKeysResponse, EnrollmentPollResponse, EnrollmentRequestView, EnrollmentResponse,
    KeyEnrollmentRequest, RedeemEnrollmentRequest,
};
pub use events::{StreamEvent, UserEvent};
pub use execution::{
    AcquireInstanceRequest, InstanceLease, InstanceRequest, ReceiptRequest,
    RoutingSelectionRequest, RoutingSelectionResponse,
};
pub use history::{ActivityListResponse, ActivityResponse, ConfigurationRevisionList};
pub use live::{ConfigurationApplyProgress, UserConfigurationView, UserEntitiesView};
pub use management::{
    AccountResponse, AccountSessionsResponse, ChangeEmailRequest, ChangeMemberRoleRequest,
    ChangePasswordRequest, CreateJoinLinkRequest, JoinLinkCreated, JoinLinkCredentialRequest,
    JoinLinkInspection, JoinLinkListResponse, JoinLinkRedeemed, JoinLinkResponse,
    MemberMutationResponse, MemberRemovedResponse, PasswordResetInspectRequest,
    PasswordResetInspection, PasswordResetRedeemRequest, RemovalImpactResponse,
    RemoveMemberRequest, RoleChangeImpactResponse,
};
pub use netmap::{
    DnsAssociationAccepted, DnsAssociationReport, DnsAssociationsUpdate, NetmapKeyRotation,
    NetmapSignature, NetmapSigner, PeerExitFacts, PeerFacts, PeerFactsDelta, PeerListenerFacts,
    ServerSigningKey, StateCursor, SyncNetmap, SyncRequest, SyncResponse,
    dns_associations_transcript, fact_visible_peers, netmap_key_rotation_transcript,
    netmap_lease_transcript,
};
pub use node::{
    DeleteNodeRequest, NodeChallenge, NodeChallengeRequest, NodeInventoryResponse,
    NodeInventoryView, NodeKeyMetadata, NodeLeaveRequest, NodeLeaveResponse, NodeRotationRequest,
    NodeSessionRequest, NodeSessionResponse, NodeSettingsResponse, PinnedServerKeyStatus,
    PinnedServerKeyView, RevokeNodeKeyRequest,
};
pub use node_observations::{
    CreateNodeObservation, NodeObservationCancelled, NodeObservationQuery, NodeObservationResponse,
    NodeObservationTarget, NodeObservationTargetView, RenewNodeObservation,
};
pub use observations::{
    CreateObservation, HistoryAccepted, HistoryPage, HistoryQuery, HistoryReport, ObservationList,
    ObservationResult, ObservationTask,
};
pub use state::{
    STATE_FACT_MAX_SECONDS, STATE_REPORT_TTL_SECONDS, StateAccepted, StateQuery, StateReport,
    StateView,
};

pub use auth::{
    AuthOptionsResponse, AuthSessionResponse, CodeChallengeMethod, CreateDelegationRequest,
    DelegatedSessionResponse, DelegationCreated, ExchangeDelegationRequest, LoginRequest,
    PasswordLengthUnit, PasswordRules, RegisterRequest, SessionIssuedResponse, SessionMetadata,
    UserView,
};
pub use meshway_model::{
    ActivityId, EnrollmentKeyId, EnrollmentRequestId, InstanceId, JoinLinkId, MembershipId,
    MembershipRole, NetworkId, NodeId, ObservationId, ObservationSubscriptionId, Revision,
    SessionId, UdpTraversalId, Version,
};
pub use networks::{
    CreateNetworkRequest, MemberActions, MemberSummary, NetworkActions, NetworkListResponse,
    NetworkMembersResponse, NetworkPageQuery, NetworkResponse, NetworkSummary,
    RenameNetworkRequest,
};
pub use problem::{ConfigurationValidationParams, EmptyParams, ErrorResponse, Problem};
pub use resources::{RefreshRuleSetRequest, RuleSetList, RuleSetReport, RuleSetView};
pub use secret::WireSecret;
