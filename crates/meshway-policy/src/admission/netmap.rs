//! Local admission over an installed netmap and its lease.
//!
//! Endpoints admit with a [`Decision`] they computed with `compile_local` on
//! the same projection; relays admit with [`admit_transit`]. The caller binds
//! the netmap to its own authenticated network and Node, verifies its digest
//! before installation, and passes the authenticated arrival peer. `now` is
//! used only for the lease. Replies require the caller's own forward state.
use super::{
    AdmissionError, Carrier, Hop, Traffic, contains, keys_valid, valid_path, valid_traffic,
};
use crate::compile::{
    MAX_EXIT_CANDIDATES, MAX_PATHS,
    netmap::{Decision, LocalCompiler, Target, observable},
};
use meshway_configuration::{
    NodeAddresses,
    execution::{BusinessProtocol, ExitNat, LogicalPath},
    netmap::NodeNetmap,
};
use meshway_model::{NetmapLease, NodeId, exit::ExitFamily};
use meshway_transport_spec::{Listener, Port, TransportProtocol};
use std::net::IpAddr;

/// An installed netmap with the lease issued for exactly its content.
#[derive(Clone, Copy)]
pub struct Leased<'a> {
    pub netmap: &'a NodeNetmap,
    pub lease: &'a NetmapLease,
}

fn addresses(netmap: &NodeNetmap, node: NodeId) -> Result<&NodeAddresses, AdmissionError> {
    if node == netmap.node_id {
        Ok(&netmap.self_.addresses)
    } else {
        netmap
            .projection
            .configuration
            .nodes
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

fn valid_self(netmap: &NodeNetmap) -> Result<(), AdmissionError> {
    if !netmap.self_.enabled
        || !keys_valid(
            &netmap.self_.self_keys.control_key,
            &netmap.self_.self_keys.dataplane_key,
        )
    {
        return Err(AdmissionError::Configuration);
    }
    Ok(())
}

fn current(leased: Leased<'_>, now_ms: u64) -> Result<(), AdmissionError> {
    let netmap = leased.netmap;
    if leased.lease.netmap.serial != netmap.serial
        || leased.lease.netmap.revision != netmap.revision
    {
        return Err(AdmissionError::Configuration);
    }
    if !leased.lease.valid_at(now_ms) {
        return Err(AdmissionError::Expired);
    }
    valid_self(netmap)
}

/// Exact path membership, peer keys and arrival direction shared by every entry.
fn positioned(
    leased: Leased<'_>,
    paths: &[LogicalPath],
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    reply: bool,
    now_ms: u64,
) -> Result<Option<NodeId>, AdmissionError> {
    current(leased, now_ms)?;
    let netmap = leased.netmap;
    if !valid_path(path)
        || path.first() != Some(&origin)
        || paths.len() > MAX_PATHS * MAX_EXIT_CANDIDATES
        || !paths.iter().any(|candidate| candidate.nodes == path)
    {
        return Err(AdmissionError::Path);
    }
    let position = path
        .iter()
        .position(|node| *node == netmap.node_id)
        .ok_or(AdmissionError::Path)?;
    for node in path.iter().filter(|node| **node != netmap.node_id) {
        let peer = netmap
            .projection
            .inventory
            .get(node)
            .ok_or(AdmissionError::Peer)?;
        if !keys_valid(&peer.control_key, &peer.dataplane_key) {
            return Err(AdmissionError::Configuration);
        }
    }
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
    Ok(next)
}

/// Admit a no-application-data handshake at the source or terminal of a
/// decision's exact path. `reply` selects travel direction only.
pub fn admit_handshake(
    leased: Leased<'_>,
    decision: &Decision,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    reply: bool,
    now_ms: u64,
) -> Result<Hop, AdmissionError> {
    let netmap = leased.netmap;
    if decision.source != origin {
        return Err(AdmissionError::Permit);
    }
    let next = positioned(
        leased,
        &decision.paths,
        path,
        origin,
        previous,
        reply,
        now_ms,
    )?;
    let terminal = *path.last().ok_or(AdmissionError::Path)?;
    if netmap.node_id != origin && netmap.node_id != terminal {
        return Err(AdmissionError::Path);
    }
    let external = match &decision.target {
        Target::Mesh { node } => {
            if *node != terminal {
                return Err(AdmissionError::Path);
            }
            false
        }
        Target::Exit { nodes, .. } => {
            if !nodes.contains(&terminal) {
                return Err(AdmissionError::Path);
            }
            if decision.selection.is_none() || addresses(netmap, origin)?.ipv4.is_none() {
                return Err(AdmissionError::Traffic);
            }
            if terminal == netmap.node_id {
                if !netmap.self_.exit_enabled {
                    return Err(AdmissionError::ExitDisabled);
                }
                if !netmap.exit_role.as_ref().is_some_and(|role| {
                    role.family == ExitFamily::Ipv4 && role.nat == ExitNat::Masquerade
                }) {
                    return Err(AdmissionError::Permit);
                }
            }
            true
        }
    };
    Ok(match next {
        Some(next) => Hop::Forward(next),
        None if external && !reply => Hop::Exit,
        None => Hop::Deliver,
    })
}

fn admitted_traffic(
    netmap: &NodeNetmap,
    decision: &Decision,
    traffic: &Traffic,
) -> Result<(), AdmissionError> {
    let destination = traffic.destination.to_canonical();
    if !valid_traffic(traffic)
        || !contains(addresses(netmap, decision.source)?, traffic.source)
        || !decision
            .services
            .contains(traffic.protocol, traffic.destination_port)
    {
        return Err(AdmissionError::Traffic);
    }
    match &decision.target {
        Target::Mesh { node } => {
            if !contains(addresses(netmap, *node)?, destination) {
                return Err(AdmissionError::Traffic);
            }
        }
        Target::Exit { .. } => {
            if destination != decision.destination.to_canonical()
                || contains(&netmap.self_.addresses, destination)
                || netmap
                    .projection
                    .configuration
                    .nodes
                    .values()
                    .any(|peer| contains(&peer.addresses, destination))
                || in_pools(netmap, destination)
                || !crate::is_exit_destination(destination)
            {
                return Err(AdmissionError::Traffic);
            }
        }
    }
    Ok(())
}

/// Admit a forward packet at the source or terminal; relays use `admit_transit`.
pub fn admit_forward(
    leased: Leased<'_>,
    decision: &Decision,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    traffic: &Traffic,
    now_ms: u64,
) -> Result<Hop, AdmissionError> {
    let hop = admit_handshake(leased, decision, path, origin, previous, false, now_ms)?;
    admitted_traffic(leased.netmap, decision, traffic)?;
    Ok(hop)
}

/// `path`, `origin` and `forward` describe the caller's own established
/// forward flow; the reply must be its exact reversed tuple.
#[allow(clippy::too_many_arguments)]
pub fn admit_reply(
    leased: Leased<'_>,
    decision: &Decision,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    forward: &Traffic,
    reply: &Traffic,
    now_ms: u64,
) -> Result<Hop, AdmissionError> {
    let hop = admit_handshake(leased, decision, path, origin, previous, true, now_ms)?;
    admitted_traffic(leased.netmap, decision, forward)?;
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

/// Admit a circuit through this relay: the projection must produce `path`
/// for `(origin, target)` with some Access service, and this Node must be an
/// intermediate hop. Relays never inspect business traffic.
#[allow(clippy::too_many_arguments)]
pub fn admit_transit(
    leased: Leased<'_>,
    compiler: &LocalCompiler,
    origin: NodeId,
    target: &Target,
    path: &[NodeId],
    previous: Option<NodeId>,
    reply: bool,
    now_ms: u64,
) -> Result<Hop, AdmissionError> {
    current(leased, now_ms)?;
    let node = leased.netmap.node_id;
    if path.first() == Some(&node) || path.last() == Some(&node) {
        return Err(AdmissionError::Path);
    }
    let paths = compiler
        .transit(origin, target)
        .map_err(|_| AdmissionError::Configuration)?;
    let next = positioned(leased, &paths, path, origin, previous, reply, now_ms)?;
    next.map(Hop::Forward).ok_or(AdmissionError::Path)
}

/// Admit a diagnostic circuit: the measured service must be in the
/// decision and every Node on the path must declare path observation;
/// otherwise the request is unsupported, reported as `Path`.
#[allow(clippy::too_many_arguments)]
pub fn admit_observation(
    leased: Leased<'_>,
    decision: &Decision,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    reply: bool,
    service: (BusinessProtocol, Option<Port>),
    now_ms: u64,
) -> Result<Hop, AdmissionError> {
    let hop = admit_handshake(leased, decision, path, origin, previous, reply, now_ms)?;
    if !matches!(decision.target, Target::Mesh { .. }) {
        return Err(AdmissionError::Permit);
    }
    if !decision.services.contains(service.0, service.1) {
        return Err(AdmissionError::Traffic);
    }
    if !observable(&leased.netmap.projection.inventory, path) {
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

/// Admit an authenticated adjacency with a visible Node: topology and both
/// sides' declarations must allow this carrier. Grants no packet or flow
/// authority; the snapshot owner enforces the lease.
pub fn admit_adjacency(
    compiler: &LocalCompiler,
    netmap: &NodeNetmap,
    peer: NodeId,
    carrier: &Carrier,
) -> Result<(), AdmissionError> {
    valid_self(netmap)?;
    let local = netmap.node_id;
    if peer == local || carrier.listener_key.is_empty() {
        return Err(AdmissionError::Peer);
    }
    let error = |_| AdmissionError::Configuration;
    if carrier.dialer == local && carrier.listener == peer {
        let key = format!("{peer}/{}", carrier.listener_key);
        let dialer = netmap.dialers.get(&key).ok_or(AdmissionError::Peer)?;
        if !compiler
            .carriers(local, peer)
            .map_err(error)?
            .contains(&carrier.listener_key)
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
        if !compiler
            .carriers(peer, local)
            .map_err(error)?
            .contains(&carrier.listener_key)
            || !enabled
            || protocol != carrier.protocol
        {
            return Err(AdmissionError::Peer);
        }
    } else {
        return Err(AdmissionError::Peer);
    }
    Ok(())
}

/// Check an actual carrier against one selected path's hop and candidates,
/// in addition to `admit_adjacency`. Carrier choice never bypasses a waypoint.
pub fn admit_path_carrier(
    compiler: &LocalCompiler,
    netmap: &NodeNetmap,
    selected: &LogicalPath,
    peer: NodeId,
    carrier: &Carrier,
) -> Result<(), AdmissionError> {
    if !super::path_carrier(selected, carrier) {
        return Err(AdmissionError::Path);
    }
    admit_adjacency(compiler, netmap, peer, carrier)
}
