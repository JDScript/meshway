//! Pure declared fields. Acceptance here is not a compiled/runtime capability.
use crate::spec::{
    Port, PositiveDuration, SecretString, TlsVersion, WebSocketHeaders, WebSocketPath,
    object::object,
};
use meshway_model::VlessStream;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr};
use utoipa::ToSchema;

pub(crate) fn yes() -> bool {
    true
}
pub(crate) fn five_seconds() -> PositiveDuration {
    "5s".parse().expect("constant duration")
}
fn idle() -> PositiveDuration {
    "5m".parse().expect("constant duration")
}
fn interval() -> PositiveDuration {
    "75s".parse().expect("constant duration")
}
fn count() -> u32 {
    3
}
fn listen() -> IpAddr {
    Ipv4Addr::UNSPECIFIED.into()
}
fn min_tls() -> TlsVersion {
    TlsVersion::Tls12
}
fn max_tls() -> TlsVersion {
    TlsVersion::Tls13
}
fn ws_path() -> WebSocketPath {
    "/meshway".parse().expect("constant path")
}
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

object! {
/// Requested socket buffer sizes, subject to provider defaults and platform resource limits.
#[derive(Default)] pub struct SocketOptions {
    /// Receive buffer size in bytes; omitted or zero uses the provider default.
    #[serde(default)] pub receive_buffer: u32,
    /// Send buffer size in bytes; omitted or zero uses the provider default, and the kernel may limit the requested size.
    #[serde(default)] pub send_buffer: u32,
} }
object! {
/// Operating-system TCP keepalive settings, independent of Meshway authorization and lease checks.
pub struct TcpKeepAlive {
    /// Whether to enable operating-system TCP keepalive; defaults to true.
    #[serde(default = "yes")] pub enabled: bool,
    /// Positive idle duration before the first keepalive probe; defaults to `5m`.
    #[serde(default = "idle")] pub idle: PositiveDuration,
    /// Positive duration between keepalive probes; defaults to `75s`.
    #[serde(default = "interval")] pub interval: PositiveDuration,
    /// Number of keepalive probes; must be greater than zero and defaults to 3.
    #[serde(default = "count")] pub count: u32,
} }
impl Default for TcpKeepAlive {
    fn default() -> Self {
        Self {
            enabled: true,
            idle: idle(),
            interval: interval(),
            count: count(),
        }
    }
}
object! {
/// Socket settings shared by carriers with an outer TCP connection.
pub struct TcpOptions {
    /// Whether to enable TCP_NODELAY; defaults to true.
    #[serde(default = "yes")] pub no_delay: bool,
    /// Operating-system keepalive settings; omission enables probes after `5m`, every `75s`, with a count of 3.
    #[serde(default)] pub keep_alive: TcpKeepAlive,
} }
impl Default for TcpOptions {
    fn default() -> Self {
        Self {
            no_delay: true,
            keep_alive: TcpKeepAlive::default(),
        }
    }
}
object! {
/// A host and port identifying a transport endpoint, without asserting reachability.
pub struct Endpoint {
    /// IP address or DNS name, without a URL scheme, path, or port suffix.
    pub host: String,
    /// Endpoint port in the range 1–65535.
    pub port: Port
} }

/// Source of listener TLS certificate and private-key material; defaults to `managed`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum TlsMode {
    /// Use explicit material when supplied; otherwise the Server generates or retains saved material.
    #[default]
    Managed,
    /// Require administrator-supplied certificate and matching private key.
    Provided,
}
impl TryFrom<String> for TlsMode {
    type Error = &'static str;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        match v.as_str() {
            "managed" => Ok(Self::Managed),
            "provided" => Ok(Self::Provided),
            _ => Err("unsupported TLS mode"),
        }
    }
}
/// TLS trust selection for a dialer; only `managed` is accepted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum TlsTrust {
    /// Verify against the Server's target-bound certificate material, trust roots, and fingerprint.
    #[default]
    Managed,
}
impl TryFrom<String> for TlsTrust {
    type Error = &'static str;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        if v == "managed" {
            Ok(Self::Managed)
        } else {
            Err("unsupported TLS trust")
        }
    }
}
object! {
/// TLS server parameters and certificate material for a VLESS or Trojan listener.
pub struct ListenerTls {
    /// Certificate-material source; defaults to `managed`, while `provided` requires both certificate and private key.
    #[serde(default)] pub mode: TlsMode,
    /// Required TLS host name or IP address used for certificate identity and the dialer's verification name.
    pub server_name: String,
    /// Nonempty PEM certificate chain, leaf first; required in provided mode, otherwise omission generates or retains saved material and null is rejected.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] pub certificate: Option<SecretString> [value_type = SecretString],
    /// Nonempty matching PEM private key; required in provided mode, otherwise omission generates or retains saved material and null is rejected.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] pub private_key: Option<SecretString> [value_type = SecretString],
    /// Minimum accepted TLS version, `1.2` or `1.3`; defaults to `1.2` and must not exceed maxVersion.
    #[serde(default = "min_tls")] pub min_version: TlsVersion,
    /// Maximum accepted TLS version, `1.2` or `1.3`; defaults to `1.3` and must not precede minVersion.
    #[serde(default = "max_tls")] pub max_version: TlsVersion,
    /// Ordered ALPN names of 1–255 bytes without control characters; defaults to empty, and WebSocket permits only `http/1.1` entries.
    #[serde(default)] pub alpn: Vec<String>,
    /// Positive TLS handshake budget; defaults to `5s` and cannot extend the overall connection or remaining lease budget.
    #[serde(default = "five_seconds")] pub handshake_timeout: PositiveDuration,
} }
object! {
/// TLS client verification and negotiation options for a target selected by the Server.
pub struct DialerTls {
    /// Target-bound TLS verification policy; defaults to `managed`, the only accepted value.
    #[serde(default)] pub trust: TlsTrust,
    /// Minimum accepted TLS version, `1.2` or `1.3`; defaults to `1.2` and must not exceed maxVersion.
    #[serde(default = "min_tls")] pub min_version: TlsVersion,
    /// Maximum accepted TLS version, `1.2` or `1.3`; defaults to `1.3` and must not precede minVersion.
    #[serde(default = "max_tls")] pub max_version: TlsVersion,
    /// Ordered ALPN names of 1–255 bytes without control characters; defaults to empty, and WebSocket permits only `http/1.1` entries.
    #[serde(default)] pub alpn: Vec<String>,
    /// Positive TLS handshake budget; defaults to `5s` and cannot extend the overall connection or remaining lease budget.
    #[serde(default = "five_seconds")] pub handshake_timeout: PositiveDuration,
} }
impl Default for DialerTls {
    fn default() -> Self {
        Self {
            trust: TlsTrust::Managed,
            min_version: min_tls(),
            max_version: max_tls(),
            alpn: vec![],
            handshake_timeout: five_seconds(),
        }
    }
}
object! {
/// HTTP/1.1 WebSocket Upgrade parameters supplied by the target VLESS listener.
pub struct ListenerWebSocket {
    /// Exact path beginning with `/`, without query, fragment, whitespace, or control characters; defaults to `/meshway`.
    #[serde(default = "ws_path")] pub path: WebSocketPath,
    /// Exact WebSocket Host name or IP address; omission uses TLS serverName, while null is rejected.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] pub host: Option<String> [value_type = String],
    /// Additional response headers, excluding reserved handshake and framing names; defaults to an empty object.
    #[serde(default)] pub response_headers: WebSocketHeaders,
} }
impl Default for ListenerWebSocket {
    fn default() -> Self {
        Self {
            path: ws_path(),
            host: None,
            response_headers: WebSocketHeaders::default(),
        }
    }
}
object! {
/// Additional WebSocket client options; the target listener supplies the path and Host.
#[derive(Default)] pub struct DialerWebSocket {
    /// Additional request headers, excluding reserved handshake and framing names; defaults to an empty object.
    #[serde(default)] pub headers: WebSocketHeaders
} }

macro_rules! listener {
    ($name:ident { $($(#[$attr:meta])* $field:ident: $ty:ty $([$($schema:tt)*])?),* $(,)? }) => {
        object! {
        /// Declared inbound carrier settings for a fixed listening resource, with protocol-specific parameters.
        pub struct $name {
            /// Whether to create this listening resource; defaults to true, while false retains configuration without executing it.
            #[serde(default = "yes")] pub enabled: bool,
            /// Descriptive text; defaults to an empty string and cannot be null.
            #[serde(default)] pub remark: String,
            /// Local IPv4 or IPv6 bind address; defaults to `0.0.0.0` and cannot be a DNS name.
            #[serde(default = "listen")] pub listen: IpAddr [value_type = String],
            /// Required local listening port in the range 1–65535.
            pub listen_port: Port,
            /// Explicitly advertised external addresses, which may differ from the bind address; omitted or empty advertises none.
            #[serde(default)] pub endpoints: Vec<Endpoint>,
            /// Interface in the current controlled underlay; omitted or empty lets the platform choose.
            #[serde(default)] pub bind_interface: String,
            /// Socket buffer requests; omission uses provider defaults for both receive and send buffers.
            #[serde(default)] pub socket: SocketOptions,
            $($(#[$attr])* pub $field: $ty $([$($schema)*])?),*
        } }
    }
}
listener!(NativeUdpListener {});
listener!(NativeTcpListener {
    /// Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings.
    #[serde(default)]
    tcp: TcpOptions
});
fn tcp_stream() -> VlessStream {
    VlessStream::Tcp
}
listener!(VlessListener {
    /// Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings.
    #[serde(default)] tcp: TcpOptions,
    /// VLESS encapsulation, `tcp` or `websocket`, both over TLS; defaults to `tcp`.
    #[serde(default = "tcp_stream")] stream: VlessStream,
    /// Hyphenated UUID credential with case-insensitive hexadecimal digits; omission makes the Server generate or retain saved material, while null is rejected.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] uuid: Option<SecretString> [value_type = SecretString],
    /// VLESS flow selection; only the empty string is accepted and omission defaults to it.
    #[serde(default)] flow: String,
    /// Required TLS server settings, including the certificate verification name and material source.
    tls: ListenerTls,
    /// WebSocket Upgrade settings; omission uses defaults for a websocket stream, while a tcp stream forbids this field and null is rejected.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] websocket: Option<ListenerWebSocket> [value_type = ListenerWebSocket],
});
/// Trojan stream encapsulation; only `tcp` is accepted and is the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum TrojanStream {
    /// Trojan TCP CONNECT over TLS.
    #[default]
    Tcp,
}
impl TryFrom<String> for TrojanStream {
    type Error = &'static str;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        if v == "tcp" {
            Ok(Self::Tcp)
        } else {
            Err("unsupported Trojan stream")
        }
    }
}
listener!(TrojanListener {
    /// Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings.
    #[serde(default)] tcp: TcpOptions,
    /// Trojan encapsulation; only `tcp` is accepted and omission defaults to it.
    #[serde(default)] stream: TrojanStream,
    /// Nonempty password preserved without trimming or case normalization; omission makes the Server generate or retain saved material, while null is rejected.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] password: Option<SecretString> [value_type = SecretString],
    /// Required TLS server settings, including the certificate verification name and material source.
    tls: ListenerTls,
});
/// Inbound listener declaration selected by `type`, with optional material completed by the Server when saved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Listener {
    /// Native UDP listener using Meshway authentication and encryption.
    NativeUdp(NativeUdpListener),
    /// Native TCP listener using Meshway authentication and encryption.
    NativeTcp(NativeTcpListener),
    /// VLESS TCP CONNECT listener over TCP/TLS or WebSocket/TLS.
    Vless(VlessListener),
    /// Trojan TCP CONNECT listener over TLS.
    Trojan(TrojanListener),
}
impl utoipa::PartialSchema for Listener {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        listener_schema(false)
    }
}
impl ToSchema for Listener {
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
            (VlessListener::name().into_owned(), VlessListener::schema()),
            (
                TrojanListener::name().into_owned(),
                TrojanListener::schema(),
            ),
        ]);
        NativeUdpListener::schemas(schemas);
        NativeTcpListener::schemas(schemas);
        VlessListener::schemas(schemas);
        TrojanListener::schemas(schemas);
    }
}

// Serde flattens a tagged newtype variant. allOf cannot add a discriminator to
// its closed payload object, so build each closed variant from that same payload.
pub(crate) fn listener_schema(
    prepared: bool,
) -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
    use utoipa::{
        PartialSchema,
        openapi::{
            RefOr,
            schema::{ObjectBuilder, OneOfBuilder, Schema, Type},
        },
    };
    let mut union = OneOfBuilder::new().description(Some(if prepared {
        "An inbound listener as saved: `type` selects the carrier, and credential \
         and TLS materials are complete. Returned by configuration reads."
    } else {
        "An inbound listener as declared: `type` selects the carrier; credential \
         and TLS materials may be omitted for the Server to generate."
    }));
    for (tag, schema, credential) in [
        ("native_udp", NativeUdpListener::schema(), None),
        ("native_tcp", NativeTcpListener::schema(), None),
        ("vless", VlessListener::schema(), Some("uuid")),
        ("trojan", TrojanListener::schema(), Some("password")),
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
        if let Some(credential) = credential.filter(|_| prepared) {
            object.required.push(credential.into());
            let RefOr::T(Schema::Object(mut tls)) = ListenerTls::schema() else {
                unreachable!("listener TLS is an object")
            };
            tls.required
                .extend(["certificate".into(), "privateKey".into()]);
            object.properties.insert("tls".into(), tls.into());
        }
        union = union.item(object);
    }
    union.into()
}
impl<'de> Deserialize<'de> for Listener {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
        enum Fields {
            NativeUdp(NativeUdpListener),
            NativeTcp(NativeTcpListener),
            Vless(VlessListener),
            Trojan(TrojanListener),
        }
        Ok(
            match Fields::deserialize(crate::spec::object::MapOnly(d))? {
                Fields::NativeUdp(v) => Self::NativeUdp(v),
                Fields::NativeTcp(v) => Self::NativeTcp(v),
                Fields::Vless(v) => Self::Vless(v),
                Fields::Trojan(v) => Self::Trojan(v),
            },
        )
    }
}

macro_rules! dialer {
    ($name:ident, { $($(#[$attr:meta])* $field:ident: $ty:ty $([$($schema:tt)*])?),* $(,)? }) => {
        object! {
        /// Outbound carrier selection and connection options; the Server resolves target listener parameters separately.
        pub struct $name {
            /// Whether this dialer may be used for outgoing connections; defaults to true.
            #[serde(default = "yes")] pub enabled: bool,
            /// Descriptive text rather than a resource key; defaults to an empty string.
            #[serde(default)] pub remark: String,
            /// Positive total connection budget including resolution and required handshakes; defaults to `5s` and is limited by the remaining active lease.
            #[serde(default = "five_seconds")] pub connect_timeout: PositiveDuration,
            /// Interface in the current controlled underlay; omitted or empty lets the platform choose.
            #[serde(default)] pub bind_interface: String,
            /// Local outgoing IP address; omitted or empty selects automatically, and a Mesh address cannot bypass underlay protection.
            #[serde(default)] pub local_address: String,
            /// Socket buffer requests; omission uses provider defaults for both receive and send buffers.
            #[serde(default)] pub socket: SocketOptions,
            $($(#[$attr])* pub $field: $ty $([$($schema)*])?),*
        } }
        impl Default for $name { fn default() -> Self { Self {
            enabled: true, remark: String::new(),
            connect_timeout: five_seconds(), bind_interface: String::new(),
            local_address: String::new(), socket: SocketOptions::default(),
            $($field: Default::default()),*
        } } }
    }
}
dialer!(NativeUdpDialer, {});
dialer!(NativeTcpDialer, {
    /// Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings.
    #[serde(default)] tcp: TcpOptions
});
/// VLESS target encapsulation selector; defaults to `auto` to use the listener's encapsulation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum DialerStream {
    /// Accept the target listener's TCP/TLS or WebSocket/TLS encapsulation.
    #[default]
    Auto,
    /// Select only TCP/TLS listeners.
    Tcp,
    /// Select only WebSocket/TLS listeners.
    #[serde(rename = "websocket")]
    WebSocket,
}
impl TryFrom<String> for DialerStream {
    type Error = &'static str;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        match v.as_str() {
            "auto" => Ok(Self::Auto),
            "tcp" => Ok(Self::Tcp),
            "websocket" => Ok(Self::WebSocket),
            _ => Err("unsupported VLESS stream"),
        }
    }
}
dialer!(VlessDialer, {
    /// Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings.
    #[serde(default)] tcp: TcpOptions,
    /// Encapsulation selector, `auto`, `tcp`, or `websocket`; defaults to `auto`, while explicit values filter compatible listeners.
    #[serde(default)] stream: DialerStream,
    /// TLS client options; omission uses managed trust, TLS 1.2–1.3, empty ALPN, and a `5s` handshake budget.
    #[serde(default)] tls: DialerTls,
    /// Extra WebSocket client options; omission adds no request headers, tcp selection forbids this field, and null is rejected.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] websocket: Option<DialerWebSocket> [value_type = DialerWebSocket],
});
dialer!(TrojanDialer, {
    /// Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings.
    #[serde(default)] tcp: TcpOptions,
    /// TLS client options; omission uses managed trust, TLS 1.2–1.3, empty ALPN, and a `5s` handshake budget.
    #[serde(default)] tls: DialerTls
});
object! {
/// Protocol-keyed outbound selection; an explicit empty object disables active dialing, and a node's explicit set replaces the whole network default set.
#[derive(Default)] pub struct DialerSet {
    /// Native UDP options; omission excludes this protocol from the selected set, an empty object enables defaults, and null is rejected.
    #[serde(default, deserialize_with = "present", rename = "native_udp", skip_serializing_if = "Option::is_none")] pub native_udp: Option<NativeUdpDialer> [value_type = NativeUdpDialer],
    /// Native TCP options; omission excludes this protocol from the selected set, an empty object enables defaults, and null is rejected.
    #[serde(default, deserialize_with = "present", rename = "native_tcp", skip_serializing_if = "Option::is_none")] pub native_tcp: Option<NativeTcpDialer> [value_type = NativeTcpDialer],
    /// VLESS options; omission excludes this protocol from the selected set, an empty object enables defaults, and null is rejected.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] pub vless: Option<VlessDialer> [value_type = VlessDialer],
    /// Trojan options; omission excludes this protocol from the selected set, an empty object enables defaults, and null is rejected.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] pub trojan: Option<TrojanDialer> [value_type = TrojanDialer],
} }
impl DialerSet {
    pub fn network_default() -> Self {
        Self {
            native_udp: Some(NativeUdpDialer::default()),
            native_tcp: Some(NativeTcpDialer::default()),
            vless: Some(VlessDialer::default()),
            trojan: None,
        }
    }
}
