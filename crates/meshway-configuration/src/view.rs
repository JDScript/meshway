use crate::{
    object::{object, unique_map},
    *,
};
use meshway_model::NodeId;
use meshway_model::{TransportProtocol, VlessStream};
use meshway_transport::spec::{
    DialerSet, DialerStream, DialerTls, Endpoint, Listener, NativeTcpDialer, NativeUdpDialer,
    PositiveDuration, SocketOptions, TcpOptions, TlsMode, TlsVersion, TrojanDialer, WebSocketPath,
};
use std::{collections::BTreeMap, net::IpAddr};

object! {
/// TLS listener settings with material presence indicators instead of certificate and key contents.
pub struct ListenerTlsView {
    /// Whether certificate materials are managed by the platform or provided by the administrator.
    pub mode: TlsMode,
    /// TLS name used for certificate verification and peer dialing.
    pub server_name: String,
    /// Lowest permitted TLS version, no greater than `maxVersion`.
    pub min_version: TlsVersion,
    /// Highest permitted TLS version, no lower than `minVersion`.
    pub max_version: TlsVersion,
    /// Ordered application protocols offered for negotiation; WebSocket requires HTTP/1.1-compatible values.
    pub alpn: Vec<String>,
    /// Positive TLS handshake budget, also bounded by the remaining connection and execution deadlines.
    pub handshake_timeout: PositiveDuration,
    /// Whether certificate material is present in this declaration; preview generation plans may leave it false.
    pub certificate_present: bool,
    /// Whether private-key material is present in this declaration; preview generation plans may leave it false.
    pub private_key_present: bool,
} }
object! {
/// Read-only listener declaration with credentials and sensitive header values removed.
pub struct ListenerView {
    /// Outer transport protocol configured for this fixed inbound resource.
    pub protocol: TransportProtocol,
    /// Whether the configuration requests creation of this listener, rather than its actual installation state.
    pub enabled: bool,
    /// Free-form description with no authorization meaning.
    pub remark: String,
    /// Local IPv4 or IPv6 bind address, distinct from advertised endpoints.
    pub listen: IpAddr [value_type=String],
    /// Local listening port in 1–65535.
    pub listen_port: u16,
    /// Explicitly advertised external host/port pairs; their presence does not establish reachability.
    pub endpoints: Vec<Endpoint>,
    /// Requested underlay interface; an empty string leaves interface selection to the platform.
    pub bind_interface: String,
    /// Receive and send buffer requests in bytes, with zero selecting provider defaults.
    pub socket: SocketOptions,
    /// TCP socket options; null for UDP listeners.
    pub tcp: Option<TcpOptions>,
    /// Redacted TLS settings for VLESS or Trojan; null for Native transports.
    pub tls: Option<ListenerTlsView>,
    /// Whether a VLESS UUID or Trojan password is present; false for Native transports and unprepared automatic credentials.
    pub credential_present: bool,
    /// VLESS stream encapsulation; null for other protocols.
    pub stream: Option<VlessStream>,
    /// VLESS flow, currently always empty when present; null for other protocols.
    pub flow: Option<String>,
    /// Redacted WebSocket settings when configured for VLESS; null otherwise.
    pub websocket: Option<ListenerWebSocketView>,
} }
object! {
/// Public WebSocket listener settings without response header values.
pub struct ListenerWebSocketView {
    /// Exact request path beginning with `/`, without query, fragment, spaces or control characters.
    pub path:WebSocketPath,
    /// Explicit HTTP Host override; null uses the listener TLS server name.
    pub host:Option<String>,
    /// Names of extra response headers, with all values withheld.
    pub response_header_names:Vec<String>
} }
impl From<&Listener> for ListenerView {
    fn from(listener: &Listener) -> Self {
        macro_rules! common {
            ($v:expr,$protocol:expr,$tcp:expr,$tls:expr,$credential:expr) => {
                Self {
                    protocol: $protocol,
                    enabled: $v.enabled,
                    remark: $v.remark.clone(),
                    listen: $v.listen,
                    listen_port: $v.listen_port.get(),
                    endpoints: $v.endpoints.clone(),
                    bind_interface: $v.bind_interface.clone(),
                    socket: $v.socket.clone(),
                    tcp: $tcp,
                    tls: $tls,
                    credential_present: $credential,
                    stream: None,
                    flow: None,
                    websocket: None,
                }
            };
        }
        fn tls(t: &meshway_transport::spec::ListenerTls) -> ListenerTlsView {
            ListenerTlsView {
                mode: t.mode,
                server_name: t.server_name.clone(),
                min_version: t.min_version,
                max_version: t.max_version,
                alpn: t.alpn.clone(),
                handshake_timeout: t.handshake_timeout.clone(),
                certificate_present: t.certificate.is_some(),
                private_key_present: t.private_key.is_some(),
            }
        }
        match listener {
            Listener::NativeUdp(v) => common!(v, TransportProtocol::NativeUdp, None, None, false),
            Listener::NativeTcp(v) => common!(
                v,
                TransportProtocol::NativeTcp,
                Some(v.tcp.clone()),
                None,
                false
            ),
            Listener::Vless(v) => {
                let mut view = common!(
                    v,
                    TransportProtocol::Vless,
                    Some(v.tcp.clone()),
                    Some(tls(&v.tls)),
                    v.uuid.is_some()
                );
                view.stream = Some(v.stream);
                view.flow = Some(v.flow.clone());
                view.websocket = v.websocket.as_ref().map(|ws| ListenerWebSocketView {
                    path: ws.path.clone(),
                    host: ws.host.clone(),
                    response_header_names: ws
                        .response_headers
                        .iter()
                        .map(|(key, _)| key.to_owned())
                        .collect(),
                });
                view
            }
            Listener::Trojan(v) => common!(
                v,
                TransportProtocol::Trojan,
                Some(v.tcp.clone()),
                Some(tls(&v.tls)),
                v.password.is_some()
            ),
        }
    }
}
object! {
/// Read-only VLESS outbound settings with sensitive WebSocket header values removed.
pub struct VlessDialerView {
    /// Whether this dialer may actively acquire a transport.
    pub enabled:bool,
    /// Free-form description, distinct from the protocol key.
    pub remark:String,
    /// Positive total connection budget covering resolution, transport and protocol handshakes.
    pub connect_timeout:PositiveDuration,
    /// Requested underlay interface; an empty string leaves selection to the platform.
    pub bind_interface:String,
    /// Local outbound IP address; an empty string selects it automatically.
    pub local_address:String,
    /// Receive and send buffer requests in bytes, with zero selecting provider defaults.
    pub socket:SocketOptions,
    /// TCP no-delay and operating-system keepalive settings.
    pub tcp:TcpOptions,
    /// `auto` accepts the target listener's stream, while explicit values restrict compatible listeners.
    pub stream:DialerStream,
    /// TLS client verification and negotiation settings for the selected listener's bound trust materials.
    pub tls:DialerTls,
    /// Names of extra WebSocket request headers; null when no WebSocket options are configured.
    pub websocket_header_names:Option<Vec<String>>,
} }
object! {
/// Selected outbound protocol set with sensitive values removed.
pub struct DialerSetView {
    /// Native UDP options; null means this protocol is absent from the selected set.
    #[serde(rename="native_udp")]pub native_udp:Option<NativeUdpDialer>,
    /// Native TCP options; null means this protocol is absent from the selected set.
    #[serde(rename="native_tcp")]pub native_tcp:Option<NativeTcpDialer>,
    /// Redacted VLESS options; null means this protocol is absent from the selected set.
    pub vless:Option<VlessDialerView>,
    /// Trojan options; null means this protocol is absent from the selected set.
    pub trojan:Option<TrojanDialer>,
} }
impl From<&DialerSet> for DialerSetView {
    fn from(d: &DialerSet) -> Self {
        Self {
            native_udp: d.native_udp.clone(),
            native_tcp: d.native_tcp.clone(),
            trojan: d.trojan.clone(),
            vless: d.vless.as_ref().map(|v| VlessDialerView {
                enabled: v.enabled,
                remark: v.remark.clone(),
                connect_timeout: v.connect_timeout.clone(),
                bind_interface: v.bind_interface.clone(),
                local_address: v.local_address.clone(),
                socket: v.socket.clone(),
                tcp: v.tcp.clone(),
                stream: v.stream,
                tls: v.tls.clone(),
                websocket_header_names: v
                    .websocket
                    .as_ref()
                    .map(|ws| ws.headers.iter().map(|(key, _)| key.to_owned()).collect()),
            }),
        }
    }
}
object! {
/// Public projection of network defaults with sensitive dialer values removed.
pub struct DefaultsView{
    /// Denial applied to Mesh business traffic without an explicit access grant.
    pub access:AccessDefault,
    /// Adjacency permission used when no topology rule matches.
    pub topology:TopologyDefault,
    /// Outbound protocol set inherited by nodes without a dialer override.
    pub dialers:DialerSetView
} }
impl From<&Defaults> for DefaultsView {
    fn from(d: &Defaults) -> Self {
        Self {
            access: d.access,
            topology: d.topology,
            dialers: (&d.dialers).into(),
        }
    }
}
object! {
/// Public desired node settings without listener credentials or sensitive header values.
pub struct NodeSettingsView {
    /// Network-unique DNS label used with the network DNS suffix.
    pub hostname: String,
    /// Free-form description with no authorization meaning.
    pub remark: String,
    /// Effective user or tag identity used for policy matching.
    pub identity: NodeIdentity,
    /// Whether business execution is administratively enabled, independent of current installation or credential revocation.
    pub enabled: bool,
    /// Desired Mesh addresses, with null for an unassigned family.
    pub addresses: NodeAddresses,
    /// Redacted fixed inbound resources keyed by node-local listener name.
    #[serde(deserialize_with="unique_map")] pub listeners: BTreeMap<String,ListenerView>,
    /// Omission inherits network dialers; a present set replaces the default protocol selection.
    #[serde(default,skip_serializing_if="Input::is_omitted")] pub dialers: Input<DialerSetView> [value_type=DialerSetView],
    /// Administrative exit eligibility, independent of actual forwarding readiness.
    pub exit_enabled: bool,
} }
object! {
/// Redacted network configuration projection used for reads and non-executable previews.
pub struct ConfigurationView {
    /// Network-wide usable Mesh address pools.
    pub address_pools: AddressPools,
    /// Unmatched-rule behavior and redacted inherited dialers.
    pub defaults: DefaultsView,
    /// `group:<name>` definitions containing direct network member emails.
    #[serde(deserialize_with="unique_map")] pub groups: BTreeMap<String,Vec<String>>,
    /// `tag:<name>` definitions containing owner emails or `group:<name>` references.
    #[serde(deserialize_with="unique_map")] pub tags_owners: BTreeMap<String,Vec<String>>,
    /// Additive business traffic grants, with no grant meaning denial.
    pub access: Vec<AccessRule>,
    /// Registered node IDs mapped to redacted desired settings.
    #[serde(deserialize_with="unique_map")] pub nodes: BTreeMap<NodeId,NodeSettingsView>,
    /// Network naming and explicit resolver routing.
    pub dns: DnsConfiguration,
    /// Undirected adjacency rules, evaluated independently of business access grants.
    pub topology: Vec<TopologyRule>,
    /// Mesh waypoint requirements and external destination routing.
    pub routing: RoutingConfiguration,
    /// Named destination-set declarations; downloaded content and check status are exposed by resource views.
    #[serde(deserialize_with="unique_map")] pub rule_sets: BTreeMap<String,RuleSet>,
} }
impl From<&NetworkConfiguration> for ConfigurationView {
    fn from(c: &NetworkConfiguration) -> Self {
        Self {
            address_pools: c.address_pools.clone(),
            defaults: (&c.defaults).into(),
            groups: c.groups.clone(),
            tags_owners: c.tags_owners.clone(),
            access: c.access.clone(),
            nodes: c
                .nodes
                .iter()
                .map(|(id, n)| (*id, NodeSettingsView::from(n)))
                .collect(),
            dns: c.dns.clone(),
            topology: c.topology.clone(),
            routing: c.routing.clone(),
            rule_sets: c.rule_sets.clone(),
        }
    }
}

impl From<&NodeSettings> for NodeSettingsView {
    fn from(n: &NodeSettings) -> Self {
        Self {
            hostname: n.hostname.clone(),
            remark: n.remark.clone(),
            identity: n.identity.clone(),
            enabled: n.enabled,
            addresses: n.addresses.clone(),
            listeners: n
                .listeners
                .iter()
                .map(|(key, l)| (key.clone(), l.declaration().into()))
                .collect(),
            dialers: match &n.dialers {
                Input::Omitted => Input::Omitted,
                Input::Value(d) => Input::Value(d.into()),
            },
            exit_enabled: n.exit_enabled,
        }
    }
}
