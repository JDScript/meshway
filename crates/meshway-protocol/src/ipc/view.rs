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
    /// No channel or circuit now, but a new circuit would relay through a Node this one has a channel with and that reports a channel onward.
    RelayIdle,
    /// Adjacent, but nothing needs a channel now.
    Idle,
    /// Something needs the peer, but no channel, circuit or relay reaches it.
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
/// Size limits of one direct Native UDP channel. Outer datagrams carry DF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct PeerMtu {
    /// Outer path MTU in IP bytes, as this Node's route toward the channel's remote knows it.
    pub path: u16,
    /// Largest inner IP packet one Data frame carries within it: path minus outer IP/UDP and the 86-byte frame overhead.
    pub inner: u16,
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
    /// Size limits of the selected direct Native UDP channel; null for
    /// another transport, without a direct channel, or while its route MTU is unknown.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub mtu: Option<PeerMtu> [required = true],
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
/// Whether observers at different IP addresses saw the same mapping of one socket.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum EndpointMapping {
    /// Observers at two or more IP addresses saw the same mapped address.
    Independent,
    /// Observers at different IP addresses saw different mapped addresses
    /// (a symmetric NAT): a peer reaches this socket from another port than any STUN observer saw.
    Dependent,
    /// Fewer than two observers at different IP addresses answered.
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
/// One socket of this Node's listeners and dialers in the installed netmap:
/// a bound Native UDP socket with the buffer sizes the kernel granted it, or
/// a listener whose bind is being retried.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct SocketStatus {
    pub resource: ResourceRef,
    /// Address the socket is bound to; null while unavailable.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub bound_address: Option<SocketAddress> [required = true],
    /// Effective receive buffer in bytes as the kernel reports it (`SO_RCVBUF`; Linux reports twice the granted size, covering bookkeeping); null while unavailable.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub receive_buffer: Option<u32> [required = true],
    /// Bytes requested: `socket.receiveBuffer`, or the default when 0; null while unavailable.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub receive_requested: Option<u32> [required = true],
    /// Effective send buffer in bytes as the kernel reports it (`SO_SNDBUF`); null while unavailable.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub send_buffer: Option<u32> [required = true],
    /// Bytes requested: `socket.sendBuffer`, or the default when 0; null while unavailable.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub send_requested: Option<u32> [required = true],
    /// The listener is not bound and its bind is being retried: the last failure, e.g. `listener lab-udp cannot bind 0.0.0.0:35000: Address in use (os error 98)`; null when bound.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub unavailable: Option<String> [required = true],
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
/// One DF-marked Binding request as large as the route toward the observer allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct NetcheckSizeProbe {
    /// IP bytes of the probe datagram.
    pub size: u16,
    /// The observer answered: the path carried `size` bytes unfragmented.
    /// False is not a measurement: an observer may also drop large requests.
    pub answered: bool,
} }

meshway_model::object! {
/// The fastest fresh answer of one STUN observer across sockets.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct NetcheckObserver {
    pub observer: SocketAddress,
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub rtt_us: Option<Sequence> [required = true],
    /// This netcheck's size probe toward the observer; null when none was sent.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub size_probe: Option<NetcheckSizeProbe> [required = true],
    /// Route MTU toward the observer after the size probe: a smaller path
    /// MTU the probe's ICMP taught the route, else the interface MTU; null when unknown.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub path_mtu: Option<u16> [required = true],
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
