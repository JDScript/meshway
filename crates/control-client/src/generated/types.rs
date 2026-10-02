//! Generated types from OpenAPI specification
//!
//! This file contains all the generated types for the API.
//! Do not edit manually - regenerate using the appropriate script.
#![allow(clippy::large_enum_variant)]
#![allow(clippy::format_in_format_args)]
#![allow(clippy::let_unit_value)]
#![allow(unreachable_patterns)]
use serde::{Deserialize, Serialize};
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReportLatencySamplesRequest {
    ///Constraint: minItems=1, maxItems=64
    pub samples: Vec<LatencySample>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReportEndpointDiscoveryRequest {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    ///Reuses the authenticated managed runtime's acquired epoch, not a new identity.
    ///Constraint: minimum=1, maximum=4294967295
    pub epoch: i64,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
    ///Independent monotonic discovery report sequence within that epoch.
    ///Constraint: minimum=1, maximum=4294967295
    pub sequence: i64,
    ///Constraint: maxItems=128
    pub sockets: Vec<UdpSocketDiscovery>,
    pub state: EndpointDiscoveryState,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReportConnectionRuntimeRequest {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub epoch: i64,
    ///Constraint: maxItems=128
    pub listeners: Vec<ConnectionListenerObservation>,
    ///Constraint: maxItems=256
    pub peers: Vec<ConnectionNeighborObservation>,
    pub reason: ConnectionRuntimeReason,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
    ///Constraint: minimum=1, maximum=4294967295
    pub sequence: i64,
    pub state: ConnectionRuntimeState,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishManagedTransportCapabilitiesRequest {
    ///Constraint: maxItems=2
    pub vless_streams: Vec<VlessStreamTransport>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishConnectionIdentityResponse {
    pub capabilities: Vec<ConnectionCapability>,
    pub node_id: String,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub public_key: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishConnectionIdentityRequest {
    ///Constraint: maxItems=3
    pub capabilities: Vec<ConnectionCapability>,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub public_key: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeshExecutionResponse {
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
    pub snapshot: MeshExecutionSnapshot,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeshExecutionSnapshot {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    pub payload: NodeMeshExecution,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeMeshExecution {
    pub connections: NodeConnections,
    ///Constraint: maxItems=64
    pub identities: Vec<ConnectionPeer>,
    ///Constraint: maxItems=64
    pub materials: Vec<ManagedVlessMaterial>,
    pub network: MeshNetwork,
    pub network_id: String,
    pub node: MeshNode,
    pub node_id: String,
    ///Constraint: maxItems=128
    pub paths: Vec<MeshExecutionPath>,
    ///Constraint: maxItems=1023
    pub peers: Vec<MeshNode>,
    pub required_capability: MeshExecutionCapability,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
    ///Constraint: minimum=1, maximum=1
    pub schema_version: i64,
    pub valid: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeshExecutionPath {
    ///Constraint: minItems=2, maxItems=8
    pub nodes: Vec<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LatencySample {
    /**Runtime returns exact channel metadata only with successful proof. A
    timeout leaves this empty rather than racing a separate registry read.
    Exactly one association for success, zero for timeout.*/
    ///Constraint: maxItems=1
    pub channel_associations: Vec<LatencyChannelAssociation>,
    pub completed_at: chrono::DateTime<chrono::Utc>,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    pub direction: LatencyDirection,
    /**One sanitized attempt failure for Failed, otherwise empty. This is not
    a network loss classification; local send failures are kept separate.*/
    ///Constraint: maxItems=1
    pub failure_reasons: Vec<LatencyFailureReason>,
    ///Actual probe invocation start; retained unchanged through queue/retry.
    pub measured_at: chrono::DateTime<chrono::Utc>,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub measurement_id: String,
    pub outcome: LatencyOutcome,
    pub peer_node_id: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
    ///Integer microseconds, measured using a monotonic clock, never HTTP RTT.
    ///Constraint: minimum=1, maximum=60000000
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub rtt_us: Option<i64>,
    ///Constraint: minimum=1, maximum=4294967295
    pub runtime_epoch: i64,
    ///Local sampler generation, not an assertion of physical channel identity.
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub sampler_generation: String,
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum LatencyOutcome {
    #[default]
    #[serde(rename = "success")]
    Success,
    #[serde(rename = "timeout")]
    Timeout,
    #[serde(rename = "failed")]
    Failed,
}
impl LatencyOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
        }
    }
}
impl ::std::fmt::Display for LatencyOutcome {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for LatencyOutcome {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum LatencyFailureReason {
    #[default]
    #[serde(rename = "channel_unavailable")]
    ChannelUnavailable,
    #[serde(rename = "authentication_failed")]
    AuthenticationFailed,
    #[serde(rename = "local_send_failed")]
    LocalSendFailed,
}
impl LatencyFailureReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ChannelUnavailable => "channel_unavailable",
            Self::AuthenticationFailed => "authentication_failed",
            Self::LocalSendFailed => "local_send_failed",
        }
    }
}
impl ::std::fmt::Display for LatencyFailureReason {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for LatencyFailureReason {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum LatencyDirection {
    #[default]
    #[serde(rename = "source_to_peer")]
    SourceToPeer,
}
impl LatencyDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SourceToPeer => "source_to_peer",
        }
    }
}
impl ::std::fmt::Display for LatencyDirection {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for LatencyDirection {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LatencyChannelAssociation {
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub candidate_id: Option<String>,
    pub initiator_node_id: String,
    pub listener_id: String,
    pub transport: ConnectionTransport,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UdpSocketDiscovery {
    ///Constraint: minLength=3, maxLength=64
    pub bound_address: String,
    ///Public process-local runtime generation and socket ordinal.
    ///Constraint: minimum=1, maximum=4294967295
    pub generation: i64,
    ///Constraint: maxItems=4
    pub observations: Vec<StunEndpointObservation>,
    ///Controller listener/candidate ID, never a private runtime token.
    ///Constraint: minLength=1, maxLength=64, pattern=`^[A-Za-z0-9._-]+$`
    pub resource_id: String,
    pub resource_kind: UdpDiscoveryResourceKind,
    ///Constraint: minimum=1, maximum=4294967295
    pub socket_id: i64,
    pub source: EndpointDiscoverySource,
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum UdpDiscoveryResourceKind {
    #[default]
    #[serde(rename = "listener")]
    Listener,
    #[serde(rename = "outgoing_candidate")]
    OutgoingCandidate,
}
impl UdpDiscoveryResourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Listener => "listener",
            Self::OutgoingCandidate => "outgoing_candidate",
        }
    }
}
impl ::std::fmt::Display for UdpDiscoveryResourceKind {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for UdpDiscoveryResourceKind {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StunEndpointObservation {
    pub expires_at: chrono::DateTime<chrono::Utc>,
    ///Constraint: minLength=3, maxLength=64
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub mapped_endpoint: Option<String>,
    ///Repeated reports preserve these times; retransmission is not renewal.
    pub observed_at: chrono::DateTime<chrono::Utc>,
    ///Explicit numeric STUN observer endpoint, not a Meshway peer identity.
    ///Constraint: minLength=3, maxLength=64
    pub observer: String,
    pub outcome: StunDiscoveryOutcome,
    ///Constraint: minimum=0, maximum=4294967295
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub rtt_ms: Option<i64>,
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum StunDiscoveryOutcome {
    #[default]
    #[serde(rename = "mapped")]
    Mapped,
    #[serde(rename = "timeout")]
    Timeout,
    #[serde(rename = "invalid_response")]
    InvalidResponse,
    #[serde(rename = "io_error")]
    IoError,
    #[serde(rename = "stun_error")]
    StunError,
}
impl StunDiscoveryOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mapped => "mapped",
            Self::Timeout => "timeout",
            Self::InvalidResponse => "invalid_response",
            Self::IoError => "io_error",
            Self::StunError => "stun_error",
        }
    }
}
impl ::std::fmt::Display for StunDiscoveryOutcome {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for StunDiscoveryOutcome {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum EndpointDiscoverySource {
    #[default]
    #[serde(rename = "managed_udp_socket_stun")]
    ManagedUdpSocketStun,
}
impl EndpointDiscoverySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ManagedUdpSocketStun => "managed_udp_socket_stun",
        }
    }
}
impl ::std::fmt::Display for EndpointDiscoverySource {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for EndpointDiscoverySource {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum EndpointDiscoveryState {
    #[default]
    #[serde(rename = "awaiting_report")]
    AwaitingReport,
    #[serde(rename = "disabled")]
    Disabled,
    #[serde(rename = "probing")]
    Probing,
    #[serde(rename = "observed")]
    Observed,
    #[serde(rename = "withdrawn")]
    Withdrawn,
    #[serde(rename = "expired")]
    Expired,
    #[serde(rename = "error")]
    Error,
}
impl EndpointDiscoveryState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AwaitingReport => "awaiting_report",
            Self::Disabled => "disabled",
            Self::Probing => "probing",
            Self::Observed => "observed",
            Self::Withdrawn => "withdrawn",
            Self::Expired => "expired",
            Self::Error => "error",
        }
    }
}
impl ::std::fmt::Display for EndpointDiscoveryState {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for EndpointDiscoveryState {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum ConnectionRuntimeState {
    #[default]
    #[serde(rename = "awaiting_report")]
    AwaitingReport,
    #[serde(rename = "disabled")]
    Disabled,
    #[serde(rename = "applying")]
    Applying,
    #[serde(rename = "applied")]
    Applied,
    #[serde(rename = "error")]
    Error,
    #[serde(rename = "withdrawn")]
    Withdrawn,
}
impl ConnectionRuntimeState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AwaitingReport => "awaiting_report",
            Self::Disabled => "disabled",
            Self::Applying => "applying",
            Self::Applied => "applied",
            Self::Error => "error",
            Self::Withdrawn => "withdrawn",
        }
    }
}
impl ::std::fmt::Display for ConnectionRuntimeState {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for ConnectionRuntimeState {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionNeighborObservation {
    ///Constraint: minLength=1, maxLength=64, pattern=`^[A-Za-z0-9._-]+$`
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub candidate_id: Option<String>,
    ///Constraint: minLength=1, maxLength=64, pattern=`^[A-Za-z0-9._-]+$`
    pub initiator_node_id: String,
    ///Constraint: minLength=1, maxLength=64, pattern=`^[A-Za-z0-9._-]+$`
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub listener_id: Option<String>,
    ///Constraint: minLength=1, maxLength=64, pattern=`^[A-Za-z0-9._-]+$`
    pub peer_node_id: String,
    ///A recent authenticated control-nonce probe, not merely a control heartbeat.
    pub probe_fresh: bool,
    ///Constraint: minimum=0, maximum=4294967295
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub probe_rtt_ms: Option<i64>,
    pub reason: ConnectionRuntimeReason,
    pub state: ConnectionNeighborState,
    pub transport: ConnectionTransport,
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum ConnectionNeighborState {
    #[default]
    #[serde(rename = "authenticated")]
    Authenticated,
    #[serde(rename = "waiting")]
    Waiting,
    #[serde(rename = "unreachable")]
    Unreachable,
}
impl ConnectionNeighborState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Authenticated => "authenticated",
            Self::Waiting => "waiting",
            Self::Unreachable => "unreachable",
        }
    }
}
impl ::std::fmt::Display for ConnectionNeighborState {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for ConnectionNeighborState {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionListenerObservation {
    ///Constraint: minLength=1, maxLength=64, pattern=`^[A-Za-z0-9._-]+$`
    pub listener_id: String,
    pub reason: ConnectionRuntimeReason,
    pub state: ConnectionListenerState,
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum ConnectionRuntimeReason {
    #[default]
    #[serde(rename = "none")]
    None,
    #[serde(rename = "disabled")]
    Disabled,
    #[serde(rename = "invalid_configuration")]
    InvalidConfiguration,
    #[serde(rename = "profile_unavailable")]
    ProfileUnavailable,
    #[serde(rename = "bind_failed")]
    BindFailed,
    #[serde(rename = "resolve_failed")]
    ResolveFailed,
    #[serde(rename = "transport_failed")]
    TransportFailed,
    #[serde(rename = "authentication_failed")]
    AuthenticationFailed,
    #[serde(rename = "probe_failed")]
    ProbeFailed,
    #[serde(rename = "lease_superseded")]
    LeaseSuperseded,
    #[serde(rename = "lease_expired")]
    LeaseExpired,
    #[serde(rename = "shutting_down")]
    ShuttingDown,
    #[serde(rename = "internal_error")]
    InternalError,
}
impl ConnectionRuntimeReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Disabled => "disabled",
            Self::InvalidConfiguration => "invalid_configuration",
            Self::ProfileUnavailable => "profile_unavailable",
            Self::BindFailed => "bind_failed",
            Self::ResolveFailed => "resolve_failed",
            Self::TransportFailed => "transport_failed",
            Self::AuthenticationFailed => "authentication_failed",
            Self::ProbeFailed => "probe_failed",
            Self::LeaseSuperseded => "lease_superseded",
            Self::LeaseExpired => "lease_expired",
            Self::ShuttingDown => "shutting_down",
            Self::InternalError => "internal_error",
        }
    }
}
impl ::std::fmt::Display for ConnectionRuntimeReason {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for ConnectionRuntimeReason {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum ConnectionListenerState {
    #[default]
    #[serde(rename = "bound")]
    Bound,
    #[serde(rename = "error")]
    Error,
}
impl ConnectionListenerState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bound => "bound",
            Self::Error => "error",
        }
    }
}
impl ::std::fmt::Display for ConnectionListenerState {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for ConnectionListenerState {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationSnapshot {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    pub payload: NodeConfiguration,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfiguration {
    pub network: MeshNetwork,
    pub node: MeshNode,
    pub peers: Vec<MeshNode>,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
    ///Constraint: minimum=1, maximum=4294967295
    pub schema_version: i64,
}
///Public active-node inventory; country is administrator-managed metadata.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeshNode {
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub country: Option<String>,
    pub id: String,
    pub mesh_ipv4: String,
    pub mesh_ipv6: String,
    pub name: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeshNetwork {
    pub ipv4_prefix: String,
    pub ipv6_prefix: String,
}
///Also lets a pending local enrollment recover its server-assigned node ID.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentNodeResponse {
    pub node: NodeSummary,
}
///Durable control-plane identity. Last contact is not tunnel health.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSummary {
    ///Constraint: minimum=0, maximum=4294967295
    pub acknowledged_revision: i64,
    pub agent_version: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub configuration_revision: i64,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub country: Option<String>,
    pub id: String,
    pub last_seen: chrono::DateTime<chrono::Utc>,
    pub mesh_ipv4: String,
    pub mesh_ipv6: String,
    pub name: String,
    pub registered_at: chrono::DateTime<chrono::Utc>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentManagedConnectionsSnapshot {
    pub connections: AgentConnectionsSnapshot,
    pub digest: String,
    ///Constraint: maxItems=128
    pub materials: Vec<ManagedVlessMaterial>,
    pub network_id: String,
}
///No Debug: serialization is permitted only on the node-authenticated endpoint.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedVlessMaterial {
    ///Constraint: minLength=1, maxLength=32768
    pub certificate_pem: String,
    pub certificate_sha256: String,
    pub credential_ref: String,
    pub listener_id: String,
    ///Only listener owner receives this; dialers receive null.
    ///Constraint: minLength=1, maxLength=16384
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub private_key_pem: Option<String>,
    pub settings: ManagedVlessSettings,
    pub uuid: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub version: i64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedVlessSettings {
    ///Certificate name and client verification name, independent of HTTP Host.
    ///Constraint: minLength=1, maxLength=253
    pub server_name: String,
    pub stream: VlessStreamTransport,
    ///Constraint: minLength=1, maxLength=253
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub websocket_host: Option<String>,
    ///Constraint: minLength=1, maxLength=1024
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub websocket_path: Option<String>,
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum VlessStreamTransport {
    #[default]
    #[serde(rename = "tcp")]
    Tcp,
    #[serde(rename = "websocket")]
    Websocket,
}
impl VlessStreamTransport {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Websocket => "websocket",
        }
    }
}
impl ::std::fmt::Display for VlessStreamTransport {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for VlessStreamTransport {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDnsSnapshot {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    pub payload: NodeDnsConfiguration,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeDnsConfiguration {
    ///Constraint: maxItems=64
    pub diagnostics: Vec<PolicyDiagnostic>,
    pub dns: MeshDnsConfiguration,
    ///Constraint: minLength=32, maxLength=32, pattern=`^[0-9a-f]{32}$`
    pub network_id: String,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub node_id: String,
    ///Constraint: maxItems=1024
    pub records: Vec<NodeDnsRecord>,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
    ///Constraint: minimum=1, maximum=1
    pub schema_version: i32,
}
///Bounded diagnostics shared by address and DNS publication.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyDiagnostic {
    pub field: String,
    pub message: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub rule_id: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeDnsRecord {
    ///Constraint: maxItems=1
    pub aliases: Vec<String>,
    ///Constraint: minLength=1, maxLength=253, pattern=`^id\.[0-9a-f]{32}\.[0-9a-f]{32}\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$`
    pub canonical_name: String,
    pub mesh_ipv4: String,
    pub mesh_ipv6: String,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub node_id: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeshDnsConfiguration {
    pub enabled: bool,
    ///Constraint: minimum=1, maximum=300
    pub ttl_seconds: i32,
    ///Constraint: minLength=3, maxLength=128
    pub zone: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConnectionsSnapshot {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    pub payload: NodeConnections,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConnections {
    pub capabilities: Vec<ConnectionCapability>,
    pub diagnostics: Vec<ConnectionDiagnostic>,
    pub incoming: Vec<IncomingConnectionApproval>,
    pub listeners: Vec<ConnectionListener>,
    pub node_id: String,
    pub outgoing: Vec<OutgoingConnectionCandidate>,
    pub peers: Vec<ConnectionPeer>,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub public_key: Option<String>,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
    ///Constraint: minimum=1, maximum=4294967295
    pub schema_version: i64,
    pub valid: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OutgoingConnectionCandidate {
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub credential_ref: Option<String>,
    pub endpoint: ConnectionEndpoint,
    pub id: String,
    pub listener_id: String,
    pub peer_node_id: String,
    ///Constraint: minimum=0, maximum=65535
    pub priority: i32,
    pub transport: ConnectionTransport,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IncomingConnectionApproval {
    pub candidate_id: String,
    pub listener_id: String,
    pub peer_node_id: String,
    pub transport: ConnectionTransport,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionPeer {
    pub node_id: String,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub public_key: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionListener {
    ///Constraint: maxItems=16
    pub advertised_endpoints: Vec<ConnectionEndpoint>,
    pub bind: String,
    ///Constraint: minLength=1, maxLength=64, pattern=`^[A-Za-z0-9._-]+$`
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub credential_ref: Option<String>,
    ///Constraint: minLength=1, maxLength=64, pattern=`^[A-Za-z0-9._-]+$`
    pub id: String,
    pub node_id: String,
    pub transport: ConnectionTransport,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionEndpoint {
    ///Constraint: minLength=1, maxLength=253
    pub host: String,
    ///Constraint: minimum=1, maximum=65535
    pub port: i32,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionDiagnostic {
    pub field: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub item_id: Option<String>,
    pub message: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionCapability {
    pub can_accept: bool,
    pub can_dial: bool,
    pub semantics: ConnectionSemantics,
    pub transport: ConnectionTransport,
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum ConnectionTransport {
    #[default]
    #[serde(rename = "native_udp")]
    NativeUdp,
    #[serde(rename = "native_tcp")]
    NativeTcp,
    #[serde(rename = "vless")]
    Vless,
}
impl ConnectionTransport {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NativeUdp => "native_udp",
            Self::NativeTcp => "native_tcp",
            Self::Vless => "vless",
        }
    }
}
impl ::std::fmt::Display for ConnectionTransport {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for ConnectionTransport {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum ConnectionSemantics {
    #[default]
    #[serde(rename = "datagram")]
    Datagram,
    #[serde(rename = "stream")]
    Stream,
}
impl ConnectionSemantics {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Datagram => "datagram",
            Self::Stream => "stream",
        }
    }
}
impl ::std::fmt::Display for ConnectionSemantics {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for ConnectionSemantics {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeMeshExecutionRequest {
    pub digest: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
    pub runtime: MeshExecutionRequest,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeshExecutionRequest {
    ///Constraint: maxItems=1
    pub capabilities: Vec<MeshExecutionCapability>,
    ///Constraint: minimum=1, maximum=4294967295
    pub epoch: i64,
    pub instance_id: String,
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum MeshExecutionCapability {
    #[default]
    #[serde(rename = "ipv4_exact_path_v1")]
    Ipv4ExactPathV1,
}
impl MeshExecutionCapability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ipv4ExactPathV1 => "ipv4_exact_path_v1",
        }
    }
}
impl ::std::fmt::Display for MeshExecutionCapability {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for MeshExecutionCapability {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeAgentConnectionsResponse {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
    pub status: ConnectionReceiptStatus,
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum ConnectionReceiptStatus {
    #[default]
    #[serde(rename = "saved")]
    Saved,
}
impl ConnectionReceiptStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Saved => "saved",
        }
    }
}
impl ::std::fmt::Display for ConnectionReceiptStatus {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for ConnectionReceiptStatus {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeAgentConnectionsRequest {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeConfigurationRequest {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeConfigurationResponse {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeMeshExecutionResponse {
    pub digest: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
    pub saved: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BeginConnectionRuntimeRequest {
    ///Pin the initial epoch read across retries; never refresh after a conflict.
    ///Constraint: minimum=0, maximum=4294967295
    pub expected_epoch: i64,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub instance_id: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BeginConnectionRuntimeResponse {
    ///Constraint: minimum=1, maximum=4294967295
    pub epoch: i64,
}
/**Persist the proposed random credential before sending this request. Retry
with the same enrollment token, credential, name and enrollment version.*/
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollAgentRequest {
    ///Constraint: minLength=1, maxLength=64, pattern=`[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]`
    pub agent_version: String,
    ///Constraint: minLength=1, maxLength=128, pattern=`[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]`
    pub name: String,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub node_credential: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    pub message: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExchangeDeviceRequest {
    ///Constraint: minLength=1, maxLength=64, pattern=`[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]`
    pub agent_version: String,
    ///Constraint: minLength=43, maxLength=128, pattern=`^[A-Za-z0-9._~-]{43,128}$`
    pub code_verifier: String,
    ///Constraint: minLength=1, maxLength=128, pattern=`[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]`
    pub name: String,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub node_credential: String,
    ///Constraint: minLength=25, maxLength=25, pattern=`^meshway://enroll/callback$`
    pub redirect_uri: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetConnectionRuntimeEpochResponse {
    ///Zero means this node has never acquired a runtime lease.
    ///Constraint: minimum=0, maximum=4294967295
    pub epoch: i64,
}
///Node identity is derived exclusively from the node bearer credential.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatRequest {
    ///Constraint: minLength=1, maxLength=64, pattern=`[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]`
    pub agent_version: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishManagedTransportCapabilitiesResponse {
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReportConnectionRuntimeResponse {
    ///Constraint: minimum=1, maximum=4294967295
    pub epoch: i64,
    pub reported_at: chrono::DateTime<chrono::Utc>,
    ///Constraint: minimum=1, maximum=4294967295
    pub sequence: i64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReportEndpointDiscoveryResponse {
    ///Constraint: minimum=1, maximum=4294967295
    pub epoch: i64,
    pub reported_at: chrono::DateTime<chrono::Utc>,
    ///Constraint: minimum=1, maximum=4294967295
    pub sequence: i64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReportLatencySamplesResponse {
    ///Constraint: minimum=0, maximum=64
    pub duplicates: i64,
    ///Constraint: minimum=0, maximum=64
    pub inserted: i64,
}

// Generated from required nullable OpenAPI properties. deserialize_with makes
// serde require presence; Option accepts explicit null. Never skip None on output.
fn deserialize_required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

impl std::fmt::Debug for ReportLatencySamplesRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReportLatencySamplesRequest([REDACTED])")
    }
}

impl std::fmt::Debug for ReportEndpointDiscoveryRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReportEndpointDiscoveryRequest([REDACTED])")
    }
}

impl std::fmt::Debug for ReportConnectionRuntimeRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReportConnectionRuntimeRequest([REDACTED])")
    }
}

impl std::fmt::Debug for PublishManagedTransportCapabilitiesRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PublishManagedTransportCapabilitiesRequest([REDACTED])")
    }
}

impl std::fmt::Debug for PublishConnectionIdentityResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PublishConnectionIdentityResponse([REDACTED])")
    }
}

impl std::fmt::Debug for PublishConnectionIdentityRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PublishConnectionIdentityRequest([REDACTED])")
    }
}

impl std::fmt::Debug for MeshExecutionResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MeshExecutionResponse([REDACTED])")
    }
}

impl std::fmt::Debug for MeshExecutionSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MeshExecutionSnapshot([REDACTED])")
    }
}

impl std::fmt::Debug for NodeMeshExecution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NodeMeshExecution([REDACTED])")
    }
}

impl std::fmt::Debug for MeshExecutionPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MeshExecutionPath([REDACTED])")
    }
}

impl std::fmt::Debug for LatencySample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LatencySample([REDACTED])")
    }
}

impl std::fmt::Debug for LatencyOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LatencyOutcome([REDACTED])")
    }
}

impl std::fmt::Debug for LatencyFailureReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LatencyFailureReason([REDACTED])")
    }
}

impl std::fmt::Debug for LatencyDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LatencyDirection([REDACTED])")
    }
}

impl std::fmt::Debug for LatencyChannelAssociation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LatencyChannelAssociation([REDACTED])")
    }
}

impl std::fmt::Debug for UdpSocketDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UdpSocketDiscovery([REDACTED])")
    }
}

impl std::fmt::Debug for UdpDiscoveryResourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UdpDiscoveryResourceKind([REDACTED])")
    }
}

impl std::fmt::Debug for StunEndpointObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StunEndpointObservation([REDACTED])")
    }
}

impl std::fmt::Debug for StunDiscoveryOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StunDiscoveryOutcome([REDACTED])")
    }
}

impl std::fmt::Debug for EndpointDiscoverySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EndpointDiscoverySource([REDACTED])")
    }
}

impl std::fmt::Debug for EndpointDiscoveryState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EndpointDiscoveryState([REDACTED])")
    }
}

impl std::fmt::Debug for ConnectionRuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionRuntimeState([REDACTED])")
    }
}

impl std::fmt::Debug for ConnectionNeighborObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionNeighborObservation([REDACTED])")
    }
}

impl std::fmt::Debug for ConnectionNeighborState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionNeighborState([REDACTED])")
    }
}

impl std::fmt::Debug for ConnectionListenerObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionListenerObservation([REDACTED])")
    }
}

impl std::fmt::Debug for ConnectionRuntimeReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionRuntimeReason([REDACTED])")
    }
}

impl std::fmt::Debug for ConnectionListenerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionListenerState([REDACTED])")
    }
}

impl std::fmt::Debug for ConfigurationSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConfigurationSnapshot([REDACTED])")
    }
}

impl std::fmt::Debug for NodeConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NodeConfiguration([REDACTED])")
    }
}

impl std::fmt::Debug for MeshNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MeshNode([REDACTED])")
    }
}

impl std::fmt::Debug for MeshNetwork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MeshNetwork([REDACTED])")
    }
}

impl std::fmt::Debug for AgentNodeResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AgentNodeResponse([REDACTED])")
    }
}

impl std::fmt::Debug for NodeSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NodeSummary([REDACTED])")
    }
}

impl std::fmt::Debug for AgentManagedConnectionsSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AgentManagedConnectionsSnapshot([REDACTED])")
    }
}

impl std::fmt::Debug for ManagedVlessMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedVlessMaterial([REDACTED])")
    }
}

impl std::fmt::Debug for ManagedVlessSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedVlessSettings([REDACTED])")
    }
}

impl std::fmt::Debug for VlessStreamTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VlessStreamTransport([REDACTED])")
    }
}

impl std::fmt::Debug for AgentDnsSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AgentDnsSnapshot([REDACTED])")
    }
}

impl std::fmt::Debug for NodeDnsConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NodeDnsConfiguration([REDACTED])")
    }
}

impl std::fmt::Debug for PolicyDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PolicyDiagnostic([REDACTED])")
    }
}

impl std::fmt::Debug for NodeDnsRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NodeDnsRecord([REDACTED])")
    }
}

impl std::fmt::Debug for MeshDnsConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MeshDnsConfiguration([REDACTED])")
    }
}

impl std::fmt::Debug for AgentConnectionsSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AgentConnectionsSnapshot([REDACTED])")
    }
}

impl std::fmt::Debug for NodeConnections {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NodeConnections([REDACTED])")
    }
}

impl std::fmt::Debug for OutgoingConnectionCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OutgoingConnectionCandidate([REDACTED])")
    }
}

impl std::fmt::Debug for IncomingConnectionApproval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IncomingConnectionApproval([REDACTED])")
    }
}

impl std::fmt::Debug for ConnectionPeer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionPeer([REDACTED])")
    }
}

impl std::fmt::Debug for ConnectionListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionListener([REDACTED])")
    }
}

impl std::fmt::Debug for ConnectionEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionEndpoint([REDACTED])")
    }
}

impl std::fmt::Debug for ConnectionDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionDiagnostic([REDACTED])")
    }
}

impl std::fmt::Debug for ConnectionCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionCapability([REDACTED])")
    }
}

impl std::fmt::Debug for ConnectionTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionTransport([REDACTED])")
    }
}

impl std::fmt::Debug for ConnectionSemantics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionSemantics([REDACTED])")
    }
}

impl std::fmt::Debug for AcknowledgeMeshExecutionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AcknowledgeMeshExecutionRequest([REDACTED])")
    }
}

impl std::fmt::Debug for MeshExecutionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MeshExecutionRequest([REDACTED])")
    }
}

impl std::fmt::Debug for MeshExecutionCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MeshExecutionCapability([REDACTED])")
    }
}

impl std::fmt::Debug for AcknowledgeAgentConnectionsResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AcknowledgeAgentConnectionsResponse([REDACTED])")
    }
}

impl std::fmt::Debug for ConnectionReceiptStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionReceiptStatus([REDACTED])")
    }
}

impl std::fmt::Debug for AcknowledgeAgentConnectionsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AcknowledgeAgentConnectionsRequest([REDACTED])")
    }
}

impl std::fmt::Debug for AcknowledgeConfigurationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AcknowledgeConfigurationRequest([REDACTED])")
    }
}

impl std::fmt::Debug for AcknowledgeConfigurationResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AcknowledgeConfigurationResponse([REDACTED])")
    }
}

impl std::fmt::Debug for AcknowledgeMeshExecutionResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AcknowledgeMeshExecutionResponse([REDACTED])")
    }
}

impl std::fmt::Debug for BeginConnectionRuntimeRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BeginConnectionRuntimeRequest([REDACTED])")
    }
}

impl std::fmt::Debug for BeginConnectionRuntimeResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BeginConnectionRuntimeResponse([REDACTED])")
    }
}

impl std::fmt::Debug for EnrollAgentRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EnrollAgentRequest([REDACTED])")
    }
}

impl std::fmt::Debug for ErrorResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ErrorResponse([REDACTED])")
    }
}

impl std::fmt::Debug for ExchangeDeviceRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExchangeDeviceRequest([REDACTED])")
    }
}

impl std::fmt::Debug for GetConnectionRuntimeEpochResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GetConnectionRuntimeEpochResponse([REDACTED])")
    }
}

impl std::fmt::Debug for HeartbeatRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HeartbeatRequest([REDACTED])")
    }
}

impl std::fmt::Debug for PublishManagedTransportCapabilitiesResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PublishManagedTransportCapabilitiesResponse([REDACTED])")
    }
}

impl std::fmt::Debug for ReportConnectionRuntimeResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReportConnectionRuntimeResponse([REDACTED])")
    }
}

impl std::fmt::Debug for ReportEndpointDiscoveryResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReportEndpointDiscoveryResponse([REDACTED])")
    }
}

impl std::fmt::Debug for ReportLatencySamplesResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReportLatencySamplesResponse([REDACTED])")
    }
}
