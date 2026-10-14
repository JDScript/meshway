//! Local admission over the desired netmap.
//!
//! Endpoints admit with a [`Decision`] they computed with `compile_local` on
//! the same projection; relays admit with [`admit_transit`]. The caller binds
//! the netmap to its own authenticated network and Node, verifies its digest
//! before installation, and passes the authenticated arrival peer. The lease
//! is not an input: its owner withdraws the content when it ends. Replies
//! require the caller's own forward state.
use super::{
    AdmissionError, Carrier, Hop, Traffic, contains, keys_valid, valid_path, valid_traffic,
};
use crate::compile::{
    MAX_EXIT_CANDIDATES, MAX_SEARCH_PATHS,
    netmap::{Decision, LocalCompiler, Target, observable},
};
use meshway_configuration::{
    NodeAddresses,
    execution::{BusinessProtocol, ExitNat, LogicalPath},
    netmap::NodeNetmap,
};
use meshway_model::TransportProtocol;
use meshway_model::{NodeId, exit::ExitFamily};
use meshway_transport::spec::{Listener, Port};
use std::net::IpAddr;

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

/// Exact path membership, peer keys and arrival direction shared by every entry.
fn positioned(
    netmap: &NodeNetmap,
    paths: &[LogicalPath],
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    reply: bool,
) -> Result<Option<NodeId>, AdmissionError> {
    valid_self(netmap)?;
    if !valid_path(path)
        || path.first() != Some(&origin)
        || paths.len() > MAX_SEARCH_PATHS * MAX_EXIT_CANDIDATES
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
    netmap: &NodeNetmap,
    decision: &Decision,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    reply: bool,
) -> Result<Hop, AdmissionError> {
    if decision.source != origin {
        return Err(AdmissionError::Permit);
    }
    let next = positioned(netmap, &decision.paths, path, origin, previous, reply)?;
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
    netmap: &NodeNetmap,
    decision: &Decision,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    traffic: &Traffic,
) -> Result<Hop, AdmissionError> {
    let hop = admit_handshake(netmap, decision, path, origin, previous, false)?;
    admitted_traffic(netmap, decision, traffic)?;
    Ok(hop)
}

/// `path`, `origin` and `forward` describe the caller's own established
/// forward flow; the reply must be its exact reversed tuple.
#[allow(clippy::too_many_arguments)]
pub fn admit_reply(
    netmap: &NodeNetmap,
    decision: &Decision,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    forward: &Traffic,
    reply: &Traffic,
) -> Result<Hop, AdmissionError> {
    let hop = admit_handshake(netmap, decision, path, origin, previous, true)?;
    admitted_traffic(netmap, decision, forward)?;
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

/// Admit an ICMP error that relates to the caller's own established flow:
/// `quoted` (the packet the error is about) is exactly that flow's `forward`
/// traffic, and the error travels the reply direction of its original path,
/// addressed to the flow's source. It comes from the quoted destination
/// itself, from the terminal Node's own address or, for an exit flow, from an
/// address outside the Mesh (a router beyond the exit). Nothing else of the
/// error (type, code, quoted length) authorizes anything.
#[allow(clippy::too_many_arguments)]
pub fn admit_related(
    netmap: &NodeNetmap,
    decision: &Decision,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    forward: &Traffic,
    quoted: &Traffic,
    error_source: IpAddr,
    error_destination: IpAddr,
) -> Result<Hop, AdmissionError> {
    let hop = admit_handshake(netmap, decision, path, origin, previous, true)?;
    admitted_traffic(netmap, decision, forward)?;
    let terminal = *path.last().ok_or(AdmissionError::Path)?;
    let source = error_source.to_canonical();
    let from = source == quoted.destination.to_canonical()
        || contains(addresses(netmap, terminal)?, source)
        || (matches!(decision.target, Target::Exit { .. })
            && !in_pools(netmap, source)
            && !source.is_unspecified()
            && !source.is_multicast()
            && source != IpAddr::V4(std::net::Ipv4Addr::BROADCAST));
    if quoted != forward || error_destination != forward.source || !from {
        return Err(AdmissionError::Traffic);
    }
    Ok(hop)
}

/// Admit a circuit through this relay: the projection must produce `path`
/// for `(origin, target)` with some Access service, and this Node must be an
/// intermediate hop. Relays never inspect business traffic.
#[allow(clippy::too_many_arguments)]
pub fn admit_transit(
    netmap: &NodeNetmap,
    compiler: &LocalCompiler,
    origin: NodeId,
    target: &Target,
    path: &[NodeId],
    previous: Option<NodeId>,
    reply: bool,
) -> Result<Hop, AdmissionError> {
    let paths = compiler
        .transit(origin, target)
        .map_err(|_| AdmissionError::Configuration)?;
    admit_transit_paths(netmap, &paths, origin, path, previous, reply)
}

/// The same transit admission with paths already computed by the caller's
/// content-bound local compiler cache. No second compiler or search is needed.
pub fn admit_transit_paths(
    netmap: &NodeNetmap,
    paths: &[LogicalPath],
    origin: NodeId,
    path: &[NodeId],
    previous: Option<NodeId>,
    reply: bool,
) -> Result<Hop, AdmissionError> {
    valid_self(netmap)?;
    let node = netmap.node_id;
    if path.first() == Some(&node) || path.last() == Some(&node) {
        return Err(AdmissionError::Path);
    }
    let next = positioned(netmap, paths, path, origin, previous, reply)?;
    next.map(Hop::Forward).ok_or(AdmissionError::Path)
}

/// Admit the exact path of a DNS service derived by the local compiler.
/// Source and resolver additionally require their published copies to match;
/// relays forward the authenticated circuit and gain no resolver authority.
#[allow(clippy::too_many_arguments)]
pub fn admit_dns(
    netmap: &NodeNetmap,
    service: &meshway_configuration::netmap::DnsService,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    reply: bool,
) -> Result<Hop, AdmissionError> {
    if origin != service.source || path != service.path.nodes || path.last() != Some(&service.exit)
    {
        return Err(AdmissionError::Path);
    }
    if netmap.node_id == service.source || netmap.node_id == service.exit {
        crate::dns_route::netmap_dns_service(
            netmap,
            service,
            origin,
            service.exit,
            path,
            *service.transports.first().ok_or(AdmissionError::Permit)?,
        )
        .map_err(|_| AdmissionError::Permit)?;
    }
    let next = positioned(
        netmap,
        std::slice::from_ref(&service.path),
        path,
        origin,
        previous,
        reply,
    )?;
    Ok(next.map_or(Hop::Deliver, Hop::Forward))
}

/// Admit a diagnostic circuit: the measured service must be in the
/// decision and every Node on the path must declare path observation;
/// otherwise the request is unsupported, reported as `Path`.
#[allow(clippy::too_many_arguments)]
pub fn admit_observation(
    netmap: &NodeNetmap,
    decision: &Decision,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    reply: bool,
    service: (BusinessProtocol, Option<Port>),
) -> Result<Hop, AdmissionError> {
    let hop = admit_handshake(netmap, decision, path, origin, previous, reply)?;
    if !matches!(decision.target, Target::Mesh { .. }) {
        return Err(AdmissionError::Permit);
    }
    if !decision.services.contains(service.0, service.1) {
        return Err(AdmissionError::Traffic);
    }
    if !observable(&netmap.projection.inventory, path) {
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
