//! Transport inputs selected by configuration compilation. Their presence does
//! not attest TLS verification, runtime support, or permission to connect.
use crate::{
    DialerTls, ListenerWebSocket, NativeTcpDialer, NativeUdpDialer, SecretString,
    TransportProtocol, TrojanDialer, VlessDialer, VlessStream, object::object,
};

object! {
/// Target-bound TLS verification material and client options resolved from the selected listener and dialer.
pub struct ClientTls {
    /// TLS verification name supplied by the target listener, distinct from its endpoint host and WebSocket Host.
    pub server_name: String,
    /// Target listener's PEM certificate chain, leaf first, used for trust roots and exact leaf-certificate verification.
    pub certificate_chain: SecretString,
    /// Selected dialer's TLS version, ALPN, trust, and handshake-budget options.
    pub options: DialerTls,
} }
meshway_serde::tagged! {
    #[derive(Clone, Debug, PartialEq, Eq)]
    /// Protocol-specific connection inputs resolved by the Server for a selected target listener.
    pub enum DialParameters {
        /// Native UDP connection settings.
        NativeUdp {
            /// Selected source node's Native UDP dialer options.
            options: NativeUdpDialer
        },
        /// Native TCP connection settings.
        NativeTcp {
            /// Selected source node's Native TCP dialer options.
            options: NativeTcpDialer
        },
        /// VLESS connection settings with the target's credential and encapsulation.
        Vless {
            /// Selected source node's VLESS dialer options, including any additional WebSocket request headers.
            options: VlessDialer,
            /// Concrete target listener encapsulation, `tcp` or `websocket`, after dialer compatibility selection.
            stream: VlessStream,
            /// Target listener's VLESS flow; only the empty string is supported.
            flow: String,
            /// Target listener's explicit or Server-generated, saved UUID credential.
            uuid: SecretString,
            /// Target-bound TLS verification material and selected client options.
            tls: ClientTls,
            /// Target listener's resolved WebSocket path, Host, and response headers; null or omission indicates no WebSocket parameters.
            websocket: Option<ListenerWebSocket>
        },
        /// Trojan TCP CONNECT settings with the target's password and TLS verification material.
        Trojan {
            /// Selected source node's Trojan dialer options.
            options: TrojanDialer,
            /// Target listener's explicit or Server-generated, saved password, preserved without trimming or case normalization.
            password: SecretString,
            /// Target-bound TLS verification material and selected client options.
            tls: ClientTls
        },
    }
}

impl DialParameters {
    pub fn transport(&self) -> TransportProtocol {
        match self {
            Self::NativeUdp { .. } => TransportProtocol::NativeUdp,
            Self::NativeTcp { .. } => TransportProtocol::NativeTcp,
            Self::Vless { .. } => TransportProtocol::Vless,
            Self::Trojan { .. } => TransportProtocol::Trojan,
        }
    }
}
