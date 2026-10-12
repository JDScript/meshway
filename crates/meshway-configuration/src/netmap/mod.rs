//! Per-node netmap: a projection of one configuration revision plus the Node's
//! own resources. The Node runs the shared `meshway-policy` compiler on the
//! projection for each pair it terminates or relays; no per-pair grant or path
//! is materialized by the Server. Dynamic facts (endpoints, STUN, exit
//! readiness, instances, observation tasks) never enter it. Authority
//! additionally requires a current `NetmapLease` bound to the exact `NetmapRef`.
use crate::{
    AddressPools, DnsTransport, NodeAddresses, NodeIdentity, SerializationError,
    execution::{
        DialCandidate, DnsEgressUnavailable, EgressCapturePlan, ExecutionAddress, ExitNat,
        LogicalPath, TrustedNode,
    },
    object::{object, tagged, unique_map},
};
use meshway_model::{Digest, NetmapRef, NetworkId, NodeId, Revision, Version, exit::ExitFamily};
use meshway_transport::spec::PreparedListener;
use std::collections::BTreeMap;
mod delta;
mod projection;
mod services;
pub use delta::*;
pub use projection::*;
pub use services::*;

/// Contract family of every netmap produced by this compiler generation.
pub const NETMAP_COMPILER_VERSION: &str = "meshway-netmap-2";

tagged! {
/// Source-local action for one managed destination prefix.
pub enum EgressAction {
    /// Forward through the first ready exit, in this order; never direct.
    Exit {
        /// Ordered primary and backup exit Nodes.
        nodes: Vec<NodeId>,
        /// Ordered waypoints before the exit.
        via: Vec<NodeId>
    },
    /// Execute locally outside the Mesh.
    Direct,
    /// Leave to unmanaged local routing.
    Unmanaged,
} }

object! {
/// One prefix of the source's first-match egress partition.
pub struct EgressRoute {
    /// IPv4 or IPv6 prefix, disjoint from every other route.
    pub prefix: String,
    /// Action selected by the first matching rule or the profile fallback.
    pub action: EgressAction,
} }

object! {
/// Source egress ownership with the action of every partition prefix.
pub struct EgressPlan {
    /// Capture boundary handed to the packet owner; `generation` is the selection identity.
    pub capture: EgressCapturePlan,
    /// Complete disjoint partition of IPv4 and IPv6; empty when capture is unavailable.
    pub routes: Vec<EgressRoute>,
} }

object! {
/// Exit resource responsibility; never a grant to forward a packet.
pub struct ExitRole {
    /// Role identity, stable while the role content is unchanged.
    pub generation: Digest,
    /// Inner family implemented by this role.
    pub family: ExitFamily,
    /// Required translation mode.
    pub nat: ExitNat,
} }

object! {
/// Mesh resolver service between one source and one exit.
pub struct DnsService {
    /// Original querying Node.
    pub source: NodeId,
    /// Exact inner source addresses.
    pub source_addresses: NodeAddresses,
    /// Resolver Node terminating this service.
    pub exit: NodeId,
    /// Exact IPv4 Mesh resolver endpoint; port 53 is reserved.
    pub address: ExecutionAddress,
    /// Source routing profile.
    pub profile: String,
    /// Same selection identity as the source egress capture.
    pub generation: Digest,
    /// Exit role required at the resolver.
    pub role: ExitRole,
    /// One complete ordered path, reversed for replies.
    pub path: LogicalPath,
    /// Independently authorized transports.
    pub transports: Vec<DnsTransport>,
    /// Maximum simultaneous queries from this source.
    pub max_pending: u16,
    /// Maximum query duration in milliseconds.
    pub timeout_ms: u32,
} }

tagged! {
/// Ordinary DNS selection, separate from business grants.
pub enum DnsPlan {
    /// The DNS selection has no exit possibility.
    NoExit,
    /// Ordinary queries use only this Mesh resolver service.
    ViaExit {
        /// Exact service, equal to the copy held by the resolver.
        service: Box<DnsService>
    },
    /// Serve these sources with verified system upstreams.
    ServeExitResolver {
        /// Accepted source services.
        services: Vec<DnsService>
    },
    /// A required executable plan is missing.
    Unavailable {
        /// Condition preventing DNS egress.
        reason: DnsEgressUnavailable
    },
} }

meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// Local socket decision for one configured resolver endpoint.
pub enum ResolverAction {
    /// Query from a local socket; the route is direct.
    Direct,
    /// Query from a local socket; the route is unmanaged.
    Unmanaged,
    /// The route selects an exit; only the Mesh DNS service may carry it.
    Exit,
    /// Mesh, reserved or unroutable address; no local socket.
    Rejected,
}
}

object! {
/// Names and resolver decisions for one Node; settings are in the projection.
pub struct NetmapDns {
    /// Every name this Node may discover: itself and Nodes it has Access to.
    #[serde(deserialize_with = "unique_map")]
    pub mesh_hosts: BTreeMap<String, NodeAddresses>,
    /// Decision for each configured resolver key.
    #[serde(deserialize_with = "unique_map")]
    pub resolvers: BTreeMap<String, ResolverAction>,
    /// Effective routing profile used for ordinary resolution.
    pub profile: String,
    /// Ordinary DNS selection.
    pub egress: DnsPlan,
} }

object! {
/// This Node's own configured identity and resources.
pub struct NetmapSelf {
    /// Whether business execution is enabled.
    pub enabled: bool,
    /// Configured canonical hostname.
    pub hostname: String,
    /// Configured User or Tag identity.
    pub identity: NodeIdentity,
    /// Configured Mesh addresses.
    pub addresses: NodeAddresses,
    /// Trusted key bindings and authenticated capabilities.
    pub self_keys: TrustedNode,
    /// Complete managed Mesh pools.
    pub mesh_address_pools: AddressPools,
    /// Local listeners; empty while disabled.
    #[serde(deserialize_with = "unique_map")]
    pub listeners: BTreeMap<String, PreparedListener>,
    /// Configured exit eligibility gated by enablement.
    pub exit_enabled: bool,
} }

object! {
/// Complete per-node netmap content.
pub struct NodeNetmap {
    /// Network of this netmap.
    pub network_id: NetworkId,
    /// Only Node intended to install it.
    pub node_id: NodeId,
    /// Revision whose compilation first produced this content.
    pub revision: Revision,
    /// Per-node content sequence.
    pub serial: Version,
    /// Compiler contract family, `meshway-netmap-2`.
    pub compiler_version: String,
    /// Digest of the network compilation input that produced this serial.
    pub dependency_digest: Digest,
    /// Own configured identity and resources.
    #[serde(rename = "self")]
    pub self_: NetmapSelf,
    /// Configuration subset the shared compiler runs on.
    pub projection: ProjectedConfiguration,
    /// Dial candidates with configured endpoints and client material, from this Node to each topology-adjacent visible Node.
    #[serde(deserialize_with = "unique_map")]
    pub dialers: BTreeMap<String, DialCandidate>,
    /// Source egress plan; null while disabled.
    pub egress: Option<EgressPlan> [required = true],
    /// Exit role; null unless enabled, exit-enabled and capable.
    pub exit_role: Option<ExitRole> [required = true],
    /// DNS settings, names and plan.
    pub dns: NetmapDns,
} }

object! {
/// Compiled netmap paired with its content digest.
pub struct CompiledNetmap {
    /// Canonical content digest.
    pub digest: Digest,
    /// Complete content.
    pub netmap: std::sync::Arc<NodeNetmap> [value_type = NodeNetmap],
} }

impl NodeNetmap {
    /// Canonical digest of the content. Revision, serial and dependency digest
    /// identify the content's first compilation and are excluded.
    pub fn content_digest(&self) -> Result<Digest, SerializationError> {
        #[derive(serde::Serialize)]
        struct Content<'a> {
            domain: &'static str,
            network_id: NetworkId,
            node_id: NodeId,
            compiler_version: &'a str,
            self_: &'a NetmapSelf,
            projection: &'a ProjectedConfiguration,
            dialers: &'a BTreeMap<String, DialCandidate>,
            egress: &'a Option<EgressPlan>,
            exit_role: &'a Option<ExitRole>,
            dns: &'a NetmapDns,
        }
        crate::digest(&Content {
            domain: "meshway-netmap-content",
            network_id: self.network_id,
            node_id: self.node_id,
            compiler_version: &self.compiler_version,
            self_: &self.self_,
            projection: &self.projection,
            dialers: &self.dialers,
            egress: &self.egress,
            exit_role: &self.exit_role,
            dns: &self.dns,
        })
    }

    /// Serialized size, checked against `MAX_NETMAP_BYTES` before distribution.
    pub fn encoded_len(&self) -> Result<usize, SerializationError> {
        struct Counter(usize);
        impl std::io::Write for Counter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 += bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut counter = Counter(0);
        serde_json::to_writer(&mut counter, self).map_err(|_| SerializationError)?;
        Ok(counter.0)
    }
}

impl EgressPlan {
    /// The unique partition prefix containing `address`.
    pub fn route(&self, address: std::net::IpAddr) -> Option<&EgressRoute> {
        self.routes.iter().find(|route| {
            route
                .prefix
                .parse::<ipnet::IpNet>()
                .is_ok_and(|net| net.contains(&address))
        })
    }
}

impl CompiledNetmap {
    /// Recompute and compare the content digest; required before saving or installing.
    pub fn verify(&self) -> Result<(), SerializationError> {
        if self.netmap.content_digest()? != self.digest
            || self.netmap.compiler_version != NETMAP_COMPILER_VERSION
        {
            return Err(SerializationError);
        }
        Ok(())
    }

    pub fn reference(&self) -> NetmapRef {
        NetmapRef {
            revision: self.netmap.revision,
            serial: self.netmap.serial,
            digest: self.digest,
        }
    }
}
