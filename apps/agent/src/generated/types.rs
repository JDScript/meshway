//! Generated types from OpenAPI specification
//!
//! This file contains all the generated types for the API.
//! Do not edit manually - regenerate using the appropriate script.
#![allow(clippy::large_enum_variant)]
#![allow(clippy::format_in_format_args)]
#![allow(clippy::let_unit_value)]
#![allow(unreachable_patterns)]
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishConnectionIdentityResponse {
    pub capabilities: Vec<ConnectionCapability>,
    pub node_id: String,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub public_key: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishConnectionIdentityRequest {
    ///Constraint: maxItems=3
    pub capabilities: Vec<ConnectionCapability>,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub public_key: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionListenerObservation {
    ///Constraint: minLength=1, maxLength=64, pattern=`^[A-Za-z0-9._-]+$`
    pub listener_id: String,
    pub reason: ConnectionRuntimeReason,
    pub state: ConnectionListenerState,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationSnapshot {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    pub payload: NodeConfiguration,
}
///Also lets a pending local enrollment recover its server-assigned node ID.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentNodeResponse {
    pub node: NodeSummary,
}
///Durable control-plane identity. Last contact is not tunnel health.
#[derive(Debug, Clone, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDataplaneSnapshot {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    pub payload: NodeDataplane,
}
///One coherent global revision, projected for the authenticated node.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeDataplane {
    pub access_policy: MeshPathAuthorization,
    pub access_valid: bool,
    pub configuration: NodeConfiguration,
    pub connections: NodeConnections,
    ///Constraint: maxItems=64
    pub identities: Vec<ConnectionPeer>,
    pub node_id: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
    ///Constraint: minimum=1, maximum=1
    pub schema_version: i64,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfiguration {
    pub diagnostics: Vec<PolicyDiagnostic>,
    pub network: MeshNetwork,
    pub node: MeshNode,
    pub peers: Vec<MeshNode>,
    pub policy_valid: bool,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
    pub routing_policy: RoutingPolicy,
    ///Constraint: minimum=1, maximum=4294967295
    pub schema_version: i64,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingPolicy {
    pub default_action: DefaultRouteAction,
    pub rules: Vec<RouteRule>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteRule {
    pub action: RouteAction,
    pub destination_any: Vec<DestinationSelector>,
    pub destination_scope: DestinationScope,
    pub id: String,
    pub source: SourceSelector,
}
///Empty dimensions are wildcards; dimensions combine with AND, members with OR.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSelector {
    pub countries: Vec<String>,
    pub node_ids: Vec<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteAction {
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub exit_node_id: Option<String>,
    pub kind: RouteActionKind,
    pub via: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum RouteActionKind {
    #[default]
    #[serde(rename = "route")]
    Route,
    #[serde(rename = "reject")]
    Reject,
}
impl RouteActionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Route => "route",
            Self::Reject => "reject",
        }
    }
}
impl ::std::fmt::Display for RouteActionKind {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for RouteActionKind {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DestinationSelector {
    pub kind: DestinationSelectorKind,
    pub value: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum DestinationSelectorKind {
    #[default]
    #[serde(rename = "node")]
    Node,
    #[serde(rename = "domain_suffix")]
    DomainSuffix,
    #[serde(rename = "cidr")]
    Cidr,
    #[serde(rename = "country")]
    Country,
}
impl DestinationSelectorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::DomainSuffix => "domain_suffix",
            Self::Cidr => "cidr",
            Self::Country => "country",
        }
    }
}
impl ::std::fmt::Display for DestinationSelectorKind {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for DestinationSelectorKind {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum DestinationScope {
    #[default]
    #[serde(rename = "any")]
    Any,
    #[serde(rename = "mesh")]
    Mesh,
    #[serde(rename = "internet")]
    Internet,
}
impl DestinationScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Mesh => "mesh",
            Self::Internet => "internet",
        }
    }
}
impl ::std::fmt::Display for DestinationScope {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for DestinationScope {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum DefaultRouteAction {
    #[default]
    #[serde(rename = "direct")]
    Direct,
    #[serde(rename = "reject")]
    Reject,
}
impl DefaultRouteAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Reject => "reject",
        }
    }
}
impl ::std::fmt::Display for DefaultRouteAction {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl AsRef<str> for DefaultRouteAction {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyDiagnostic {
    pub field: String,
    pub message: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub rule_id: Option<String>,
}
///Public active-node inventory; country is administrator-managed metadata.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeshNode {
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub country: Option<String>,
    pub id: String,
    pub mesh_ipv4: String,
    pub mesh_ipv6: String,
    pub name: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeshNetwork {
    pub ipv4_prefix: String,
    pub ipv6_prefix: String,
}
/**Internal compiled path authorization. Its JSON remains `{ "grants": [...] }`
so schema-version-1 NodeDataplane remains byte-contract compatible.*/
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeshPathAuthorization {
    ///Constraint: maxItems=128
    pub grants: Vec<MeshAccessGrant>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeshAccessGrant {
    ///Constraint: minLength=1, maxLength=64, pattern=`^[A-Za-z0-9._-]+$`
    pub id: String,
    ///Exact ordered full path, including source and destination. No repeats.
    ///Constraint: minItems=2, maxItems=8
    pub path: Vec<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConnectionsSnapshot {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    pub payload: NodeConnections,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IncomingConnectionApproval {
    pub candidate_id: String,
    pub listener_id: String,
    pub peer_node_id: String,
    pub transport: ConnectionTransport,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionPeer {
    pub node_id: String,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub public_key: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionListener {
    ///Constraint: minItems=1, maxItems=16
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
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionEndpoint {
    ///Constraint: minLength=1, maxLength=253
    pub host: String,
    ///Constraint: minimum=1, maximum=65535
    pub port: i32,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionDiagnostic {
    pub field: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub item_id: Option<String>,
    pub message: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionCapability {
    pub can_accept: bool,
    pub can_dial: bool,
    pub semantics: ConnectionSemantics,
    pub transport: ConnectionTransport,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeAgentConnectionsResponse {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
    pub status: ConnectionReceiptStatus,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeAgentConnectionsRequest {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeConfigurationRequest {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeConfigurationResponse {
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub digest: String,
    ///Constraint: minimum=1, maximum=4294967295
    pub revision: i64,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BeginConnectionRuntimeRequest {
    ///Pin the initial epoch read across retries; never refresh after a conflict.
    ///Constraint: minimum=0, maximum=4294967295
    pub expected_epoch: i64,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub instance_id: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BeginConnectionRuntimeResponse {
    ///Constraint: minimum=1, maximum=4294967295
    pub epoch: i64,
}
/**Persist the proposed random credential before sending this request. Retry
with the same enrollment token, credential, name and enrollment version.*/
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollAgentRequest {
    ///Constraint: minLength=1, maxLength=64, pattern=`[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]`
    pub agent_version: String,
    ///Constraint: minLength=1, maxLength=128, pattern=`[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]`
    pub name: String,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub node_credential: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    pub message: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetConnectionRuntimeEpochResponse {
    ///Zero means this node has never acquired a runtime lease.
    ///Constraint: minimum=0, maximum=4294967295
    pub epoch: i64,
}
///Node identity is derived exclusively from the node bearer credential.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatRequest {
    ///Constraint: minLength=1, maxLength=64, pattern=`[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]`
    pub agent_version: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReportConnectionRuntimeResponse {
    ///Constraint: minimum=1, maximum=4294967295
    pub epoch: i64,
    pub reported_at: chrono::DateTime<chrono::Utc>,
    ///Constraint: minimum=1, maximum=4294967295
    pub sequence: i64,
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
