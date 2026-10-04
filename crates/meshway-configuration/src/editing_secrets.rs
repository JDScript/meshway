//! Form edit inputs retain omitted secrets; document inputs remain full values.
use crate::{object::object, *};
use meshway_transport_spec::*;

object! { #[derive(Default)] pub struct DialerWebSocketUpdate {
    #[serde(default, skip_serializing_if="Input::is_omitted")]
    pub headers: Input<WebSocketHeaders> [value_type=WebSocketHeaders],
} }
object! { pub struct VlessDialerUpdate {
    #[serde(default="enabled")] pub enabled: bool,
    #[serde(default)] pub remark: String,
    #[serde(default="priority")] pub priority: u16,
    #[serde(default="timeout")] pub connect_timeout: PositiveDuration,
    #[serde(default)] pub bind_interface: String,
    #[serde(default)] pub local_address: String,
    #[serde(default)] pub socket: SocketOptions,
    #[serde(default)] pub tcp: TcpOptions,
    #[serde(default)] pub stream: DialerStream,
    #[serde(default)] pub tls: DialerTls,
    #[serde(default, skip_serializing_if="Input::is_omitted")]
    pub websocket: Input<Option<DialerWebSocketUpdate>> [value_type=Option<DialerWebSocketUpdate>],
} }
fn enabled() -> bool {
    true
}
fn priority() -> u16 {
    VlessDialer::default().priority
}
fn timeout() -> PositiveDuration {
    VlessDialer::default().connect_timeout
}

object! { #[derive(Default)] pub struct DialerSetUpdate {
    #[serde(default, skip_serializing_if="Input::is_omitted", rename="native_udp")]
    pub native_udp: Input<NativeUdpDialer> [value_type=NativeUdpDialer],
    #[serde(default, skip_serializing_if="Input::is_omitted", rename="native_tcp")]
    pub native_tcp: Input<NativeTcpDialer> [value_type=NativeTcpDialer],
    #[serde(default, skip_serializing_if="Input::is_omitted")]
    pub vless: Input<VlessDialerUpdate> [value_type=VlessDialerUpdate],
    #[serde(default, skip_serializing_if="Input::is_omitted")]
    pub trojan: Input<TrojanDialer> [value_type=TrojanDialer],
} }
impl DialerSetUpdate {
    pub fn apply(&self, previous: &DialerSet) -> DialerSet {
        fn value<T: Clone>(v: &Input<T>) -> Option<T> {
            match v {
                Input::Omitted => None,
                Input::Value(v) => Some(v.clone()),
            }
        }
        DialerSet {
            native_udp: value(&self.native_udp),
            native_tcp: value(&self.native_tcp),
            trojan: value(&self.trojan),
            vless: match &self.vless {
                Input::Omitted => None,
                Input::Value(v) => {
                    let old = previous.vless.as_ref().and_then(|v| v.websocket.as_ref());
                    let websocket = match &v.websocket {
                        Input::Omitted => old.cloned(),
                        Input::Value(None) => None,
                        Input::Value(Some(ws)) => Some(DialerWebSocket {
                            headers: match &ws.headers {
                                Input::Omitted => {
                                    old.map(|w| w.headers.clone()).unwrap_or_default()
                                }
                                Input::Value(headers) => headers.clone(),
                            },
                        }),
                    };
                    Some(VlessDialer {
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
                        websocket,
                    })
                }
            },
        }
    }
}
object! { pub struct DefaultsUpdate {
    #[serde(default, skip_serializing_if="Input::is_omitted")]
    pub access: Input<AccessDefault> [value_type=AccessDefault],
    #[serde(default, skip_serializing_if="Input::is_omitted")]
    pub topology: Input<TopologyDefault> [value_type=TopologyDefault],
    #[serde(default, skip_serializing_if="Input::is_omitted")]
    pub dialers: Input<DialerSetUpdate> [value_type=DialerSetUpdate],
} }
impl DefaultsUpdate {
    pub fn apply(&self, previous: &Defaults) -> Defaults {
        Defaults {
            access: match self.access {
                Input::Omitted => previous.access,
                Input::Value(v) => v,
            },
            topology: match self.topology {
                Input::Omitted => previous.topology,
                Input::Value(v) => v,
            },
            dialers: match &self.dialers {
                Input::Omitted => previous.dialers.clone(),
                Input::Value(v) => v.apply(&previous.dialers),
            },
        }
    }
}

fn present<'de, D: serde::Deserializer<'de>, T: serde::Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}
fn ws_path() -> WebSocketPath {
    ListenerWebSocket::default().path
}
fn tcp_stream() -> VlessStream {
    VlessStream::Tcp
}
fn listen() -> std::net::IpAddr {
    std::net::Ipv4Addr::UNSPECIFIED.into()
}

// WebSocket settings of a listener update: omitted response headers keep the
// previous value, like dialer headers do.
object! { pub struct ListenerWebSocketUpdate {
    #[serde(default = "ws_path")] pub path: WebSocketPath,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] pub host: Option<String> [value_type = String],
    #[serde(default, skip_serializing_if = "Input::is_omitted")]
    pub response_headers: Input<WebSocketHeaders> [value_type = WebSocketHeaders],
} }
impl ListenerWebSocketUpdate {
    fn apply(&self, previous: Option<&ListenerWebSocket>) -> ListenerWebSocket {
        ListenerWebSocket {
            path: self.path.clone(),
            host: self.host.clone(),
            response_headers: match &self.response_headers {
                Input::Value(headers) => headers.clone(),
                Input::Omitted => previous
                    .map(|p| p.response_headers.clone())
                    .unwrap_or_default(),
            },
        }
    }
}
// A VLESS listener as an update: the declaration's fields, with the WebSocket
// block and its response headers inheritable by omission.
object! { pub struct VlessListenerUpdate {
    #[serde(default = "yes")] pub enabled: bool,
    #[serde(default)] pub remark: String,
    #[serde(default = "listen")] pub listen: std::net::IpAddr [value_type = String],
    pub listen_port: Port,
    #[serde(default)] pub endpoints: Vec<Endpoint>,
    #[serde(default)] pub bind_interface: String,
    #[serde(default)] pub socket: SocketOptions,
    #[serde(default)] pub tcp: TcpOptions,
    #[serde(default = "tcp_stream")] pub stream: VlessStream,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] pub uuid: Option<SecretString> [value_type = SecretString],
    #[serde(default)] pub flow: String,
    pub tls: ListenerTls,
    #[serde(default, skip_serializing_if = "Input::is_omitted")]
    pub websocket: Input<Option<ListenerWebSocketUpdate>> [value_type = Option<ListenerWebSocketUpdate>],
} }
fn yes() -> bool {
    true
}
impl VlessListenerUpdate {
    fn apply(&self, previous: Option<&Listener>) -> VlessListener {
        let old = match previous {
            Some(Listener::Vless(v)) => v.websocket.as_ref(),
            _ => None,
        };
        VlessListener {
            enabled: self.enabled,
            remark: self.remark.clone(),
            listen: self.listen,
            listen_port: self.listen_port,
            endpoints: self.endpoints.clone(),
            bind_interface: self.bind_interface.clone(),
            socket: self.socket.clone(),
            tcp: self.tcp.clone(),
            stream: self.stream,
            uuid: self.uuid.clone(),
            flow: self.flow.clone(),
            tls: self.tls.clone(),
            websocket: match &self.websocket {
                Input::Omitted => old.cloned(),
                Input::Value(None) => None,
                Input::Value(Some(update)) => Some(update.apply(old)),
            },
        }
    }
}

/// Same protocol fields as a declaration. Omitted secrets and WebSocket
/// response headers keep their values only while updating the same resource;
/// new resources have no predecessor.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ListenerUpdate {
    NativeUdp(NativeUdpListener),
    NativeTcp(NativeTcpListener),
    Vless(VlessListenerUpdate),
    Trojan(TrojanListener),
}
impl<'de> serde::Deserialize<'de> for ListenerUpdate {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
        enum Fields {
            NativeUdp(NativeUdpListener),
            NativeTcp(NativeTcpListener),
            Vless(VlessListenerUpdate),
            Trojan(TrojanListener),
        }
        Ok(match Fields::deserialize(meshway_serde::MapOnly(d))? {
            Fields::NativeUdp(v) => Self::NativeUdp(v),
            Fields::NativeTcp(v) => Self::NativeTcp(v),
            Fields::Vless(v) => Self::Vless(v),
            Fields::Trojan(v) => Self::Trojan(v),
        })
    }
}
impl utoipa::PartialSchema for ListenerUpdate {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::{
            RefOr,
            schema::{ObjectBuilder, OneOfBuilder, Schema, Type},
        };
        // Serde flattens a tagged newtype variant; build each closed variant
        // object with its own `type` discriminator.
        let mut union = OneOfBuilder::new();
        for (tag, schema) in [
            ("native_udp", NativeUdpListener::schema()),
            ("native_tcp", NativeTcpListener::schema()),
            ("vless", VlessListenerUpdate::schema()),
            ("trojan", TrojanListener::schema()),
        ] {
            let RefOr::T(Schema::Object(mut object)) = schema else {
                unreachable!("listener payload is an object")
            };
            object.properties.insert(
                "type".into(),
                ObjectBuilder::new()
                    .schema_type(Type::String)
                    .enum_values(Some([tag]))
                    .into(),
            );
            object.required.push("type".into());
            union = union.item(object);
        }
        union.into()
    }
}
impl utoipa::ToSchema for ListenerUpdate {
    fn schemas(
        schemas: &mut Vec<(
            String,
            utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
        )>,
    ) {
        use utoipa::PartialSchema;
        schemas.extend([
            (
                NativeUdpListener::name().into_owned(),
                NativeUdpListener::schema(),
            ),
            (
                NativeTcpListener::name().into_owned(),
                NativeTcpListener::schema(),
            ),
            (
                VlessListenerUpdate::name().into_owned(),
                VlessListenerUpdate::schema(),
            ),
            (
                TrojanListener::name().into_owned(),
                TrojanListener::schema(),
            ),
        ]);
        NativeUdpListener::schemas(schemas);
        NativeTcpListener::schemas(schemas);
        VlessListenerUpdate::schemas(schemas);
        TrojanListener::schemas(schemas);
    }
}
impl ListenerUpdate {
    pub fn apply(&self, previous: Option<&Listener>) -> Listener {
        let mut listener = match self {
            Self::NativeUdp(v) => Listener::NativeUdp(v.clone()),
            Self::NativeTcp(v) => Listener::NativeTcp(v.clone()),
            Self::Vless(v) => Listener::Vless(v.apply(previous)),
            Self::Trojan(v) => Listener::Trojan(v.clone()),
        };
        if let Some(previous) = previous {
            inherit_materials(&mut listener, previous);
        }
        listener
    }
}
