use std::{collections::BTreeMap, collections::BTreeSet, net::IpAddr};

use meshway_protocol::{
    AgentConnectionsSnapshot, CONNECTIONS_SCHEMA_VERSION, ConnectionCapability, ConnectionEndpoint,
    ConnectionTransport, MAX_CONNECTIONS_BYTES, PublishConnectionIdentityRequest,
    connection_digest, valid_connection_public_key,
};

use super::super::{AgentError, Identity, local};

pub(super) fn validate(
    snapshot: &AgentConnectionsSnapshot,
    identity: &Identity,
    own_key: &str,
    previous: Option<&AgentConnectionsSnapshot>,
) -> Result<(), AgentError> {
    let payload = &snapshot.payload;
    if serde_json::to_vec(snapshot)
        .map_err(|_| AgentError::Invalid("cannot encode connection configuration"))?
        .len()
        > MAX_CONNECTIONS_BYTES
        || payload.schema_version != CONNECTIONS_SCHEMA_VERSION
        || payload.revision == 0
        || Some(&payload.node_id) != identity.node_id.as_ref()
        || payload.public_key.as_deref() != Some(own_key)
        || connection_digest(payload)
            .map_err(|_| AgentError::Invalid("cannot hash connection configuration"))?
            != snapshot.digest
    {
        return Err(AgentError::Invalid(
            "invalid connection schema, node/key binding, size, or digest",
        ));
    }
    PublishConnectionIdentityRequest {
        public_key: own_key.to_owned(),
        capabilities: payload.capabilities.clone(),
    }
    .validate()
    .map_err(|_| AgentError::Invalid("invalid connection capabilities"))?;
    if payload
        .capabilities
        .windows(2)
        .any(|pair| pair[0].transport >= pair[1].transport)
    {
        return Err(AgentError::Invalid("noncanonical connection capabilities"));
    }
    if let Some(old) = previous {
        if payload.revision < old.payload.revision
            || (payload.revision == old.payload.revision && snapshot.digest != old.digest)
        {
            return Err(AgentError::Invalid(
                "connection configuration rollback or same-revision mutation",
            ));
        }
        for peer in &payload.peers {
            if old.payload.peers.iter().any(|previous| {
                (previous.node_id == peer.node_id && previous.public_key != peer.public_key)
                    || (previous.public_key == peer.public_key && previous.node_id != peer.node_id)
            }) {
                return Err(AgentError::Invalid("connection peer key binding changed"));
            }
        }
    }
    if payload.listeners.len() > 128
        || payload.outgoing.len() + payload.incoming.len() > 256
        || payload.peers.len() > 256
        || payload.diagnostics.len() > 64
        || payload.diagnostics.iter().any(|diagnostic| {
            diagnostic.item_id.as_ref().is_some_and(|id| !valid_id(id))
                || diagnostic.field.is_empty()
                || diagnostic.field.len() > 128
                || diagnostic.message.is_empty()
                || diagnostic.message.len() > 1024
        })
        || (payload.valid && !payload.diagnostics.is_empty())
        || (!payload.valid
            && (payload.diagnostics.is_empty()
                || !payload.listeners.is_empty()
                || !payload.outgoing.is_empty()
                || !payload.incoming.is_empty()
                || !payload.peers.is_empty()))
    {
        return Err(AgentError::Invalid(
            "connection configuration exceeds bounds or is not fail-closed",
        ));
    }

    let mut peer_ids = BTreeSet::new();
    let mut keys = BTreeSet::from([own_key]);
    for peer in &payload.peers {
        if !local::valid_credential(&peer.node_id)
            || peer.node_id == payload.node_id
            || !valid_connection_public_key(&peer.public_key)
            || !keys.insert(&peer.public_key)
            || !peer_ids.insert(&peer.node_id)
        {
            return Err(AgentError::Invalid(
                "invalid or duplicate connection peer binding",
            ));
        }
    }
    if payload
        .peers
        .windows(2)
        .any(|pair| pair[0].node_id >= pair[1].node_id)
        || payload
            .listeners
            .windows(2)
            .any(|pair| pair[0].id >= pair[1].id)
        || payload
            .incoming
            .windows(2)
            .any(|pair| pair[0].candidate_id >= pair[1].candidate_id)
        || payload
            .outgoing
            .windows(2)
            .any(|pair| (pair[0].priority, &pair[0].id) >= (pair[1].priority, &pair[1].id))
    {
        return Err(AgentError::Invalid(
            "noncanonical connection projection ordering",
        ));
    }
    let mut listeners = BTreeMap::new();
    for listener in &payload.listeners {
        let bind = listener
            .bind
            .parse::<std::net::SocketAddr>()
            .map_err(|_| AgentError::Invalid("invalid local connection listener bind"))?;
        if !valid_id(&listener.id)
            || listener.node_id != payload.node_id
            || !supports(&payload.capabilities, listener.transport, false)
            || bind.port() == 0
            || !usable_ip(bind.ip(), true)
            || bind.to_string() != listener.bind
            || listener.advertised_endpoints.is_empty()
            || listener.advertised_endpoints.len() > 16
            || !valid_profile(listener.transport, listener.credential_ref.as_deref())
            || listeners.insert(&listener.id, listener).is_some()
            || listener
                .advertised_endpoints
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || listener
                .advertised_endpoints
                .iter()
                .any(|endpoint| !valid_endpoint(endpoint))
        {
            return Err(AgentError::Invalid(
                "invalid owned connection listener or capability",
            ));
        }
    }
    let mut referenced_peers = BTreeSet::new();
    let mut candidates = BTreeSet::new();
    let mut remote_listeners = BTreeMap::new();
    let mut outgoing_endpoints = BTreeSet::new();
    for candidate in &payload.outgoing {
        let binding = (&candidate.peer_node_id, candidate.transport);
        if !valid_id(&candidate.id)
            || !valid_id(&candidate.listener_id)
            || !peer_ids.contains(&candidate.peer_node_id)
            || !candidates.insert(&candidate.id)
            || listeners.contains_key(&candidate.listener_id)
            || !supports(&payload.capabilities, candidate.transport, true)
            || !valid_endpoint(&candidate.endpoint)
            || !valid_profile(candidate.transport, candidate.credential_ref.as_deref())
            || remote_listeners
                .insert(&candidate.listener_id, binding)
                .is_some_and(|old| old != binding)
            || !outgoing_endpoints.insert((&candidate.listener_id, &candidate.endpoint))
        {
            return Err(AgentError::Invalid(
                "invalid outgoing connection reference or capability",
            ));
        }
        // Remote listener profiles/binds intentionally are not projected. We can
        // validate the local dial grant and its public peer, not remote secrets.
        referenced_peers.insert(&candidate.peer_node_id);
    }
    for approval in &payload.incoming {
        if !valid_id(&approval.candidate_id)
            || !peer_ids.contains(&approval.peer_node_id)
            || !candidates.insert(&approval.candidate_id)
            || !listeners
                .get(&approval.listener_id)
                .is_some_and(|listener| listener.transport == approval.transport)
        {
            return Err(AgentError::Invalid(
                "invalid incoming connection approval reference",
            ));
        }
        referenced_peers.insert(&approval.peer_node_id);
    }
    if referenced_peers != peer_ids {
        return Err(AgentError::Invalid(
            "connection peers exceed adjacent authorization scope",
        ));
    }
    Ok(())
}

fn supports(
    capabilities: &[ConnectionCapability],
    transport: ConnectionTransport,
    dial: bool,
) -> bool {
    capabilities.iter().any(|capability| {
        capability.transport == transport
            && if dial {
                capability.can_dial
            } else {
                capability.can_accept
            }
    })
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn valid_profile(transport: ConnectionTransport, value: Option<&str>) -> bool {
    match transport {
        ConnectionTransport::NativeUdp | ConnectionTransport::NativeTcp => value.is_none(),
        ConnectionTransport::Vless => value.is_some_and(valid_id),
    }
}

fn valid_endpoint(endpoint: &ConnectionEndpoint) -> bool {
    let host = &endpoint.host;
    if endpoint.port == 0 || host.is_empty() || host.len() > 253 {
        return false;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return ip.to_string() == *host && usable_ip(ip, false);
    }
    // Stored DNS is canonical lowercase IDNA ASCII, without a trailing dot.
    if host
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.')
        || !host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
    {
        return false;
    }
    idna::domain_to_ascii_strict(host).is_ok_and(|normalized| normalized == *host)
}

fn usable_ip(ip: IpAddr, bind: bool) -> bool {
    if (!bind && ip.is_unspecified()) || ip.is_multicast() {
        return false;
    }
    match ip {
        IpAddr::V4(ip) => !ip.is_broadcast(),
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .is_none_or(|ip| usable_ip(IpAddr::V4(ip), bind)),
    }
}
