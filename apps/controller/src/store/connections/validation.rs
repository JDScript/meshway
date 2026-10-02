use std::{
    collections::BTreeSet,
    net::{IpAddr, SocketAddr},
};

use meshway_protocol::{
    ConnectionDiagnostic, ConnectionEndpoint, ConnectionNode, ConnectionPlan, ConnectionTransport,
};

pub(super) fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

pub(crate) fn usable_ip(ip: IpAddr, bind: bool) -> bool {
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        _ => ip,
    };
    (bind || !ip.is_unspecified())
        && !ip.is_multicast()
        && !matches!(ip, IpAddr::V4(v4) if v4.is_broadcast())
}

pub(crate) fn endpoint(value: &ConnectionEndpoint) -> Result<ConnectionEndpoint, &'static str> {
    if value.port == 0 || value.host.is_empty() || value.host.chars().count() > 253 {
        return Err("endpoint requires a valid host and a nonzero port");
    }
    let host = if let Ok(ip) = value.host.parse::<IpAddr>() {
        if !usable_ip(ip, false) {
            return Err("advertised IP must not be unspecified, multicast or broadcast");
        }
        ip.to_string()
    } else {
        let input = value.host.strip_suffix('.').unwrap_or(&value.host);
        let ascii = idna::domain_to_ascii_strict(input)
            .map_err(|_| "invalid IDNA hostname")?
            .to_ascii_lowercase();
        if ascii.is_empty()
            || ascii.len() > 253
            || ascii
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.')
            || ascii.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
        {
            return Err("invalid DNS hostname");
        }
        // Recognize legacy numeric IPv4 syntax (hex, octal, shortened dotted
        // forms) with the URL library. These must not masquerade as DNS names
        // and bypass the standard IP parser's wildcard/multicast checks.
        if !matches!(url::Host::parse(&ascii), Ok(url::Host::Domain(_))) {
            return Err("IP endpoints must use canonical address syntax");
        }
        ascii
    };
    Ok(ConnectionEndpoint {
        host,
        port: value.port,
    })
}

fn profile(transport: ConnectionTransport, value: &Option<String>) -> bool {
    match transport {
        ConnectionTransport::Vless => value.as_deref().is_some_and(identifier),
        ConnectionTransport::NativeUdp | ConnectionTransport::NativeTcp => value.is_none(),
    }
}

fn capable(
    nodes: &[ConnectionNode],
    id: &str,
    transport: ConnectionTransport,
    accept: bool,
) -> bool {
    nodes.iter().any(|node| {
        node.node.id == id
            && node.node.revoked_at.is_none()
            && node.public_key.is_some()
            && node.capabilities.iter().any(|capability| {
                capability.transport == transport
                    && if accept {
                        capability.can_accept
                    } else {
                        capability.can_dial
                    }
            })
    })
}

/// Normalize unordered source collections and report bounded, typed diagnostics.
/// Inventory observations never contribute to the normalized plan.
pub(super) fn compile(
    source: &ConnectionPlan,
    nodes: &[ConnectionNode],
) -> (ConnectionPlan, Vec<ConnectionDiagnostic>) {
    let mut diagnostics = Vec::new();
    let mut report = |id: Option<&str>, field: &str, message: &'static str| {
        if diagnostics.len() < 64 {
            diagnostics.push(ConnectionDiagnostic {
                item_id: id.filter(|id| identifier(id)).map(str::to_owned),
                field: field.into(),
                message: message.into(),
            });
        }
    };
    if source.listeners.len() > 128 || source.candidates.len() > 256 {
        report(
            None,
            "configuration",
            "at most 128 listeners and 256 candidates are supported",
        );
        return (ConnectionPlan::default(), diagnostics);
    }
    let mut plan = source.clone();
    let mut ids = BTreeSet::new();
    for listener in &mut plan.listeners {
        if !identifier(&listener.id) || !ids.insert(listener.id.clone()) {
            report(
                Some(&listener.id),
                "listeners.id",
                "listener IDs must be unique bounded ASCII identifiers",
            );
        }
        if !capable(nodes, &listener.node_id, listener.transport, true) {
            report(
                Some(&listener.id),
                "listeners.node_id",
                "listener owner must be active with a published key and accept capability",
            );
        }
        match listener.bind.parse::<SocketAddr>() {
            Ok(bind)
                if bind.port() != 0 && usable_ip(bind.ip(), true) && listener.bind.len() <= 128 =>
            {
                listener.bind = bind.to_string()
            }
            _ => report(
                Some(&listener.id),
                "listeners.bind",
                "bind requires a unicast or unspecified socket address and nonzero port",
            ),
        }
        if !profile(listener.transport, &listener.credential_ref) {
            report(
                Some(&listener.id),
                "listeners.credential_ref",
                "VLESS requires a bounded local profile identifier; native transports require null",
            );
        }
        if listener.advertised_endpoints.len() > 16 {
            report(
                Some(&listener.id),
                "listeners.advertised_endpoints",
                "listener requires 1 to 16 advertised endpoints",
            );
        } else {
            let mut endpoints = BTreeSet::new();
            for value in &mut listener.advertised_endpoints {
                match endpoint(value) {
                    Ok(normalized) => {
                        *value = normalized;
                        if !endpoints.insert(value.clone()) {
                            report(
                                Some(&listener.id),
                                "listeners.advertised_endpoints",
                                "duplicate normalized advertised endpoint",
                            );
                        }
                    }
                    Err(message) => report(
                        Some(&listener.id),
                        "listeners.advertised_endpoints",
                        message,
                    ),
                }
            }
            listener.advertised_endpoints.sort();
        }
    }
    ids.clear();
    let mut tuples = BTreeSet::new();
    for candidate in &mut plan.candidates {
        if !identifier(&candidate.id) || !ids.insert(candidate.id.clone()) {
            report(
                Some(&candidate.id),
                "candidates.id",
                "candidate IDs must be unique bounded ASCII identifiers",
            );
        }
        match endpoint(&candidate.endpoint) {
            Ok(normalized) => candidate.endpoint = normalized,
            Err(message) => report(Some(&candidate.id), "candidates.endpoint", message),
        }
        let Some(listener) = plan
            .listeners
            .iter()
            .find(|listener| listener.id == candidate.listener_id)
        else {
            report(
                Some(&candidate.id),
                "candidates.listener_id",
                "candidate references an unknown listener",
            );
            continue;
        };
        if !capable(nodes, &candidate.dialer_node_id, listener.transport, false) {
            report(
                Some(&candidate.id),
                "candidates.dialer_node_id",
                "dialer must be active with a published key and dial capability",
            );
        }
        if candidate.dialer_node_id == listener.node_id {
            report(
                Some(&candidate.id),
                "candidates.dialer_node_id",
                "a node cannot dial its own listener",
            );
        }
        if !listener.advertised_endpoints.contains(&candidate.endpoint) {
            report(
                Some(&candidate.id),
                "candidates.endpoint",
                "candidate endpoint must match a listener advertisement",
            );
        }
        if !profile(listener.transport, &candidate.credential_ref) {
            report(
                Some(&candidate.id),
                "candidates.credential_ref",
                "VLESS requires a bounded local profile identifier; native transports require null",
            );
        }
        if !tuples.insert((
            candidate.dialer_node_id.clone(),
            candidate.listener_id.clone(),
            candidate.endpoint.clone(),
        )) {
            report(
                Some(&candidate.id),
                "candidates",
                "duplicate dialer/listener/endpoint candidate",
            );
        }
    }
    plan.listeners.sort_by(|a, b| a.id.cmp(&b.id));
    plan.candidates.sort_by(|a, b| a.id.cmp(&b.id));
    (plan, diagnostics)
}
