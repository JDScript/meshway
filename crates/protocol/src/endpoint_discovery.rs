//! Leased STUN self-observations of live UDP carrier sockets. Never authorization.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const ENDPOINT_DISCOVERY_TTL_SECONDS: u32 = 30;
pub const MAX_DISCOVERY_SOCKETS: usize = 128;
pub const MAX_DISCOVERY_OBSERVERS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UdpDiscoveryResourceKind {
    Listener,
    OutgoingCandidate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EndpointDiscoverySource {
    ManagedUdpSocketStun,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EndpointDiscoveryState {
    AwaitingReport,
    Disabled,
    Probing,
    Observed,
    Withdrawn,
    Expired,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StunDiscoveryOutcome {
    Mapped,
    Timeout,
    InvalidResponse,
    IoError,
    StunError,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StunEndpointObservation {
    /// Explicit numeric STUN observer endpoint, not a Meshway peer identity.
    #[schema(min_length = 3, max_length = 64)]
    pub observer: String,
    pub outcome: StunDiscoveryOutcome,
    #[schema(required = true, min_length = 3, max_length = 64)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub mapped_endpoint: Option<String>,
    #[schema(required = true, format = Int64, minimum = 0, maximum = 4294967295_u64)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub rtt_ms: Option<u32>,
    /// Repeated reports preserve these times; retransmission is not renewal.
    pub observed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UdpSocketDiscovery {
    pub resource_kind: UdpDiscoveryResourceKind,
    /// Controller listener/candidate ID, never a private runtime token.
    #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9._-]+$")]
    pub resource_id: String,
    /// Public process-local runtime generation and socket ordinal.
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub generation: u32,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub socket_id: u32,
    #[schema(min_length = 3, max_length = 64)]
    pub bound_address: String,
    pub source: EndpointDiscoverySource,
    #[schema(max_items = 4)]
    pub observations: Vec<StunEndpointObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportEndpointDiscoveryRequest {
    /// Reuses the authenticated managed runtime's acquired epoch, not a new identity.
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub epoch: u32,
    /// Independent monotonic discovery report sequence within that epoch.
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub sequence: u32,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub digest: String,
    pub state: EndpointDiscoveryState,
    #[schema(max_items = 128)]
    pub sockets: Vec<UdpSocketDiscovery>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportEndpointDiscoveryResponse {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub epoch: u32,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub sequence: u32,
    pub reported_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeEndpointDiscovery {
    pub node_id: String,
    #[schema(format = Int64, minimum = 0, maximum = 4294967295_u64)]
    pub epoch: u32,
    #[schema(format = Int64, minimum = 0, maximum = 4294967295_u64)]
    pub sequence: u32,
    #[schema(required = true, format = Int64, minimum = 1, maximum = 4294967295_u64)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub revision: Option<u32>,
    #[schema(
        required = true,
        min_length = 64,
        max_length = 64,
        pattern = "^[0-9a-f]{64}$"
    )]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub digest: Option<String>,
    pub state: EndpointDiscoveryState,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub reported_at: Option<DateTime<Utc>>,
    pub current: bool,
    pub stale: bool,
    #[schema(max_items = 128)]
    pub sockets: Vec<UdpSocketDiscovery>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EndpointDiscoveriesSnapshot {
    pub network_id: String,
    pub nodes: Vec<NodeEndpointDiscovery>,
}
