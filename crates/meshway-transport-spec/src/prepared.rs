//! Transport inputs selected by configuration compilation. Their presence does
//! not attest TLS verification, runtime support, or permission to connect.
use crate::{
    DialerTls, ListenerWebSocket, NativeTcpDialer, NativeUdpDialer, SecretString,
    TransportProtocol, TrojanDialer, VlessDialer, VlessStream, object::object,
};

object! { pub struct ClientTls {
    pub server_name: String, pub certificate_chain: SecretString,
    pub options: DialerTls,
} }
meshway_serde::tagged! {
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum DialParameters {
        NativeUdp { options: NativeUdpDialer },
        NativeTcp { options: NativeTcpDialer },
        Vless { options: VlessDialer, stream: VlessStream, flow: String, uuid: SecretString,
            tls: ClientTls, websocket: Option<ListenerWebSocket> },
        Trojan { options: TrojanDialer, password: SecretString, tls: ClientTls },
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
