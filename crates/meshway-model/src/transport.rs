//! Transport protocol keys and outer socket classification shared by
//! configuration, observations and execution.
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Unknown protocol key values. Errors do not echo the input.
#[derive(Clone, Copy, Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvalidTransportKey {
    #[error("unknown transport protocol key")]
    TransportProtocol,
    #[error("outer socket must be udp or tcp")]
    OuterSocket,
    #[error("VLESS stream must be tcp or websocket")]
    VlessStream,
}

/// Protocol keys, not a Controller enabled catalogue or a Node support report.
#[derive(
    Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash, ToSchema,
)]
#[serde(rename_all = "snake_case", try_from = "String")]
pub enum TransportProtocol {
    /// Native Meshway datagrams over an outer UDP socket.
    NativeUdp,
    /// Native Meshway byte stream over an outer TCP socket.
    NativeTcp,
    /// VLESS TCP CONNECT over TCP/TLS or WebSocket/TLS.
    Vless,
    /// Trojan TCP CONNECT over TLS.
    Trojan,
}

impl TryFrom<String> for TransportProtocol {
    type Error = InvalidTransportKey;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "native_udp" => Ok(Self::NativeUdp),
            "native_tcp" => Ok(Self::NativeTcp),
            "vless" => Ok(Self::Vless),
            "trojan" => Ok(Self::Trojan),
            _ => Err(InvalidTransportKey::TransportProtocol),
        }
    }
}

impl TransportProtocol {
    /// Carrier order tried on an edge whose topology rule lists no order:
    /// Native UDP, Native TCP, VLESS, Trojan.
    pub const PLATFORM_ORDER: [Self; 4] =
        [Self::NativeUdp, Self::NativeTcp, Self::Vless, Self::Trojan];

    /// Position of this protocol in `PLATFORM_ORDER`.
    pub fn platform_rank(self) -> u16 {
        Self::PLATFORM_ORDER
            .iter()
            .position(|p| *p == self)
            .map_or(u16::MAX, |i| i as u16)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NativeUdp => "native_udp",
            Self::NativeTcp => "native_tcp",
            Self::Vless => "vless",
            Self::Trojan => "trojan",
        }
    }

    /// The outer socket, irrespective of the carried business packet protocol.
    /// This classification does not promise NAT traversal or reachability.
    pub const fn outer_socket(self) -> OuterSocket {
        match self {
            Self::NativeUdp => OuterSocket::Udp,
            Self::NativeTcp | Self::Vless | Self::Trojan => OuterSocket::Tcp,
        }
    }
}

/// Outer transport socket classification; never a business access decision.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum OuterSocket {
    /// UDP datagram socket used by the outer carrier.
    Udp,
    /// TCP stream socket used by the outer carrier.
    Tcp,
}

impl TryFrom<String> for OuterSocket {
    type Error = InvalidTransportKey;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "udp" => Ok(Self::Udp),
            "tcp" => Ok(Self::Tcp),
            _ => Err(InvalidTransportKey::OuterSocket),
        }
    }
}

/// Concrete VLESS stream encapsulation. Dialer selection such as `auto` is not
/// an encapsulation, and neither variant means that its runtime is installed.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum VlessStream {
    /// VLESS byte stream directly over TCP/TLS.
    Tcp,
    /// VLESS byte stream wrapped in HTTP/1.1 WebSocket over TCP/TLS.
    #[serde(rename = "websocket")]
    WebSocket,
}

impl TryFrom<String> for VlessStream {
    type Error = InvalidTransportKey;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "tcp" => Ok(Self::Tcp),
            "websocket" => Ok(Self::WebSocket),
            _ => Err(InvalidTransportKey::VlessStream),
        }
    }
}

impl VlessStream {
    pub const fn outer_socket(self) -> OuterSocket {
        OuterSocket::Tcp
    }
}
