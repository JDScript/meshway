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
    let Some(rule) = crate::topology_decision(cfg, input.subjects, a, b) else {
        return Ok(vec![]);
    };
    let allowed = rule.map(|index| &cfg.topology[index].transports);
    // A listed edge order is the carrier preference; otherwise the platform order.
    let rank = |protocol: TransportProtocol| -> u16 {
        match allowed {
            Some(Input::Value(protocols)) => protocols
                .iter()
                .position(|p| *p == protocol)
                .map_or(u16::MAX, |i| i as u16),
            _ => protocol.platform_rank(),
        }
    };
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
        let protocol = protocol(l.declaration());
        if matches!(allowed, Some(Input::Value(protocols)) if !protocols.contains(&protocol)) {
            continue;
        }
        if !compatible(set, l.declaration()) {
            continue;
        }
        let priority = rank(protocol);
        let (endpoints, parameters) = match l.declaration() {
            Listener::NativeUdp(l) if l.enabled && capable("native_udp", StreamKind::Udp) => {
                let Some(d) = set.native_udp.as_ref().filter(|d| d.enabled) else {
                    continue;
                };
                (
                    &l.endpoints,
                    DialParameters::NativeUdp { options: d.clone() },
                )
            }
            Listener::NativeTcp(l) if l.enabled && capable("native_tcp", StreamKind::Tcp) => {
                let Some(d) = set.native_tcp.as_ref().filter(|d| d.enabled) else {
                    continue;
                };
                (
                    &l.endpoints,
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
                let uuid = l.uuid.clone().ok_or(CompileError::Material)?;
                let tls = tls(&l.tls, &d.tls)?;
                (
                    &l.endpoints,
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
                let tls = tls(&l.tls, &d.tls)?;
                (
                    &l.endpoints,
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
            endpoints: endpoints.iter().cloned().map(Into::into).collect(),
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
fn protocol(listener: &Listener) -> TransportProtocol {
    match listener {
        Listener::NativeUdp(_) => TransportProtocol::NativeUdp,
        Listener::NativeTcp(_) => TransportProtocol::NativeTcp,
        Listener::Vless(_) => TransportProtocol::Vless,
        Listener::Trojan(_) => TransportProtocol::Trojan,
    }
}

/// Preview uses the same edge, protocol, stream and TLS intersection, without
/// manufacturing runtime capabilities or material.
pub(super) fn declared(
    cfg: &NetworkConfiguration,
    subjects: &SubjectContext,
    a: NodeId,
    b: NodeId,
) -> bool {
    let Some(rule) = crate::topology_decision(cfg, subjects, a, b) else {
        return false;
    };
    let allowed = rule.map(|index| &cfg.topology[index].transports);
    let source = &cfg.nodes[&a];
    if !source.enabled || !cfg.nodes[&b].enabled {
        return false;
    }
    let set = match &source.dialers {
        Input::Value(set) => set,
        Input::Omitted => &cfg.defaults.dialers,
    };
    cfg.nodes[&b].listeners.values().any(|listener| {
        !matches!(allowed, Some(Input::Value(protocols)) if !protocols.contains(&protocol(listener.declaration())))
            && compatible(set, listener.declaration())
    })
}

fn compatible(set: &DialerSet, listener: &Listener) -> bool {
    let tls = |l: &ListenerTls, d: &DialerTls| {
        l.min_version <= d.max_version && d.min_version <= l.max_version
    };
    match listener {
        Listener::NativeUdp(l) => l.enabled && set.native_udp.as_ref().is_some_and(|d| d.enabled),
        Listener::NativeTcp(l) => l.enabled && set.native_tcp.as_ref().is_some_and(|d| d.enabled),
        Listener::Vless(l) => {
            l.enabled
                && set.vless.as_ref().is_some_and(|d| {
                    d.enabled
                        && tls(&l.tls, &d.tls)
                        && matches!(
                            (d.stream, l.stream),
                            (DialerStream::Auto, _)
                                | (DialerStream::Tcp, VlessStream::Tcp)
                                | (DialerStream::WebSocket, VlessStream::WebSocket)
                        )
                })
        }
        Listener::Trojan(l) => {
            l.enabled
                && set
                    .trojan
                    .as_ref()
                    .is_some_and(|d| d.enabled && tls(&l.tls, &d.tls))
        }
    }
}
fn tls(l: &ListenerTls, d: &DialerTls) -> Result<ClientTls, CompileError> {
    // PreparedListener is the trusted certificate validation boundary. Missing
    // material on a compatible candidate is corruption.
    let certificate_chain = l.certificate.clone().ok_or(CompileError::Material)?;
    if l.private_key.is_none() {
        return Err(CompileError::Material);
    }
    // Declaration compatibility has already checked the version intersection.
    Ok(ClientTls {
        server_name: l.server_name.clone(),
        certificate_chain,
        options: d.clone(),
    })
}

/// Projection counterpart of `candidates`: the same edge, protocol, stream,
/// TLS and capability intersection evaluated on redacted declarations.
/// Returns listener keys in carrier order; dial material is never derived.
pub(super) fn offers(
    input: &Topology<'_>,
    views: &ConfigurationView,
    a: NodeId,
    b: NodeId,
) -> Result<Vec<String>, CompileError> {
    let cfg = input.configuration;
    let (Some(source), Some(target), Some(ac), Some(bc)) = (
        views.nodes.get(&a),
        views.nodes.get(&b),
        input.inventory.get(&a),
        input.inventory.get(&b),
    ) else {
        return Ok(vec![]);
    };
    if !source.enabled || !target.enabled {
        return Ok(vec![]);
    }
    let Some(rule) = crate::topology_decision(cfg, input.subjects, a, b) else {
        return Ok(vec![]);
    };
    let allowed = rule.map(|index| &cfg.topology[index].transports);
    let rank = |protocol: TransportProtocol| -> u16 {
        match allowed {
            Some(Input::Value(protocols)) => protocols
                .iter()
                .position(|p| *p == protocol)
                .map_or(u16::MAX, |i| i as u16),
            _ => protocol.platform_rank(),
        }
    };
    let set = match &source.dialers {
        Input::Value(d) => d,
        Input::Omitted => &views.defaults.dialers,
    };
    let capable = |protocol: &str, stream: StreamKind| {
        let key = format!("transport:{protocol}");
        let supports = |node: &TrustedNode, role| {
            matches!(node.capabilities.get(&key),
            Some(Capability::Transport { roles, streams }) if roles.contains(&role) && streams.contains(&stream))
        };
        supports(ac, TransportRole::Dial) && supports(bc, TransportRole::Listen)
    };
    let versions = |l: &Option<ListenerTlsView>, d: &DialerTls| {
        l.as_ref()
            .is_some_and(|l| l.min_version <= d.max_version && d.min_version <= l.max_version)
    };
    let mut out = Vec::new();
    for (key, l) in &target.listeners {
        if matches!(allowed, Some(Input::Value(protocols)) if !protocols.contains(&l.protocol)) {
            continue;
        }
        let usable = l.enabled
            && match l.protocol {
                TransportProtocol::NativeUdp => {
                    set.native_udp.as_ref().is_some_and(|d| d.enabled)
                        && capable("native_udp", StreamKind::Udp)
                }
                TransportProtocol::NativeTcp => {
                    set.native_tcp.as_ref().is_some_and(|d| d.enabled)
                        && capable("native_tcp", StreamKind::Tcp)
                }
                TransportProtocol::Vless => {
                    let stream = l.stream.unwrap_or(VlessStream::Tcp);
                    set.vless.as_ref().is_some_and(|d| {
                        d.enabled
                            && versions(&l.tls, &d.tls)
                            && matches!(
                                (d.stream, stream),
                                (DialerStream::Auto, _)
                                    | (DialerStream::Tcp, VlessStream::Tcp)
                                    | (DialerStream::WebSocket, VlessStream::WebSocket)
                            )
                    }) && capable(
                        "vless",
                        if stream == VlessStream::Tcp {
                            StreamKind::Tcp
                        } else {
                            StreamKind::Websocket
                        },
                    )
                }
                TransportProtocol::Trojan => {
                    set.trojan
                        .as_ref()
                        .is_some_and(|d| d.enabled && versions(&l.tls, &d.tls))
                        && capable("trojan", StreamKind::Tcp)
                }
            };
        if usable {
            out.push((rank(l.protocol), key.clone()));
        }
    }
    out.sort();
    if out.len() > 32 {
        return Err(CompileError::Budget);
    }
    Ok(out.into_iter().map(|(_, key)| key).collect())
}
