//! Local projections of the installed netmap and of the Node's own network
//! observations. They report facts only; none grants or extends authority.
use chrono::{DateTime, Utc};
use meshway_configuration::NodeAddresses;
use meshway_model::{NodeId, Sequence, SocketAddress, TransportProtocol, state::ResourceRef};
use serde::Serialize;
use utoipa::ToSchema;

meshway_model::object! {
/// This Node as its installed netmap names it.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct NodeSelf {
    pub hostname: String,
    pub addresses: NodeAddresses,
} }

meshway_model::string_enum! {
/// How this Node reaches a visible peer right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum PeerState {
    /// An authenticated channel to the peer is selected.
    Direct,
    /// An established circuit reaches the peer through other Nodes.
    Relay,
    /// Adjacent, but nothing needs a channel now.
    Idle,
    /// Something needs the peer, but no channel or circuit reaches it.
    Unreachable,
    /// The peer's last reported facts are stale or absent, and nothing reaches it.
    Offline,
}
}

meshway_model::object! {
/// Round trip of the latest successful `ping` measurement toward the peer.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct PeerRtt {
    /// Monotonic microseconds of the measured probe.
    pub duration_us: Sequence,
    /// When the measurement completed.
    pub observed_at: DateTime<Utc>,
} }

meshway_model::object! {
/// One visible peer of the installed netmap.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct PeerStatus {
    pub node_id: NodeId,
    pub hostname: String,
    pub addresses: NodeAddresses,
    pub state: PeerState,
    /// Transport of the selected direct channel, or of the first hop of the
    /// relayed circuit; null otherwise.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub transport: Option<TransportProtocol> [required = true],
    /// Remote address of the selected direct channel; null otherwise.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub endpoint: Option<SocketAddress> [required = true],
    /// Ordered intermediate Nodes of the relayed circuit; empty otherwise.
    pub via: Vec<NodeId>,
    /// Null when no successful measurement toward the peer is known.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub rtt: Option<PeerRtt> [required = true],
} }

meshway_model::string_enum! {
/// Whether a socket's mapped port equals its bound port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum PortMapping {
    /// Every current mapping keeps the bound port.
    Preserved,
    /// A current mapping uses another port.
    Changed,
    /// No current mapping.
    Unknown,
}
}

meshway_model::string_enum! {
/// Whether different observers saw the same mapping of one socket.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum EndpointMapping {
    /// At least two observers saw the same mapped address.
    Independent,
    /// Observers saw different mapped addresses.
    Dependent,
    /// Fewer than two observers answered.
    Unknown,
}
}

meshway_model::object! {
/// The latest validated STUN answer of one socket from one observer.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct StunMapping {
    /// Observer address that answered.
    pub observer: SocketAddress,
    /// Mapped address the observer saw.
    pub mapped: SocketAddress,
    /// Round trip of a transaction answered after one send; null after a retransmission.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub rtt_us: Option<Sequence> [required = true],
    /// Answered during this netcheck rather than by an earlier observation round.
    pub fresh: bool,
} }

meshway_model::object! {
/// One Native UDP socket that observes its mapping.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct NetcheckSocket {
    pub resource: ResourceRef,
    pub bound_address: SocketAddress,
    pub mappings: Vec<StunMapping>,
    pub port_mapping: PortMapping,
    pub endpoint_mapping: EndpointMapping,
} }

meshway_model::object! {
/// The fastest fresh answer of one STUN observer across sockets.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct NetcheckObserver {
    pub observer: SocketAddress,
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub rtt_us: Option<Sequence> [required = true],
} }

meshway_model::string_enum! {
/// Why the network part of a netcheck could not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum NetcheckUnavailable {
    /// No installed netmap with a packet plane (down, control-only or preparing).
    NotRunning,
    /// The network owner did not answer in time.
    Busy,
}
}

meshway_model::object! {
/// A read-only request from this Node's control client to its Server.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct NetcheckServer {
    pub origin: String,
    /// Null when the control client was not available to probe.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub reachable: Option<bool> [required = true],
    /// HTTP round trip after one warm-up request; null when not measured.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub rtt_us: Option<Sequence> [required = true],
} }

meshway_model::object! {
/// One bounded local network diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct NetcheckReport {
    pub observed_at: DateTime<Utc>,
    pub server: NetcheckServer,
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub unavailable: Option<NetcheckUnavailable> [required = true],
    /// A fresh IPv4 STUN mapping was observed.
    pub ipv4: bool,
    /// A fresh IPv6 STUN mapping was observed.
    pub ipv6: bool,
    /// A fresh mapping was observed or an authenticated Native UDP channel is selected.
    pub udp: bool,
    pub sockets: Vec<NetcheckSocket>,
    pub observers: Vec<NetcheckObserver>,
} }
