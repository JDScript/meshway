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
    #[serde(default)] pub access: AccessDefault,
    #[serde(default)] pub topology: TopologyDefault,
    pub dialers: DialerSetUpdate,
} }
impl DefaultsUpdate {
    pub fn apply(&self, previous: &Defaults) -> Defaults {
        Defaults {
            access: self.access,
            topology: self.topology,
            dialers: self.dialers.apply(&previous.dialers),
        }
    }
}

/// Same protocol fields as a declaration, with secret omission interpreted as
/// keep only while updating the same resource. New resources have no predecessor.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(transparent)]
pub struct ListenerUpdate(pub Listener);
impl ListenerUpdate {
    pub fn apply(&self, previous: Option<&Listener>) -> Listener {
        let mut listener = self.0.clone();
        if let Some(previous) = previous {
            inherit_materials(&mut listener, previous);
        }
        listener
    }
}
