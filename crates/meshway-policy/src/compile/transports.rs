use super::*;
use meshway_model::state::{Capability, StreamKind, TransportRole};
use meshway_transport_spec::*;

pub(super) fn candidates(
    input: &Topology<'_>,
    a: NodeId,
    b: NodeId,
) -> Result<Vec<DialCandidate>, CompileError> {
    let cfg = input.configuration;
    let (Some(source), Some(target), Some(ac), Some(bc)) = (
        cfg.nodes.get(&a),
        cfg.nodes.get(&b),
        input.inventory.get(&a),
        input.inventory.get(&b),
    ) else {
        return Ok(vec![]);
    };
    if !source.enabled || !target.enabled {
        return Ok(vec![]);
    }
    let set = match &source.dialers {
        Input::Value(d) => d,
        Input::Omitted => &cfg.defaults.dialers,
    };
    let mut out = Vec::new();
    let capable = |protocol: &str, stream: StreamKind| {
        let key = format!("transport:{protocol}");
        let supports = |node: &TrustedNode, role| {
            matches!(node.capabilities.get(&key),
            Some(Capability::Transport { roles, streams }) if roles.contains(&role) && streams.contains(&stream))
        };
        supports(ac, TransportRole::Dial) && supports(bc, TransportRole::Listen)
    };
    for (key, l) in &target.listeners {
        let (endpoints, priority, parameters) = match l.declaration() {
            Listener::NativeUdp(l) if l.enabled && capable("native_udp", StreamKind::Udp) => {
                let Some(d) = set.native_udp.as_ref().filter(|d| d.enabled) else {
                    continue;
                };
                (
                    &l.endpoints,
                    d.priority,
                    DialParameters::NativeUdp { options: d.clone() },
                )
            }
            Listener::NativeTcp(l) if l.enabled && capable("native_tcp", StreamKind::Tcp) => {
                let Some(d) = set.native_tcp.as_ref().filter(|d| d.enabled) else {
                    continue;
                };
                (
                    &l.endpoints,
                    d.priority,
                    DialParameters::NativeTcp { options: d.clone() },
                )
            }
            Listener::Vless(l)
                if l.enabled
                    && capable(
                        "vless",
                        if l.stream == VlessStream::Tcp {
                            StreamKind::Tcp
                        } else {
                            StreamKind::Websocket
                        },
                    ) =>
            {
                let Some(d) = set.vless.as_ref().filter(|d| d.enabled) else {
                    continue;
                };
                if !matches!(
                    (d.stream, l.stream),
                    (DialerStream::Auto, _)
                        | (DialerStream::Tcp, VlessStream::Tcp)
                        | (DialerStream::WebSocket, VlessStream::WebSocket)
                ) {
                    continue;
                }
                let uuid = l.uuid.clone().ok_or(CompileError::Material)?;
                let Some(tls) = tls(&l.tls, &d.tls)? else {
                    continue;
                };
                (
                    &l.endpoints,
                    d.priority,
                    DialParameters::Vless {
                        options: d.clone(),
                        stream: l.stream,
                        flow: l.flow.clone(),
                        uuid,
                        tls,
                        websocket: l.websocket.clone(),
                    },
                )
            }
            Listener::Trojan(l) if l.enabled && capable("trojan", StreamKind::Tcp) => {
                let Some(d) = set.trojan.as_ref().filter(|d| d.enabled) else {
                    continue;
                };
                let password = l.password.clone().ok_or(CompileError::Material)?;
                let Some(tls) = tls(&l.tls, &d.tls)? else {
                    continue;
                };
                (
                    &l.endpoints,
                    d.priority,
                    DialParameters::Trojan {
                        options: d.clone(),
                        password,
                        tls,
                    },
                )
            }
            _ => continue,
        };
        // Empty configured endpoints remain a valid dynamic-discovery candidate.
        // State must supply an authenticated matching resource; this is not reachability.
        out.push(DialCandidate {
            peer: b,
            listener_key: key.clone(),
            endpoints: endpoints.clone(),
            priority,
            parameters,
        });
    }
    out.sort_by_key(|c| (c.priority, c.listener_key.clone()));
    if out.len() > 32 {
        return Err(CompileError::Budget);
    }
    Ok(out)
}
fn tls(l: &ListenerTls, d: &DialerTls) -> Result<Option<ClientTls>, CompileError> {
    // PreparedListener is the trusted certificate validation boundary. Missing
    // material is corruption, even if this pair also has disjoint TLS bounds.
    let certificate_chain = l.certificate.clone().ok_or(CompileError::Material)?;
    if l.private_key.is_none() {
        return Err(CompileError::Material);
    }
    // Both peers must support a common range; never weaken configured verification.
    if l.min_version > d.max_version || d.min_version > l.max_version {
        return Ok(None);
    }
    Ok(Some(ClientTls {
        server_name: l.server_name.clone(),
        certificate_chain,
        options: d.clone(),
    }))
}
