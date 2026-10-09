//! Local admission over an installed netmap and its lease.
//!
//! The caller binds the netmap to its own authenticated network and Node,
//! verifies its digest before installation, and passes the authenticated
//! arrival peer. `now` is used only for the lease; grants have no deadline.
//! Replies still require the caller's own established forward state.
use super::{
    AdmissionError, Carrier, Hop, Traffic, contains, keys_valid, valid_path, valid_traffic,
};
use crate::compile::MAX_PATHS;
use meshway_configuration::{
    NodeAddresses,
    execution::{BusinessProtocol, ExitDestinationScope, ExitNat},
    netmap::*,
};
use meshway_model::{NetmapLease, NodeId, exit::ExitFamily};
use meshway_transport_spec::{Listener, Port, TransportProtocol};
use std::{collections::BTreeMap, net::IpAddr};

/// An installed netmap with the lease issued for exactly its content.
#[derive(Clone, Copy)]
pub struct Leased<'a> {
    pub netmap: &'a NodeNetmap,
    pub lease: &'a NetmapLease,
}

/// Where the source sends a new flow, decided only from its own netmap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    /// A Mesh grant whose services contain the exact flow.
    Mesh(GrantKey),
    /// Exit grants in configured primary/backup order; readiness facts pick one.
    Exit(Vec<GrantKey>),
    /// Local direct execution outside the Mesh.
    Direct,
    /// Local unmanaged routing.
    Unmanaged,
}

fn addresses(netmap: &NodeNetmap, node: NodeId) -> Result<&NodeAddresses, AdmissionError> {
    if node == netmap.node_id {
        Ok(&netmap.self_.addresses)
    } else {
        netmap
            .peers
            .get(&node)
            .map(|peer| &peer.addresses)
            .ok_or(AdmissionError::Peer)
    }
}

fn in_pools(netmap: &NodeNetmap, address: IpAddr) -> bool {
    let pools = &netmap.self_.mesh_address_pools;
    pools.ipv4.iter().chain(&pools.ipv6).any(|pool| {
        pool.parse::<ipnet::IpNet>()
            .is_ok_and(|net| net.contains(&address))
    })
}

fn in_routes(exit: &GrantExit, address: IpAddr) -> bool {
    exit.routes.iter().any(|route| {
        route
            .parse::<ipnet::IpNet>()
            .is_ok_and(|net| net.contains(&address))
    })
}

fn covered(exit: &GrantExit, address: IpAddr) -> bool {
    match &exit.destination {
        ExitDestinationScope::Internet => crate::is_internet_address(address),
        ExitDestinationScope::Cidr { prefix } => prefix
            .parse::<ipnet::IpNet>()
            .is_ok_and(|net| net.contains(&address)),
    }
}

fn valid_self(netmap: &NodeNetmap) -> Result<(), AdmissionError> {
    if !netmap.self_.enabled
        || netmap.peers.contains_key(&netmap.node_id)
        || !keys_valid(
            &netmap.self_.self_keys.control_key,
            &netmap.self_.self_keys.dataplane_key,
        )
    {
        return Err(AdmissionError::Configuration);
    }
    Ok(())
}

/// Classify a new source flow by its destination service. Unassigned
/// Mesh-pool addresses are rejected and never fall through to local egress;
/// an exit route never degrades to direct. Packet admission of the chosen
/// grant still checks the exact source tuple.
pub fn classify(
    netmap: &NodeNetmap,
    destination: IpAddr,
    protocol: BusinessProtocol,
    port: Option<Port>,
) -> Result<Destination, AdmissionError> {
    valid_self(netmap)?;
    let destination = destination.to_canonical();
    if (protocol == BusinessProtocol::IcmpEcho) != port.is_none()
        || destination.is_unspecified()
        || destination.is_multicast()
        || contains(&netmap.self_.addresses, destination)
    {
        return Err(AdmissionError::Traffic);
    }
    if let Some((node, _)) = netmap
        .peers
        .iter()
        .find(|(_, peer)| contains(&peer.addresses, destination))
    {
        let key = GrantKey {
            source: netmap.node_id,
            target: GrantTarget::Mesh { node: *node },
        };
        let grant = netmap.grants.get(&key).ok_or(AdmissionError::Permit)?;
        if !grant
            .services
            .as_ref()
            .is_some_and(|services| services.contains(protocol, port))
        {
            return Err(AdmissionError::Traffic);
        }
        return Ok(Destination::Mesh(key));
    }
    if in_pools(netmap, destination) {
        return Err(AdmissionError::Path);
    }
    let plan = netmap
        .egress
        .as_ref()
        .ok_or(AdmissionError::Configuration)?;
    if plan.capture.unavailable.is_some() {
        return Err(AdmissionError::Traffic);
    }
    match &plan
        .route(destination)
        .ok_or(AdmissionError::Traffic)?
        .action
    {
        EgressAction::Direct => Ok(Destination::Direct),
        EgressAction::Unmanaged => Ok(Destination::Unmanaged),
        EgressAction::Exit { nodes, .. } => {
            if !destination.is_ipv4()
                || netmap.self_.addresses.ipv4.is_none()
                || !crate::is_exit_destination(destination)
            {
                return Err(AdmissionError::Traffic);
            }
            let keys: Vec<GrantKey> = nodes
                .iter()
                .filter_map(|exit| {
                    netmap
                        .exit_grants(*exit)
                        .find(|grant| {
                            grant.exit.as_ref().is_some_and(|e| {
                                in_routes(e, destination) && covered(e, destination)
                            }) && grant
                                .services
                                .as_ref()
                                .is_some_and(|services| services.contains(protocol, port))
                        })
                        .map(Grant::key)
                })
                .collect();
            if keys.is_empty() {
                Err(AdmissionError::Permit)
            } else {
                Ok(Destination::Exit(keys))
            }
        }
    }
}

fn admitted_path<'a>(
    leased: Leased<'a>,
    key: &GrantKey,
    path: &[NodeId],
    origin: NodeId,
    now_ms: u64,
) -> Result<(&'a Grant, usize), AdmissionError> {
    let netmap = leased.netmap;
    if leased.lease.netmap.serial != netmap.serial
        || leased.lease.netmap.revision != netmap.revision
    {
        return Err(AdmissionError::Configuration);
    }
    if !leased.lease.valid_at(now_ms) {
        return Err(AdmissionError::Expired);
    }
    valid_self(netmap)?;
    let grant = netmap.grants.get(key).ok_or(AdmissionError::Permit)?;
    if grant.key() != *key || key.source != origin {
        return Err(AdmissionError::Permit);
    }
    if !valid_path(path)
        || path.first() != Some(&origin)
        || grant.paths.len() > MAX_PATHS
        || grant.path(path).is_none()
    {
        return Err(AdmissionError::Path);
    }
    let position = path
        .iter()
        .position(|node| *node == netmap.node_id)
        .ok_or(AdmissionError::Path)?;
    for node in path.iter().filter(|node| **node != netmap.node_id) {
        let peer = netmap.peers.get(node).ok_or(AdmissionError::Peer)?;
        if !keys_valid(&peer.control_key, &peer.dataplane_key) {
            return Err(AdmissionError::Configuration);
        }
    }
    if addresses(netmap, origin)? != &grant.source_addresses {
        return Err(AdmissionError::Traffic);
    }
    let terminal = path.last().ok_or(AdmissionError::Path)?;
    match (&grant.target, &grant.exit) {
        (GrantTarget::Mesh { node }, None) => {
            if node != terminal {
                return Err(AdmissionError::Path);
            }
        }
        (GrantTarget::Exit { node, .. }, Some(exit)) => {
            if node != terminal {
                return Err(AdmissionError::Path);
            }
            if exit.family != ExitFamily::Ipv4 || grant.source_addresses.ipv4.is_none() {
                return Err(AdmissionError::Traffic);
            }
            if *terminal == netmap.node_id {
                if !netmap.self_.exit_enabled {
                    return Err(AdmissionError::ExitDisabled);
                }
                if !netmap.exit_role.as_ref().is_some_and(|role| {
                    role.family == exit.family && role.nat == ExitNat::Masquerade
                }) {
                    return Err(AdmissionError::Permit);
                }
            }
        }
        _ => return Err(AdmissionError::Permit),
    }
    Ok((grant, position))
}

/// Admit a no-application-data handshake along a grant's exact complete path.
/// `reply` selects travel direction only and never grants reverse traffic.
pub fn admit_handshake(
    leased: Leased<'_>,
    key: &GrantKey,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    reply: bool,
    now_ms: u64,
) -> Result<Hop, AdmissionError> {
    let (grant, position) = admitted_path(leased, key, path, origin, now_ms)?;
    let upstream = position.checked_sub(1).map(|index| path[index]);
    let downstream = path.get(position + 1).copied();
    let (expected, next) = if reply {
        (downstream, upstream)
    } else {
        (upstream, downstream)
    };
    if previous != expected {
        return Err(AdmissionError::Peer);
    }
    Ok(match next {
        Some(next) => Hop::Forward(next),
        None if grant.exit.is_some() && !reply => Hop::Exit,
        None => Hop::Deliver,
    })
}

fn admitted_traffic(
    netmap: &NodeNetmap,
    grant: &Grant,
    traffic: &Traffic,
) -> Result<(), AdmissionError> {
    let services = grant.services.as_ref().ok_or(AdmissionError::Permit)?;
    let destination = traffic.destination;
    if !valid_traffic(traffic)
        || !contains(&grant.source_addresses, traffic.source)
        || !services.contains(traffic.protocol, traffic.destination_port)
    {
        return Err(AdmissionError::Traffic);
    }
    match (&grant.target, &grant.exit) {
        (GrantTarget::Mesh { node }, None) => {
            if !contains(addresses(netmap, *node)?, destination) {
                return Err(AdmissionError::Traffic);
            }
        }
        (GrantTarget::Exit { .. }, Some(exit)) => {
            if contains(&netmap.self_.addresses, destination)
                || netmap
                    .peers
                    .values()
                    .any(|peer| contains(&peer.addresses, destination))
                || in_pools(netmap, destination)
                || !crate::is_exit_destination(destination)
                || !in_routes(exit, destination)
                || !covered(exit, destination)
            {
                return Err(AdmissionError::Traffic);
            }
        }
        _ => return Err(AdmissionError::Permit),
    }
    Ok(())
}

/// Admit a forward packet at the source or terminal; relays never call this.
pub fn admit_forward(
    leased: Leased<'_>,
    key: &GrantKey,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    traffic: &Traffic,
    now_ms: u64,
) -> Result<Hop, AdmissionError> {
    let hop = admit_handshake(leased, key, path, origin, previous, false, now_ms)?;
    admitted_traffic(leased.netmap, &leased.netmap.grants[key], traffic)?;
    Ok(hop)
}

/// `path`, `origin` and `forward` describe the caller's own established
/// forward flow; the reply must be its exact reversed tuple.
#[allow(clippy::too_many_arguments)]
pub fn admit_reply(
    leased: Leased<'_>,
    key: &GrantKey,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    forward: &Traffic,
    reply: &Traffic,
    now_ms: u64,
) -> Result<Hop, AdmissionError> {
    let hop = admit_handshake(leased, key, path, origin, previous, true, now_ms)?;
    admitted_traffic(leased.netmap, &leased.netmap.grants[key], forward)?;
    if !valid_traffic(reply)
        || reply.protocol != forward.protocol
        || reply.source != forward.destination
        || reply.destination != forward.source
        || reply.source_port != forward.destination_port
        || reply.destination_port != forward.source_port
    {
        return Err(AdmissionError::Traffic);
    }
    Ok(hop)
}

/// Admit a diagnostic circuit: the grant's services must contain the
/// measured service and the path must be observable. Without an observable
/// path the request is unsupported, reported as `Path`.
#[allow(clippy::too_many_arguments)]
pub fn admit_observation(
    leased: Leased<'_>,
    key: &GrantKey,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    reply: bool,
    service: (BusinessProtocol, Option<Port>),
    now_ms: u64,
) -> Result<Hop, AdmissionError> {
    let hop = admit_handshake(leased, key, path, origin, previous, reply, now_ms)?;
    let grant = &leased.netmap.grants[key];
    if !matches!(grant.target, GrantTarget::Mesh { .. }) {
        return Err(AdmissionError::Permit);
    }
    // Relays hold no service set; the endpoints check the measured service.
    if let Some(services) = &grant.services
        && !services.contains(service.0, service.1)
    {
        return Err(AdmissionError::Traffic);
    }
    if !grant.path(path).is_some_and(|path| path.observable) {
        return Err(AdmissionError::Path);
    }
    Ok(hop)
}

fn listener_protocol(listener: &Listener) -> (TransportProtocol, bool) {
    match listener {
        Listener::NativeUdp(value) => (TransportProtocol::NativeUdp, value.enabled),
        Listener::NativeTcp(value) => (TransportProtocol::NativeTcp, value.enabled),
        Listener::Vless(value) => (TransportProtocol::Vless, value.enabled),
        Listener::Trojan(value) => (TransportProtocol::Trojan, value.enabled),
    }
}

/// Admit an authenticated adjacency. Only `edges` authorize it; it grants no
/// packet or flow authority, and the snapshot owner enforces the lease.
pub fn admit_adjacency(
    netmap: &NodeNetmap,
    peer: NodeId,
    carrier: &Carrier,
) -> Result<(), AdmissionError> {
    valid_self(netmap)?;
    if peer == netmap.node_id || carrier.listener_key.is_empty() {
        return Err(AdmissionError::Peer);
    }
    let edges = netmap.edges.get(&peer).ok_or(AdmissionError::Peer)?;
    let local = netmap.node_id;
    if carrier.dialer == local && carrier.listener == peer {
        let key = format!("{peer}/{}", carrier.listener_key);
        let dialer = netmap.dialers.get(&key).ok_or(AdmissionError::Peer)?;
        if !edges.dial.contains(&key)
            || dialer.peer != peer
            || dialer.listener_key != carrier.listener_key
            || dialer.parameters.transport() != carrier.protocol
        {
            return Err(AdmissionError::Peer);
        }
    } else if carrier.listener == local && carrier.dialer == peer {
        let listener = netmap
            .self_
            .listeners
            .get(&carrier.listener_key)
            .ok_or(AdmissionError::Peer)?;
        let (protocol, enabled) = listener_protocol(listener.declaration());
        if !edges.accept.contains(&carrier.listener_key) || !enabled || protocol != carrier.protocol
        {
            return Err(AdmissionError::Peer);
        }
    } else {
        return Err(AdmissionError::Peer);
    }
    Ok(())
}

/// Check an actual carrier against one grant path's hop and candidates, in
/// addition to `admit_adjacency`. Carrier choice never bypasses a waypoint.
pub fn admit_path_carrier(
    netmap: &NodeNetmap,
    key: &GrantKey,
    path: &[NodeId],
    peer: NodeId,
    carrier: &Carrier,
) -> Result<(), AdmissionError> {
    let grant = netmap.grants.get(key).ok_or(AdmissionError::Permit)?;
    let selected = grant.path(path).ok_or(AdmissionError::Path)?;
    if !path_carrier(&selected.nodes, &selected.dial_candidates, carrier) {
        return Err(AdmissionError::Path);
    }
    admit_adjacency(netmap, peer, carrier)
}

fn path_carrier(
    nodes: &[NodeId],
    dial_candidates: &BTreeMap<NodeId, Vec<String>>,
    carrier: &Carrier,
) -> bool {
    valid_path(nodes)
        && nodes.windows(2).any(|edge| {
            (edge[0] == carrier.dialer && edge[1] == carrier.listener)
                || (edge[1] == carrier.dialer && edge[0] == carrier.listener)
        })
        && dial_candidates.get(&carrier.dialer).is_some_and(|keys| {
            keys.contains(&format!("{}/{}", carrier.listener, carrier.listener_key))
        })
}
