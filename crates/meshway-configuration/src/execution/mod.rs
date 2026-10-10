//! Shared stable execution declarations and local decision vocabulary.
use crate::object::object;
use meshway_model::TransportProtocol;
use meshway_model::{BoundNodeKey, NetworkId, NodeId};
use meshway_transport_spec::*;
pub use meshway_transport_spec::{ClientTls, DialParameters};
use std::{collections::BTreeMap, net::IpAddr};
mod diagnostics;
mod exit;
pub use diagnostics::*;
pub use exit::*;

object! {
/// Actual socket establishment orientation, independent of business direction.
pub struct Carrier {
    /// Node that initiates the underlying connection.
    pub dialer: NodeId,
    /// Node that accepts the underlying connection.
    pub listener: NodeId,
    /// Configuration key of the accepting Node's listener.
    pub listener_key: String,
    /// Protocol encapsulation authorized for this adjacency.
    pub protocol: TransportProtocol,
} }

/// Numeric IPv4 or IPv6 destination used in an execution binding.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(transparent)]
#[schema(value_type = String)]
pub struct ExecutionAddress(pub IpAddr);

meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// Inner business protocol checked by local Access admission.
pub enum BusinessProtocol {
    /// TCP traffic scoped to a destination port.
    Tcp,
    /// UDP traffic scoped to a destination port.
    Udp,
    /// ICMP echo traffic without a transport port.
    IcmpEcho,
}
}

object! {
/// Authenticated key and capability inventory supplied to Server compilation.
pub struct TrustedNode {
    /// Current bound Ed25519 control public key and generation.
    pub control_key: BoundNodeKey,
    /// Current bound X25519 data-plane public key and generation.
    pub dataplane_key: BoundNodeKey,
    /// Authenticated acquire declaration; reported State never overrides it.
    pub capabilities: BTreeMap<String, meshway_model::state::Capability>,
} }
object! {
/// Resolved rule-set artifact content supplied as a fixed Server compilation dependency.
pub struct RuleSetContent {
    /// Network owning this resolved rule-set input.
    pub network_id: NetworkId,
    /// Exact artifact version whose content is supplied.
    pub artifact: meshway_model::ArtifactId,
    /// IP CIDR ranges used by rule-set matching.
    #[serde(rename = "ipCIDR")] pub ip_cidr: Vec<String>,
    /// Domain suffixes used by rule-set matching.
    pub domain_suffix: Vec<String>,
    /// Exact domain names used by rule-set matching.
    pub domain: Vec<String>,
} }

meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// Where a dial endpoint came from; configured endpoints are tried before Node-reported ones.
pub enum DialEndpointSource {
    /// Declared by the administrator on the peer listener.
    Configured,
    /// Reported by the peer Node from its own interfaces in State.
    Local,
    /// A STUN-observed public mapping of the peer's UDP listener socket.
    Stun,
}
}
object! {
/// One dialable address of a peer listener together with its provenance.
pub struct DialEndpoint {
    /// Host name or IP address.
    pub host: String,
    /// Listener port.
    pub port: Port,
    /// Provenance deciding the dial order.
    pub source: DialEndpointSource,
} }
impl DialEndpoint {
    pub fn endpoint(&self) -> Endpoint {
        Endpoint {
            host: self.host.clone(),
            port: self.port,
        }
    }
}
impl From<Endpoint> for DialEndpoint {
    fn from(endpoint: Endpoint) -> Self {
        Self {
            host: endpoint.host,
            port: endpoint.port,
            source: DialEndpointSource::Configured,
        }
    }
}
object! {
/// Server-compiled outbound candidate with resolved peer listener parameters and material.
pub struct DialCandidate {
    /// Peer accepting this candidate's underlying connection.
    pub peer: NodeId,
    /// Configuration key of the peer listener.
    pub listener_key: String,
    /// Configured then Node-reported endpoints; an empty Native UDP list requires coordinated acquisition.
    pub endpoints: Vec<DialEndpoint>,
    /// Rank of this candidate's carrier in the edge's transport order, lower first; endpoints of equal rank are tried by listener key.
    pub priority: u16,
    /// Complete protocol-specific client parameters and required material.
    pub parameters: DialParameters,
} }
object! {
/// Complete ordered path derived from routing, including all required intermediate Nodes.
pub struct LogicalPath {
    /// Ordered source, intermediate Nodes and terminal Node or exit.
    pub nodes: Vec<NodeId>,
    /// Candidate keys grouped by the actual dial initiator, independent of business direction.
    pub dial_candidates: BTreeMap<NodeId, Vec<String>>,
} }

meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// Local egress selection; actual I/O also requires its owner-issued guard.
pub enum LocalEgressAction {
    /// Execute the configured direct route outside a Mesh path.
    Direct,
    /// Leave the destination to unmanaged local routing.
    Unmanaged,
}
}
meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// Condition preventing requested execution.
pub enum UnavailableReason {
    /// Exit prerequisites are unknown, expired or failed.
    ExitUnavailable,
    /// Initial exit execution cannot forward this inner address family.
    UnsupportedFamily,
    /// Domain-dependent packet ownership cannot be compiled without trusted context.
    EgressConflict,
    /// Current Access rules do not authorize the requested business scope.
    Unauthorized,
    /// No eligible complete path or assigned Mesh destination satisfies the request.
    NoPath,
    /// A recursive routing dependency or repeated required node prevents a path.
    RoutingCycle,
    /// The bounded routing expansion or path search cannot complete.
    RoutingLimit,
    /// A capability required for execution is unavailable.
    MissingCapability,
    /// A trusted rule-set or other required compilation dependency is unavailable.
    MissingDependency,
    /// No current trusted DNS association binds the source's domain to the requested address.
    MissingDns,
    /// The requested profile is not allowed or does not match the source's current selection.
    InvalidProfile,
    /// The source is disabled or absent from the trusted Node inventory.
    InactiveNode,
}
}
meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// Missing execution dependency for DNS egress.
pub enum DnsEgressUnavailable {
    /// A possible exit selection requires an explicit DNS plan that is not available.
    ExitPlanRequired,
    /// The selected exit is disabled or lacks the required capability.
    ExitUnavailable,
    /// No complete path can satisfy the required ordered waypoints.
    NoPath,
    /// Original source has no external Access qualification.
    Unauthorized,
}
}
