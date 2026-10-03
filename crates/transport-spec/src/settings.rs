//! Pure declared fields. Acceptance here is not a compiled/runtime capability.
use crate::{
    Port, PositiveDuration, SecretString, TlsVersion, VlessStream, WebSocketHeaders, WebSocketPath,
    object::object,
};
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

object! { #[derive(Default)] pub struct SocketOptions {
    #[serde(default)] pub receive_buffer: u32,
    #[serde(default)] pub send_buffer: u32,
} }
object! { pub struct TcpKeepAlive {
    #[serde(default = "yes")] pub enabled: bool,
    #[serde(default = "idle")] pub idle: PositiveDuration,
    #[serde(default = "interval")] pub interval: PositiveDuration,
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
object! { pub struct TcpOptions {
    #[serde(default = "yes")] pub no_delay: bool,
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
object! { pub struct Endpoint { pub host: String, pub port: Port } }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum TlsMode {
    #[default]
    Managed,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum TlsTrust {
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
object! { pub struct ListenerTls {
    #[serde(default)] pub mode: TlsMode,
    pub server_name: String,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] pub certificate: Option<SecretString> [value_type = SecretString],
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] pub private_key: Option<SecretString> [value_type = SecretString],
    #[serde(default = "min_tls")] pub min_version: TlsVersion,
    #[serde(default = "max_tls")] pub max_version: TlsVersion,
    #[serde(default)] pub alpn: Vec<String>,
    #[serde(default = "five_seconds")] pub handshake_timeout: PositiveDuration,
} }
object! { pub struct DialerTls {
    #[serde(default)] pub trust: TlsTrust,
    #[serde(default = "min_tls")] pub min_version: TlsVersion,
    #[serde(default = "max_tls")] pub max_version: TlsVersion,
    #[serde(default)] pub alpn: Vec<String>,
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
object! { pub struct ListenerWebSocket {
    #[serde(default = "ws_path")] pub path: WebSocketPath,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] pub host: Option<String> [value_type = String],
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
object! { #[derive(Default)] pub struct DialerWebSocket { #[serde(default)] pub headers: WebSocketHeaders } }

macro_rules! listener {
    ($name:ident { $($(#[$attr:meta])* $field:ident: $ty:ty $([$($schema:tt)*])?),* $(,)? }) => {
        object! { pub struct $name {
            #[serde(default = "yes")] pub enabled: bool,
            #[serde(default)] pub remark: String,
            #[serde(default = "listen")] pub listen: IpAddr [value_type = String],
            pub listen_port: Port,
            #[serde(default)] pub endpoints: Vec<Endpoint>,
            #[serde(default)] pub bind_interface: String,
            #[serde(default)] pub socket: SocketOptions,
            $($(#[$attr])* pub $field: $ty $([$($schema)*])?),*
        } }
    }
}
listener!(NativeUdpListener {});
listener!(NativeTcpListener {
    #[serde(default)]
    tcp: TcpOptions
});
fn tcp_stream() -> VlessStream {
    VlessStream::Tcp
}
listener!(VlessListener {
    #[serde(default)] tcp: TcpOptions,
    #[serde(default = "tcp_stream")] stream: VlessStream,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] uuid: Option<SecretString> [value_type = SecretString],
    #[serde(default)] flow: String,
    tls: ListenerTls,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] websocket: Option<ListenerWebSocket> [value_type = ListenerWebSocket],
});
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum TrojanStream {
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
    #[serde(default)] tcp: TcpOptions,
    #[serde(default)] stream: TrojanStream,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] password: Option<SecretString> [value_type = SecretString],
    tls: ListenerTls,
});
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Listener {
    NativeUdp(NativeUdpListener),
    NativeTcp(NativeTcpListener),
    Vless(VlessListener),
    Trojan(TrojanListener),
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
        Ok(match Fields::deserialize(crate::object::MapOnly(d))? {
            Fields::NativeUdp(v) => Self::NativeUdp(v),
            Fields::NativeTcp(v) => Self::NativeTcp(v),
            Fields::Vless(v) => Self::Vless(v),
            Fields::Trojan(v) => Self::Trojan(v),
        })
    }
}

macro_rules! dialer {
    ($name:ident, $priority:literal, $value:literal, { $($(#[$attr:meta])* $field:ident: $ty:ty $([$($schema:tt)*])?),* $(,)? }) => {
        object! { pub struct $name {
            #[serde(default = "yes")] pub enabled: bool,
            #[serde(default)] pub remark: String,
            #[serde(default = $priority)] pub priority: u16,
            #[serde(default = "five_seconds")] pub connect_timeout: PositiveDuration,
            #[serde(default)] pub bind_interface: String,
            #[serde(default)] pub local_address: String,
            #[serde(default)] pub socket: SocketOptions,
            $($(#[$attr])* pub $field: $ty $([$($schema)*])?),*
        } }
        impl Default for $name { fn default() -> Self { Self {
            enabled: true, remark: String::new(), priority: $value,
            connect_timeout: five_seconds(), bind_interface: String::new(),
            local_address: String::new(), socket: SocketOptions::default(),
            $($field: Default::default()),*
        } } }
    }
}
fn udp_priority() -> u16 {
    100
}
fn tcp_priority() -> u16 {
    200
}
fn vless_priority() -> u16 {
    300
}
fn trojan_priority() -> u16 {
    400
}
dialer!(NativeUdpDialer, "udp_priority", 100, {});
dialer!(NativeTcpDialer, "tcp_priority", 200, { #[serde(default)] tcp: TcpOptions });
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum DialerStream {
    #[default]
    Auto,
    Tcp,
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
dialer!(VlessDialer, "vless_priority", 300, {
    #[serde(default)] tcp: TcpOptions,
    #[serde(default)] stream: DialerStream,
    #[serde(default)] tls: DialerTls,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] websocket: Option<DialerWebSocket> [value_type = DialerWebSocket],
});
dialer!(TrojanDialer, "trojan_priority", 400, { #[serde(default)] tcp: TcpOptions, #[serde(default)] tls: DialerTls });
object! { #[derive(Default)] pub struct DialerSet {
    #[serde(default, deserialize_with = "present", rename = "native_udp", skip_serializing_if = "Option::is_none")] pub native_udp: Option<NativeUdpDialer> [value_type = NativeUdpDialer],
    #[serde(default, deserialize_with = "present", rename = "native_tcp", skip_serializing_if = "Option::is_none")] pub native_tcp: Option<NativeTcpDialer> [value_type = NativeTcpDialer],
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")] pub vless: Option<VlessDialer> [value_type = VlessDialer],
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
