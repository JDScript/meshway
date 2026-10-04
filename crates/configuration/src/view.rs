use crate::{
    object::{object, unique_map},
    *,
};
use meshway_model::NodeId;
use meshway_transport_spec::{
    DialerSet, DialerStream, DialerTls, Endpoint, Listener, NativeTcpDialer, NativeUdpDialer,
    PositiveDuration, SocketOptions, TcpOptions, TlsMode, TlsVersion, TransportProtocol,
    TrojanDialer, VlessStream, WebSocketPath,
};
use std::{collections::BTreeMap, net::IpAddr};

object! { pub struct ListenerTlsView {
    pub mode: TlsMode, pub server_name: String,
    pub min_version: TlsVersion, pub max_version: TlsVersion,
    pub alpn: Vec<String>, pub handshake_timeout: PositiveDuration,
    pub certificate_present: bool, pub private_key_present: bool,
} }
object! { pub struct ListenerView {
    pub protocol: TransportProtocol, pub enabled: bool, pub remark: String,
    pub listen: IpAddr [value_type=String], pub listen_port: u16,
    pub endpoints: Vec<Endpoint>, pub bind_interface: String, pub socket: SocketOptions,
    pub tcp: Option<TcpOptions>, pub tls: Option<ListenerTlsView>,
    pub credential_present: bool,
    pub stream: Option<VlessStream>,pub flow: Option<String>,pub websocket: Option<ListenerWebSocketView>,
} }
object! {pub struct ListenerWebSocketView {pub path:WebSocketPath,pub host:Option<String>,pub response_header_names:Vec<String>} }
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
        fn tls(t: &meshway_transport_spec::ListenerTls) -> ListenerTlsView {
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
object! {pub struct VlessDialerView {
    pub enabled:bool,pub remark:String,pub priority:u16,pub connect_timeout:PositiveDuration,
    pub bind_interface:String,pub local_address:String,pub socket:SocketOptions,pub tcp:TcpOptions,
    pub stream:DialerStream,pub tls:DialerTls,pub websocket_header_names:Option<Vec<String>>,
} }
object! {pub struct DialerSetView {
    #[serde(rename="native_udp")]pub native_udp:Option<NativeUdpDialer>,
    #[serde(rename="native_tcp")]pub native_tcp:Option<NativeTcpDialer>,
    pub vless:Option<VlessDialerView>,pub trojan:Option<TrojanDialer>,
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
                priority: v.priority,
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
object! {pub struct DefaultsView{pub access:AccessDefault,pub topology:TopologyDefault,pub dialers:DialerSetView} }
impl From<&Defaults> for DefaultsView {
    fn from(d: &Defaults) -> Self {
        Self {
            access: d.access,
            topology: d.topology,
            dialers: (&d.dialers).into(),
        }
    }
}
object! { pub struct NodeSettingsView {
    pub hostname: String, pub remark: String, pub identity: NodeIdentity, pub enabled: bool,
    pub addresses: NodeAddresses,
    #[serde(deserialize_with="unique_map")] pub listeners: BTreeMap<String,ListenerView>,
    #[serde(default,skip_serializing_if="Input::is_omitted")] pub dialers: Input<DialerSetView> [value_type=DialerSetView],
    pub exit_enabled: bool,
} }
object! { pub struct ConfigurationView {
    pub address_pools: AddressPools, pub defaults: DefaultsView,
    #[serde(deserialize_with="unique_map")] pub groups: BTreeMap<String,Vec<String>>,
    #[serde(deserialize_with="unique_map")] pub tags_owners: BTreeMap<String,Vec<String>>,
    pub access: Vec<AccessRule>,
    #[serde(deserialize_with="unique_map")] pub nodes: BTreeMap<NodeId,NodeSettingsView>,
    pub dns: DnsConfiguration, pub topology: Vec<TopologyRule>, pub routing: RoutingConfiguration,
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
