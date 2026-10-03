use super::*;
use meshway_transport_spec::*;

pub(super) fn candidates(
    input: &CompilationInput<'_>,
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
    let capable = |protocol: &str, stream: &str| {
        let key = format!("transport:{protocol}");
        ac.capabilities
            .get(&key)
            .is_some_and(|c| c.dial && c.streams.iter().any(|s| s == stream))
            && bc
                .capabilities
                .get(&key)
                .is_some_and(|c| c.listen && c.streams.iter().any(|s| s == stream))
    };
    for (key, l) in &target.listeners {
        let (endpoints, priority, parameters) = match l.declaration() {
            Listener::NativeUdp(l) if l.enabled && capable("native_udp", "udp") => {
                let Some(d) = set.native_udp.as_ref().filter(|d| d.enabled) else {
                    continue;
                };
                (
                    &l.endpoints,
                    d.priority,
                    DialParameters::NativeUdp { options: d.clone() },
                )
            }
            Listener::NativeTcp(l) if l.enabled && capable("native_tcp", "tcp") => {
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
                            "tcp"
                        } else {
                            "websocket"
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
                (
                    &l.endpoints,
                    d.priority,
                    DialParameters::Vless {
                        options: d.clone(),
                        stream: l.stream,
                        flow: l.flow.clone(),
                        uuid: l.uuid.clone().ok_or(CompileError::Material)?,
                        tls: tls(&l.tls, &d.tls)?,
                        websocket: l.websocket.clone(),
                    },
                )
            }
            Listener::Trojan(l) if l.enabled && capable("trojan", "tcp") => {
                let Some(d) = set.trojan.as_ref().filter(|d| d.enabled) else {
                    continue;
                };
                (
                    &l.endpoints,
                    d.priority,
                    DialParameters::Trojan {
                        options: d.clone(),
                        password: l.password.clone().ok_or(CompileError::Material)?,
                        tls: tls(&l.tls, &d.tls)?,
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
fn tls(l: &ListenerTls, d: &DialerTls) -> Result<ClientTls, CompileError> {
    // Both peers must support a common range; never weaken configured verification.
    if l.min_version > d.max_version || d.min_version > l.max_version {
        return Err(CompileError::Material);
    }
    Ok(ClientTls {
        server_name: l.server_name.clone(),
        certificate_chain: l.certificate.clone().ok_or(CompileError::Material)?,
        options: d.clone(),
    })
}
