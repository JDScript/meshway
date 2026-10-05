//! Complete, immutable node execution targets. No runtime state grants authority.
use crate::{
    AddressPools, DnsConfiguration, NodeAddresses, NodeIdentity,
    object::{object, tagged},
};
use meshway_model::{BoundNodeKey, Digest, NetworkId, NodeId, PublicationId, RequestId, Revision};
use meshway_transport_spec::*;
pub use meshway_transport_spec::{ClientTls, DialParameters};
use std::{collections::BTreeMap, net::IpAddr};
mod admission;
mod delta;
mod diagnostics;
pub use delta::*;
pub use diagnostics::*;

object! {
/// Actual socket establishment orientation, independent of business direction.
pub struct Carrier {
    pub dialer: NodeId, pub listener: NodeId, pub listener_key: String,
    pub protocol: TransportProtocol,
} }

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(transparent)]
#[schema(value_type = String)]
pub struct ExecutionAddress(pub IpAddr);

meshway_serde::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum BusinessProtocol {
    Tcp,
    Udp,
    IcmpEcho,
}
}

tagged! { pub enum BusinessTarget {
    Mesh { node: NodeId },
    Ip { address: ExecutionAddress },
    Domain { name: String, address: ExecutionAddress },
} }
object! { pub struct Demand {
    pub id: RequestId, pub source: NodeId, pub target: BusinessTarget,
    pub protocol: BusinessProtocol, pub port: Option<Port>,
    pub profile: String,
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }
object! { pub struct TrustedNode {
    pub control_key: BoundNodeKey, pub dataplane_key: BoundNodeKey,
    /// Authenticated acquire declaration; reported State never overrides it.
    pub capabilities: BTreeMap<String, meshway_model::state::Capability>,
} }
object! { pub struct TrustedDnsAssociation {
    pub network_id: NetworkId, pub configuration_revision: Revision,
    pub source: NodeId, pub name: String, pub cname_chain: Vec<String>,
    pub addresses: Vec<ExecutionAddress>,
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }
object! { pub struct RuleSetContent {
    pub network_id: NetworkId,
    pub artifact: meshway_model::ArtifactId,
    #[serde(rename = "ipCIDR")] pub ip_cidr: Vec<String>, pub domain_suffix: Vec<String>, pub domain: Vec<String>,
} }
object! { pub struct PeerConfiguration {
    pub identity: NodeIdentity, pub addresses: NodeAddresses,
    pub control_key: BoundNodeKey, pub dataplane_key: BoundNodeKey,
} }
object! { pub struct DialCandidate {
    pub peer: NodeId, pub listener_key: String, pub endpoints: Vec<Endpoint>,
    pub priority: u16, pub parameters: DialParameters,
} }
object! { pub struct LogicalPath {
    pub nodes: Vec<NodeId>,
    /// Candidate keys grouped by the actual dial initiator, independent of business direction.
    pub dial_candidates: BTreeMap<NodeId, Vec<String>>,
} }
object! { pub struct PathPermit {
    pub demand_id: RequestId, pub source: NodeId, pub source_addresses: NodeAddresses,
    pub target: BusinessTarget, pub protocol: BusinessProtocol, pub port: Option<Port>,
    pub profile: String, pub paths: Vec<LogicalPath>,
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
pub enum ReplyPolicy {
    StatefulExactReversePath,
}
}
tagged! { pub enum DemandDecision {
    Permitted { permit: PathPermit },
    Direct, Unmanaged,
    Unavailable { reason: UnavailableReason },
} }
meshway_serde::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum LocalEgressAction {
    Direct,
    Unmanaged,
}
}
object! { pub struct LocalEgressPermit {
    pub demand_id: RequestId, pub source: NodeId, pub target: BusinessTarget,
    pub protocol: BusinessProtocol, pub port: Option<Port>, pub profile: String,
    pub action: LocalEgressAction,
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }
meshway_serde::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum UnavailableReason {
    Unauthorized,
    NoPath,
    MissingCapability,
    MissingDependency,
    MissingDns,
    InvalidProfile,
    InactiveNode,
}
}
tagged! { pub enum DnsEgress {
    NoExit,
    Unavailable { reason: DnsEgressUnavailable },
} }
meshway_serde::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum DnsEgressUnavailable {
    ExitPlanRequired,
}
}
object! { pub struct DnsExecution {
    pub settings: DnsConfiguration,
    pub mesh_hosts: BTreeMap<String, NodeAddresses>,
    pub associations: Vec<TrustedDnsAssociation>,
    /// Regular resolution follows this selection; explicit resolver exceptions retain useWithExit.
    pub profile: String,
    /// Proved absence of exit selection only; never permission to open a socket.
    pub egress: DnsEgress,
} }
object! { pub struct NodeConfiguration {
    pub network_id: NetworkId, pub node_id: NodeId, pub revision: Revision,
    pub publication: PublicationId, pub dependency_digest: Digest,
    pub enabled: bool, pub hostname: String, pub identity: NodeIdentity,
    pub addresses: NodeAddresses, pub self_keys: TrustedNode,
    /// Complete managed capture ranges, never peer inventory or business authority.
    pub mesh_address_pools: AddressPools,
    pub listeners: BTreeMap<String, PreparedListener>, pub dialers: BTreeMap<String, DialCandidate>,
    pub peers: BTreeMap<NodeId, PeerConfiguration>,
    /// Includes origin, relay and receiver admission. Absence rejects a new flow.
    pub permits: BTreeMap<RequestId, PathPermit>,
    /// Source-only, exact local execution bindings. Never Mesh/path admission.
    pub local_egress: BTreeMap<RequestId, LocalEgressPermit>,
    /// Authenticated measurement only. Never accepted by business packet admission.
    pub diagnostics: BTreeMap<meshway_model::ObservationId, DiagnosticPermit>,
    pub decisions: BTreeMap<RequestId, DemandDecision>, pub dns: DnsExecution,
    pub exit_enabled: bool,
} }
object! { pub struct CompiledNode {
    pub digest: Digest, pub configuration: NodeConfiguration,
} }
object! { pub struct CompiledPublication {
    pub network_id: NetworkId, pub revision: Revision, pub publication: PublicationId,
    pub dependency_digest: Digest, pub compiler_version: String,
    pub nodes: BTreeMap<NodeId, CompiledNode>,
} }
