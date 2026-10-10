//! Form edit inputs retain omitted secrets; document inputs remain full values.
use crate::{object::object, *};
use meshway_model::VlessStream;
use meshway_transport_spec::*;

object! { #[derive(Default)]
/// WebSocket request header edits that retain omitted sensitive values.
pub struct DialerWebSocketUpdate {
    /// Omission retains existing extra headers, `{}` clears them, and a nonempty map replaces them without overriding handshake or framing headers.
    #[serde(default, skip_serializing_if="Input::is_omitted")]
    pub headers: Input<WebSocketHeaders> [value_type=WebSocketHeaders],
} }
object! {
/// Replacement VLESS dialer options with explicit retention of omitted WebSocket settings.
pub struct VlessDialerUpdate {
    /// Allow active dialing with this protocol; defaults to true.
    #[serde(default="enabled")] pub enabled: bool,
    /// Free-form description; defaults to an empty string.
    #[serde(default)] pub remark: String,
    /// Positive total resolution, connection and handshake budget; defaults to `5s` and remains bounded by the execution lease.
    #[serde(default="timeout")] pub connect_timeout: PositiveDuration,
    /// Requested underlay interface; defaults to an empty string for platform selection.
    #[serde(default)] pub bind_interface: String,
    /// Local outbound IP address; defaults to an empty string for automatic selection and cannot bypass Mesh address protection.
    #[serde(default)] pub local_address: String,
    /// Socket buffer requests in bytes; omitted settings use zero for provider defaults.
    #[serde(default)] pub socket: SocketOptions,
    /// TCP no-delay and keepalive settings; omission uses the shared TCP defaults.
    #[serde(default)] pub tcp: TcpOptions,
    /// Encapsulation filter, defaulting to `auto` to use the selected listener's stream.
    #[serde(default)] pub stream: DialerStream,
    /// TLS verification and negotiation settings; defaults to managed trust, TLS 1.2–1.3 and a `5s` handshake budget.
    #[serde(default)] pub tls: DialerTls,
    /// Omission retains previous WebSocket options, null removes them, and a value updates them; incompatible with explicit `tcp`.
    #[serde(default, skip_serializing_if="Input::is_omitted")]
    pub websocket: Input<Option<DialerWebSocketUpdate>> [value_type=Option<DialerWebSocketUpdate>],
} }
fn enabled() -> bool {
    true
}
fn timeout() -> PositiveDuration {
    VlessDialer::default().connect_timeout
}

object! { #[derive(Default)]
/// Replacement outbound protocol set; omitted protocol entries are removed and `{}` disables active dialing.
pub struct DialerSetUpdate {
    /// Native UDP options to include; omission excludes this protocol from the replacement set.
    #[serde(default, skip_serializing_if="Input::is_omitted", rename="native_udp")]
    pub native_udp: Input<NativeUdpDialer> [value_type=NativeUdpDialer],
    /// Native TCP options to include; omission excludes this protocol from the replacement set.
    #[serde(default, skip_serializing_if="Input::is_omitted", rename="native_tcp")]
    pub native_tcp: Input<NativeTcpDialer> [value_type=NativeTcpDialer],
    /// VLESS options to include, retaining omitted WebSocket secrets within this entry; omission excludes VLESS.
    #[serde(default, skip_serializing_if="Input::is_omitted")]
    pub vless: Input<VlessDialerUpdate> [value_type=VlessDialerUpdate],
    /// Trojan options to include; omission excludes this protocol from the replacement set.
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
object! {
/// Network default edits that preserve every omitted baseline field.
pub struct DefaultsUpdate {
    /// Unmatched Mesh traffic behavior; omission retains the current value and the only explicit value is `deny`.
    #[serde(default, skip_serializing_if="Input::is_omitted")]
    pub access: Input<AccessDefault> [value_type=AccessDefault],
    /// Unmatched adjacency permission; omission retains the current value.
    #[serde(default, skip_serializing_if="Input::is_omitted")]
    pub topology: Input<TopologyDefault> [value_type=TopologyDefault],
    /// Replacement default protocol set; omission retains it and `{}` disables network-default active dialing.
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
object! {
/// Replacement WebSocket listener options with retention of omitted response headers.
pub struct ListenerWebSocketUpdate {
    /// Exact request path beginning with `/`, without query, fragment or whitespace; defaults to `/meshway`.
    #[serde(default = "ws_path")] pub path: WebSocketPath,
    /// Explicit HTTP Host; omission uses the TLS server name and null is rejected.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] pub host: Option<String> [value_type = String],
    /// Omission retains extra response headers, `{}` clears them, and a nonempty map replaces them without overriding handshake or framing headers.
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
object! {
/// VLESS listener replacement with retention of omitted materials within the same resource.
pub struct VlessListenerUpdate {
    /// Request creation of the fixed listener; defaults to true.
    #[serde(default = "yes")] pub enabled: bool,
    /// Free-form description; defaults to an empty string.
    #[serde(default)] pub remark: String,
    /// Local IPv4 or IPv6 bind address; defaults to `0.0.0.0`.
    #[serde(default = "listen")] pub listen: std::net::IpAddr [value_type = String],
    /// Required local listening port in 1–65535.
    pub listen_port: Port,
    /// Explicitly advertised external host/port pairs; defaults to an empty list.
    #[serde(default)] pub endpoints: Vec<Endpoint>,
    /// Requested underlay interface; defaults to an empty string for platform selection.
    #[serde(default)] pub bind_interface: String,
    /// Socket buffer requests in bytes; omitted settings use zero for provider defaults.
    #[serde(default)] pub socket: SocketOptions,
    /// TCP no-delay and keepalive settings; omission uses the shared TCP defaults.
    #[serde(default)] pub tcp: TcpOptions,
    /// VLESS encapsulation, `tcp` or `websocket`; defaults to `tcp` and both use TLS over TCP.
    #[serde(default = "tcp_stream")] pub stream: VlessStream,
    /// Protocol UUID; omission retains this resource's saved UUID or generates one initially, while null is rejected.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] pub uuid: Option<SecretString> [value_type = SecretString],
    /// VLESS flow; only the default empty string is supported.
    #[serde(default)] pub flow: String,
    /// TLS declaration; omitted materials are retained for the same mode, and managed certificates are regenerated when the server name changes.
    pub tls: ListenerTls,
    /// Omission retains this resource's WebSocket options, null removes them, and a value updates them; only allowed with `websocket` stream.
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
    /// Configure a Native UDP listener without TCP, TLS or proxy credentials.
    NativeUdp(NativeUdpListener),
    /// Configure a Native TCP listener using Meshway authentication and encryption.
    NativeTcp(NativeTcpListener),
    /// Configure VLESS over TCP/TLS or WebSocket/TLS.
    Vless(VlessListenerUpdate),
    /// Configure Trojan TCP CONNECT over TLS.
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
        Ok(
            match Fields::deserialize(meshway_model::wire::MapOnly(d))? {
                Fields::NativeUdp(v) => Self::NativeUdp(v),
                Fields::NativeTcp(v) => Self::NativeTcp(v),
                Fields::Vless(v) => Self::Vless(v),
                Fields::Trojan(v) => Self::Trojan(v),
            },
        )
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
        let mut union = OneOfBuilder::new().description(Some(
            "A listener edit keyed by the listener's name: `type` selects the carrier; omitted \
             VLESS secrets, TLS materials, WebSocket block and response headers keep the \
             saved values of the same listener.",
        ));
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
