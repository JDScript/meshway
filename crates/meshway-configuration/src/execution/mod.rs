//! Complete, immutable node execution targets. No runtime state grants authority.
use crate::{
    AddressPools, DnsConfiguration, DnsTransport, NodeAddresses, NodeIdentity,
    object::{object, tagged},
};
use meshway_model::{BoundNodeKey, Digest, NetworkId, NodeId, PublicationId, RequestId, Revision};
use meshway_transport_spec::*;
pub use meshway_transport_spec::{ClientTls, DialParameters};
use std::{collections::BTreeMap, net::IpAddr};
mod admission;
mod diagnostics;
mod exit;
pub use exit::*;
mod obsolete;
pub use diagnostics::*;
pub use obsolete::{ObsoleteSliceError, ObsoleteSliceHeader, validate_obsolete_slice};

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

/// Numeric IPv4 or IPv6 business destination used in an execution binding.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(transparent)]
#[schema(value_type = String)]
pub struct ExecutionAddress(
    /// Exact destination address, serialized as an IP address string.
    pub IpAddr,
);

meshway_serde::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
/// Inner business protocol whose access scope the Server evaluates.
pub enum BusinessProtocol {
    /// TCP traffic scoped to a destination port.
    Tcp,
    /// UDP traffic scoped to a destination port.
    Udp,
    /// ICMP echo traffic without a transport port.
    IcmpEcho,
}
}

tagged! {
/// Exact business destination submitted in a demand and bound into its compiled permit.
pub enum BusinessTarget {
    /// Address a registered Mesh Node.
    Mesh {
        /// Destination Node in the demand's network.
        node: NodeId
    },
    /// Address a numeric destination, classified against the configured Mesh pools.
    Ip {
        /// Exact destination IP address.
        address: ExecutionAddress
    },
    /// Address a domain through an IP association that must have a current trusted DNS input.
    Domain {
        /// Domain name checked against the source's trusted DNS association.
        name: String,
        /// Destination IP that must belong to that association.
        address: ExecutionAddress
    },
} }
object! {
/// Server-bound business demand compiled together with the network's other live demands.
pub struct Demand {
    /// Stable demand identifier retained across creation retries and renewal.
    pub id: RequestId,
    /// Source Node determined by the authenticated request.
    pub source: NodeId,
    /// Exact requested business destination.
    pub target: BusinessTarget,
    /// Inner protocol whose access and routing scope is evaluated.
    pub protocol: BusinessProtocol,
    /// Destination port for TCP or UDP; null or omitted only for ICMP echo.
    pub port: Option<Port>,
    /// Source routing profile that must match its authorized selection.
    pub profile: String,
    /// Demand deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339.
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }
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
/// Trusted source-scoped DNS input used by compilation to bind a domain to destination addresses.
pub struct TrustedDnsAssociation {
    /// Network to which this DNS input belongs.
    pub network_id: NetworkId,
    /// Exact source configuration revision under which the association was obtained.
    pub configuration_revision: Revision,
    /// Node whose domain demand may use the association.
    pub source: NodeId,
    /// Resolved domain name matched by the demand.
    pub name: String,
    /// CNAME chain retained with the trusted resolution result.
    pub cname_chain: Vec<String>,
    /// Destination addresses supported by this resolution result.
    pub addresses: Vec<ExecutionAddress>,
    /// Association deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339; dependent permits cannot outlive it.
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
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
object! {
/// Trusted identity and addresses of a peer required by the compiled execution paths.
pub struct PeerConfiguration {
    /// Peer authorization identity from the same source configuration.
    pub identity: NodeIdentity,
    /// Peer Mesh addresses used by path and packet admission.
    pub addresses: NodeAddresses,
    /// Peer Ed25519 key binding used for control identity proofs.
    pub control_key: BoundNodeKey,
    /// Peer X25519 key binding used for end-to-end protection.
    pub dataplane_key: BoundNodeKey,
} }
meshway_serde::string_enum! {
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
/// Complete ordered path authorized by Server compilation, including all required intermediate Nodes.
pub struct LogicalPath {
    /// Ordered source, intermediate Nodes and terminal Node or exit.
    pub nodes: Vec<NodeId>,
    /// Candidate keys grouped by the actual dial initiator, independent of business direction.
    pub dial_candidates: BTreeMap<NodeId, Vec<String>>,
} }
object! {
/// Server-compiled business authority shared by the source, relays and terminal of each allowed complete path.
pub struct PathPermit {
    /// Business demand for which this permit was compiled.
    pub demand_id: RequestId,
    /// Authenticated original business source, unchanged across path segments.
    pub source: NodeId,
    /// Configured source Mesh addresses checked during packet admission.
    pub source_addresses: NodeAddresses,
    /// Authorized business destination after Mesh address classification.
    pub target: BusinessTarget,
    /// Authorized inner business protocol.
    pub protocol: BusinessProtocol,
    /// Exact TCP or UDP destination port; null or omitted only for ICMP echo.
    pub port: Option<Port>,
    /// Source routing profile used to compile the permit.
    pub profile: String,
    /// Required execution constraints for an external exit; absent for Mesh services.
    pub exit: Option<ExitPermit>,
    /// Allowed complete paths; a flow retains its established path.
    pub paths: Vec<LogicalPath>,
    /// Permit deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339 and bounded by demand and dependency deadlines.
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
    /// Always reverse the established complete path, never evaluate reverse routing for replies.
    pub replies: ReplyPolicy,
} }
meshway_serde::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
/// Reply-path constraint attached to a compiled business permit.
pub enum ReplyPolicy {
    /// Admit replies only for an established forward flow along its complete reversed path.
    StatefulExactReversePath,
}
}
tagged! {
/// Server compilation result for a business demand; feedback alone does not authorize socket I/O.
pub enum DemandDecision {
    /// The demand has authorized complete Mesh paths.
    Permitted {
        /// Exact business scope and allowed paths compiled for the demand.
        permit: PathPermit
    },
    /// Routing selected direct local egress, whose execution requires the matching local-egress permit.
    Direct,
    /// Routing selected unmanaged local egress, whose execution requires the matching local-egress permit.
    Unmanaged,
    /// Compilation could not produce executable authority for this demand.
    Unavailable {
        /// Authorization or dependency condition preventing execution.
        reason: UnavailableReason
    },
} }
meshway_serde::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
/// Local egress selection bound into a source-only execution permit.
pub enum LocalEgressAction {
    /// Execute the configured direct route outside a Mesh path.
    Direct,
    /// Leave the destination to unmanaged local routing.
    Unmanaged,
}
}
object! {
/// Server-compiled exact local egress binding published only to the business source.
pub struct LocalEgressPermit {
    /// Demand authorizing this local execution binding.
    pub demand_id: RequestId,
    /// Only Node allowed to consume the local binding.
    pub source: NodeId,
    /// Exact destination covered by the local route.
    pub target: BusinessTarget,
    /// Authorized inner business protocol.
    pub protocol: BusinessProtocol,
    /// Exact TCP or UDP destination port; null or omitted only for ICMP echo.
    pub port: Option<Port>,
    /// Routing profile under which the local action was selected.
    pub profile: String,
    /// Selected local routing action, without Mesh relay or exit admission.
    pub action: LocalEgressAction,
    /// Local permit deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339 and bounded by its dependencies.
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }
meshway_serde::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
/// Condition preventing Server compilation from granting the requested business execution.
pub enum UnavailableReason {
    /// Exit prerequisites are unknown, expired or failed.
    ExitUnavailable,
    /// Initial exit execution cannot forward this inner address family.
    UnsupportedFamily,
    /// Domain-dependent packet ownership cannot be compiled without trusted context.
    EgressConflict,
    /// Current Access rules do not authorize the requested business scope.
    Unauthorized,
    /// No eligible complete path or assigned Mesh destination satisfies the demand.
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
object! {
/// Internal resolver-service authority. It grants no external business traffic
/// and cannot produce a TrustedDnsAssociation. Enclosing publication and lease
/// remain mandatory at both endpoints and every relay.
pub struct DnsServicePermit {
    /// Original authenticated querying Node.
    pub source: NodeId,
    /// Source execution instance; replacement cannot inherit an old query.
    pub source_instance_id: meshway_model::InstanceId,
    /// Exact inner source addresses, never inferred from a socket peer.
    pub source_addresses: NodeAddresses,
    /// Only resolver Node permitted to terminate this service.
    pub exit: NodeId,
    /// Exact IPv4 Mesh resolver endpoint; the reserved port is 53.
    pub address: ExecutionAddress,
    /// Source's effective routing profile.
    pub profile: String,
    /// Same selection identity as source egress capture.
    pub generation: Digest,
    /// Exit instance and resource-role identity required by this service.
    pub role: ExitRolePlan,
    /// One complete ordered path, retained in reverse for replies.
    pub path: LogicalPath,
    /// Independently authorized transports; UDP does not imply TCP.
    pub transports: Vec<DnsTransport>,
    /// Maximum simultaneous queries from this source.
    pub max_pending: u16,
    /// Maximum query duration, also capped by permit and execution expiry.
    pub timeout_ms: u32,
    /// Permit deadline bounded by source instance and exit capability expiry.
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }
tagged! {
/// Server-compiled ordinary DNS selection, separate from business authorization.
pub enum DnsEgress {
    /// Compilation proved that the DNS selection has no exit possibility.
    NoExit,
    /// Ordinary queries use only this Mesh resolver service.
    ViaExit {
        /// Selected resolver Node.
        exit: NodeId,
        /// Exact service authority shared with its path participants.
        service: Box<DnsServicePermit>,
        /// Selection identity, equal to the service and capture generations.
        generation: Digest
    },
    /// Serve source grants using verified system
    /// upstreams. This node's own DNS selection has been proved to have no exit.
    ServeExitResolver {
        /// Exact source grants accepted by this resolver.
        permit: Vec<DnsServicePermit>
    },
    /// DNS egress lacks a required executable plan.
    Unavailable {
        /// Condition preventing DNS egress execution.
        reason: DnsEgressUnavailable
    },
} }
meshway_serde::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
/// Missing execution dependency for DNS egress.
pub enum DnsEgressUnavailable {
    /// A possible exit selection requires an explicit DNS plan that is not available.
    ExitPlanRequired,
    /// The selected exit is disabled or has no current ready capability.
    ExitUnavailable,
    /// No complete path can satisfy the required ordered waypoints.
    NoPath,
    /// Original source has no external Access qualification.
    Unauthorized,
}
}
object! {
/// DNS settings and trusted name bindings compiled for one Node's execution target.
pub struct DnsExecution {
    /// DNS configuration copied from the publication's source revision.
    pub settings: DnsConfiguration,
    /// Configured Mesh names and addresses filtered by this Node's source Access permissions.
    pub mesh_hosts: BTreeMap<String, NodeAddresses>,
    /// Current trusted DNS associations included for this source's domain demands.
    pub associations: Vec<TrustedDnsAssociation>,
    /// Regular resolution follows this selection; explicit resolver exceptions retain useWithExit.
    pub profile: String,
    /// Ordinary DNS selection; live authority also requires the execution lease.
    pub egress: DnsEgress,
} }
object! {
/// Complete private execution target compiled by the Server for one Node; installation also requires a matching valid lease.
pub struct NodeConfiguration {
    /// Network for which this target was compiled.
    pub network_id: NetworkId,
    /// Only Node intended to install this target.
    pub node_id: NodeId,
    /// Source configuration revision fixed by this publication.
    pub revision: Revision,
    /// Execution publication that fixes this target and its dependencies.
    pub publication: PublicationId,
    /// Digest of the trusted compilation inputs, including demand and identity dependencies.
    pub dependency_digest: Digest,
    /// Whether the source configuration enables this Node's business execution.
    pub enabled: bool,
    /// Configured canonical hostname, independent of the reported OS hostname.
    pub hostname: String,
    /// Configured User or Tag identity used for authorization.
    pub identity: NodeIdentity,
    /// Configured Mesh addresses used consistently by DNS and packet admission.
    pub addresses: NodeAddresses,
    /// This Node's trusted public key bindings and authenticated capabilities.
    pub self_keys: TrustedNode,
    /// Complete managed capture ranges, never peer inventory or business authority.
    pub mesh_address_pools: AddressPools,
    /// Disjoint external packet ownership and IPv6 negative capture boundary.
    pub egress_capture: EgressCapturePlan,
    /// Exit resource responsibility, issued before capability readiness.
    pub exit_role: Option<ExitRolePlan>,
    /// Complete local listener parameters and material indexed by configuration key.
    pub listeners: BTreeMap<String, PreparedListener>,
    /// Authorized outbound candidates indexed by Server-compiled candidate key.
    pub dialers: BTreeMap<String, DialCandidate>,
    /// Trusted identities and addresses of required execution peers.
    pub peers: BTreeMap<NodeId, PeerConfiguration>,
    /// Includes origin, relay and receiver admission. Absence rejects a new flow.
    pub permits: BTreeMap<RequestId, PathPermit>,
    /// Source-only, exact local execution bindings. Never Mesh/path admission.
    pub local_egress: BTreeMap<RequestId, LocalEgressPermit>,
    /// Authenticated measurement only. Never accepted by business packet admission.
    pub diagnostics: BTreeMap<meshway_model::ObservationId, DiagnosticPermit>,
    /// Server compilation results for this Node's live business demands.
    pub decisions: BTreeMap<RequestId, DemandDecision>,
    /// Compiled DNS settings, visible Mesh names and trusted associations.
    pub dns: DnsExecution,
    /// Configured exit eligibility gated by Node enablement, not proof of installed forwarding.
    pub exit_enabled: bool,
} }
object! {
/// Complete Server-compiled Node target paired with the digest checked before persistence and installation.
pub struct CompiledNode {
    /// Digest of the complete configuration value.
    pub digest: Digest,
    /// Immutable execution content whose digest is supplied.
    pub configuration: NodeConfiguration,
} }
object! {
/// Server compilation output fixing one network's source revision, dependencies and complete per-Node targets.
pub struct CompiledPublication {
    /// Network whose execution inputs were compiled.
    pub network_id: NetworkId,
    /// Source configuration revision used throughout the compilation.
    pub revision: Revision,
    /// Identifier assigned to this execution publication.
    pub publication: PublicationId,
    /// Digest binding the shared trusted compilation inputs.
    pub dependency_digest: Digest,
    /// Compiler contract version used to recognize compatible derived targets.
    pub compiler_version: String,
    /// Complete compiled targets indexed by receiving Node.
    pub nodes: BTreeMap<NodeId, CompiledNode>,
} }
