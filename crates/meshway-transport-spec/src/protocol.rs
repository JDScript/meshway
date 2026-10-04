use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::ValidationError;

/// Protocol keys, not a Controller enabled catalogue or a Node support report.
#[derive(
    Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash, ToSchema,
)]
#[serde(rename_all = "snake_case", try_from = "String")]
pub enum TransportProtocol {
    NativeUdp,
    NativeTcp,
    Vless,
    Trojan,
}

impl TryFrom<String> for TransportProtocol {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "native_udp" => Ok(Self::NativeUdp),
            "native_tcp" => Ok(Self::NativeTcp),
            "vless" => Ok(Self::Vless),
            "trojan" => Ok(Self::Trojan),
            _ => Err(ValidationError::TransportProtocol),
        }
    }
}

impl TransportProtocol {
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
    Udp,
    Tcp,
}

impl TryFrom<String> for OuterSocket {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "udp" => Ok(Self::Udp),
            "tcp" => Ok(Self::Tcp),
            _ => Err(ValidationError::OuterSocket),
        }
    }
}

/// Concrete VLESS stream encapsulation. Dialer selection such as `auto` is not
/// an encapsulation, and neither variant means that its runtime is installed.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum VlessStream {
    Tcp,
    #[serde(rename = "websocket")]
    WebSocket,
}

impl TryFrom<String> for VlessStream {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "tcp" => Ok(Self::Tcp),
            "websocket" => Ok(Self::WebSocket),
            _ => Err(ValidationError::VlessStream),
        }
    }
}

impl VlessStream {
    pub const fn outer_socket(self) -> OuterSocket {
        OuterSocket::Tcp
    }
}
