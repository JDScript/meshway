//! Pure circuit and business-packet predicates over an installed target.
//!
//! The caller must bind `configuration.network_id` and `configuration.node_id`
//! to its own authenticated network/node, check credentials and lease, and pass
//! the authenticated arrival peer. Neither a RequestId nor these predicates
//! prove authentication or establish a flow. Replies require the caller's own
//! established forward state, never a forward tuple supplied by the wire.
use crate::compile::{MAX_EXIT_CANDIDATES, MAX_HOPS, MAX_PATHS};
pub use meshway_configuration::execution::Carrier;
use meshway_configuration::{NodeAddresses, execution::*};
use meshway_model::{BoundNodeKey, KeyAlgorithm, NodeId, RequestId};
use meshway_transport_spec::{Listener, Port, TransportProtocol};
use std::net::IpAddr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Traffic {
    pub source: IpAddr,
    pub destination: IpAddr,
    pub protocol: BusinessProtocol,
    pub source_port: Option<Port>,
    pub destination_port: Option<Port>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hop {
    Forward(NodeId),
    Deliver,
    Exit,
}

/// Bounded labels only; no addresses, keys, payloads or raw configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AdmissionError {
    #[error("invalid or disabled configuration")]
    Configuration,
    #[error("permit is absent or inconsistent")]
    Permit,
    #[error("permit has expired")]
    Expired,
    #[error("complete path is not permitted")]
    Path,
    #[error("arrival peer is not permitted")]
    Peer,
    #[error("traffic is not permitted")]
    Traffic,
    #[error("external exit is disabled")]
    ExitDisabled,
}

fn contains(addresses: &NodeAddresses, address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => addresses.ipv4 == Some(address),
        IpAddr::V6(address) => addresses.ipv6 == Some(address),
    }
}

fn keys_valid(control: &BoundNodeKey, dataplane: &BoundNodeKey) -> bool {
    control.public_key.algorithm == KeyAlgorithm::Ed25519
        && dataplane.public_key.algorithm == KeyAlgorithm::X25519
        && control.id != dataplane.id
}

fn addresses(
    configuration: &NodeConfiguration,
    node: NodeId,
) -> Result<&NodeAddresses, AdmissionError> {
    if node == configuration.node_id {
        Ok(&configuration.addresses)
    } else {
        configuration
            .peers
            .get(&node)
            .map(|peer| &peer.addresses)
            .ok_or(AdmissionError::Peer)
    }
}

fn valid_traffic(traffic: &Traffic) -> bool {
    traffic.source.is_ipv4() == traffic.destination.is_ipv4()
        && !traffic.source.is_unspecified()
        && !traffic.destination.is_unspecified()
        && !traffic.source.is_multicast()
        && !traffic.destination.is_multicast()
        && match traffic.protocol {
            BusinessProtocol::Tcp | BusinessProtocol::Udp => {
                traffic.source_port.is_some() && traffic.destination_port.is_some()
            }
            BusinessProtocol::IcmpEcho => {
                traffic.source_port.is_none() && traffic.destination_port.is_none()
            }
        }
}

fn valid_path(path: &[NodeId]) -> bool {
    path.len() >= 2
        && path.len() <= MAX_HOPS
        && !path
            .iter()
            .enumerate()
            .any(|(index, node)| path[..index].contains(node))
}

/// Validate the complete original forward path, independent of the
/// direction of this frame. The exact tagged grant selects its own namespace;
/// diagnostics cannot substitute for a business permit at packet admission.
fn admitted_path(
    configuration: &NodeConfiguration,
    authorization: CircuitAuthorization,
    path: &[NodeId],
    origin: NodeId,
    now_ms: u64,
) -> Result<(usize, bool), AdmissionError> {
    if !configuration.enabled
        || configuration.peers.contains_key(&configuration.node_id)
        || !keys_valid(
            &configuration.self_keys.control_key,
            &configuration.self_keys.dataplane_key,
        )
    {
        return Err(AdmissionError::Configuration);
    }
    let permit = circuit_permit(configuration, authorization)?;
    if permit.source != origin {
        return Err(AdmissionError::Permit);
    }
    if now_ms >= permit.expires_at_ms {
        return Err(AdmissionError::Expired);
    }
    if !valid_path(path)
        || path.first() != Some(&origin)
        || permit.paths.len() > MAX_PATHS * MAX_EXIT_CANDIDATES
        || !permit.paths.iter().any(|candidate| candidate.nodes == path)
    {
        return Err(AdmissionError::Path);
    }
    let position = path
        .iter()
        .position(|node| *node == configuration.node_id)
        .ok_or(AdmissionError::Path)?;
    for node in path {
        if *node != configuration.node_id {
            let peer = configuration.peers.get(node).ok_or(AdmissionError::Peer)?;
            if !keys_valid(&peer.control_key, &peer.dataplane_key) {
                return Err(AdmissionError::Configuration);
            }
        }
    }
    if let CircuitAuthorization::Business(id) = authorization {
        let business = &configuration.permits[&id];
        if addresses(configuration, origin)? != &business.source_addresses {
            return Err(AdmissionError::Traffic);
        }
        match (&business.target, &business.exit) {
            (BusinessTarget::Mesh { .. }, None) => {}
            (BusinessTarget::Ip { address }, Some(exit)) => {
                if exit.family != meshway_model::exit::ExitFamily::Ipv4
                    || !crate::is_exit_destination(address.0)
                    || business.source_addresses.ipv4.is_none()
                    || configuration.mesh_address_pools.ipv4.iter().any(|pool| {
                        pool.parse::<ipnet::IpNet>()
                            .is_ok_and(|net| net.contains(&address.0))
                    })
                {
                    return Err(AdmissionError::Traffic);
                }
                let terminal = path.last().ok_or(AdmissionError::Path)?;
                let role = exit.roles.get(terminal).ok_or(AdmissionError::Permit)?;
                if role.family != exit.family
                    || role.nat != ExitNat::Masquerade
                    || (terminal == &configuration.node_id
                        && configuration.exit_role.as_ref() != Some(role))
                {
                    return Err(AdmissionError::Permit);
                }
                let covered = match &exit.destination {
                    ExitDestinationScope::Internet => crate::is_internet_address(address.0),
                    ExitDestinationScope::Cidr { prefix } => prefix
                        .parse::<ipnet::IpNet>()
                        .is_ok_and(|net| net.contains(&address.0)),
                };
                if !covered {
                    return Err(AdmissionError::Traffic);
                }
            }
            _ => return Err(AdmissionError::Permit),
        }
    }
    if let Some(target) = permit.target
        && path.last() != Some(&target)
    {
        return Err(AdmissionError::Path);
    }
    let external = permit.target.is_none();
    if external && position + 1 == path.len() && !configuration.exit_enabled {
        return Err(AdmissionError::ExitDisabled);
    }
    Ok((position, external))
}

fn admitted_traffic(
    configuration: &NodeConfiguration,
    demand_id: RequestId,
    traffic: &Traffic,
) -> Result<(), AdmissionError> {
    let permit = configuration
        .permits
        .get(&demand_id)
        .ok_or(AdmissionError::Permit)?;
    if !contains(&permit.source_addresses, traffic.source)
        || !valid_traffic(traffic)
        || traffic.protocol != permit.protocol
        || traffic.destination_port != permit.port
    {
        return Err(AdmissionError::Traffic);
    }
    match &permit.target {
        BusinessTarget::Mesh { node } => {
            if !contains(addresses(configuration, *node)?, traffic.destination) {
                return Err(AdmissionError::Traffic);
            }
        }
        BusinessTarget::Ip { address } | BusinessTarget::Domain { address, .. } => {
            if address.0 != traffic.destination
                || contains(&configuration.addresses, traffic.destination)
                || configuration
                    .peers
                    .values()
                    .any(|peer| contains(&peer.addresses, traffic.destination))
            {
                return Err(AdmissionError::Traffic);
            }
        }
    }
    Ok(())
}

/// Admit a no-application-data handshake along the exact complete business
/// path. `reply` selects handshake travel direction only: this predicate never
/// establishes a flow or grants reverse business traffic. Packet admission
/// still requires traffic checks and caller-owned established reply state.
pub fn admit_handshake(
    configuration: &NodeConfiguration,
    demand_id: RequestId,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    reply: bool,
    now_ms: u64,
) -> Result<Hop, AdmissionError> {
    admit_circuit_handshake(
        configuration,
        CircuitAuthorization::Business(demand_id),
        path,
        origin,
        previous,
        reply,
        now_ms,
    )
}

/// Admit an authenticated control frame on its complete tagged circuit path.
/// This grants no IP flow or arbitrary application payload authority.
pub fn admit_circuit_handshake(
    configuration: &NodeConfiguration,
    authorization: CircuitAuthorization,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    reply: bool,
    now_ms: u64,
) -> Result<Hop, AdmissionError> {
    let (position, external) = admitted_path(configuration, authorization, path, origin, now_ms)?;
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
        None if external && !reply => Hop::Exit,
        None => Hop::Deliver,
    })
}

/// Admit a forward packet only along its installed complete path.
pub fn admit_forward(
    configuration: &NodeConfiguration,
    demand_id: RequestId,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    traffic: &Traffic,
    now_ms: u64,
) -> Result<Hop, AdmissionError> {
    let hop = admit_handshake(
        configuration,
        demand_id,
        path,
        origin,
        previous,
        false,
        now_ms,
    )?;
    admitted_traffic(configuration, demand_id, traffic)?;
    Ok(hop)
}

/// `path` and `origin` always describe the original forward business packet.
/// `forward` must come from caller-owned established flow state. This pure
/// predicate checks the exact reversed tuple/path; it cannot prove that state
/// exists and never authorizes creating a new reverse flow.
#[allow(clippy::too_many_arguments)]
pub fn admit_reply(
    configuration: &NodeConfiguration,
    demand_id: RequestId,
    path: &[NodeId],
    origin: NodeId,
    previous: Option<NodeId>,
    forward: &Traffic,
    reply: &Traffic,
    now_ms: u64,
) -> Result<Hop, AdmissionError> {
    let hop = admit_handshake(
        configuration,
        demand_id,
        path,
        origin,
        previous,
        true,
        now_ms,
    )?;
    admitted_traffic(configuration, demand_id, forward)?;
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

/// Check an edge against a selected complete path's actual dial candidates.
/// Remote dial parameters are not published locally; this predicate only checks
/// the edge and candidate ID. Each socket endpoint must also use `admit_carrier`.
pub fn path_carrier(path: &LogicalPath, carrier: &Carrier) -> bool {
    valid_path(&path.nodes)
        && !carrier.listener_key.is_empty()
        && path.nodes.windows(2).any(|edge| {
            (edge[0] == carrier.dialer && edge[1] == carrier.listener)
                || (edge[1] == carrier.dialer && edge[0] == carrier.listener)
        })
        && path
            .dial_candidates
            .get(&carrier.dialer)
            .is_some_and(|keys| {
                keys.contains(&format!("{}/{}", carrier.listener, carrier.listener_key))
            })
}

/// Admit an authenticated adjacency bootstrap, without granting packet or flow
/// authority. At least one live business or diagnostic path must contain this
/// exact edge. Returns the latest matching deadline for bounded idle I/O waits.
pub fn admit_adjacency(
    configuration: &NodeConfiguration,
    peer: NodeId,
    carrier: &Carrier,
    now_ms: u64,
) -> Result<u64, AdmissionError> {
    if peer == configuration.node_id {
        return Err(AdmissionError::Peer);
    }
    let mut expires_at: Option<u64> = None;
    for authorization in configuration
        .permits
        .keys()
        .copied()
        .map(CircuitAuthorization::Business)
        .chain(
            configuration
                .diagnostics
                .keys()
                .copied()
                .map(CircuitAuthorization::Diagnostic),
        )
    {
        let Ok(permit) = circuit_permit(configuration, authorization) else {
            continue;
        };
        if permit.expires_at_ms <= now_ms {
            continue;
        }
        for path in permit.paths {
            if admitted_path(
                configuration,
                authorization,
                &path.nodes,
                permit.source,
                now_ms,
            )
            .is_ok()
                && admit_circuit_carrier(configuration, authorization, &path.nodes, peer, carrier)
                    .is_ok()
            {
                expires_at = Some(expires_at.unwrap_or(0).max(permit.expires_at_ms));
            }
        }
    }
    expires_at.ok_or(AdmissionError::Peer)
}

/// Check the actual local socket pair and its transport against a business
/// path. Compose with current handshake/packet admission for identity, expiry
/// and flow checks; diagnostics and neighbor connectivity grant nothing here.
pub fn admit_carrier(
    configuration: &NodeConfiguration,
    demand_id: RequestId,
    path: &[NodeId],
    peer: NodeId,
    carrier: &Carrier,
) -> Result<(), AdmissionError> {
    admit_circuit_carrier(
        configuration,
        CircuitAuthorization::Business(demand_id),
        path,
        peer,
        carrier,
    )
}

/// Check actual carrier parameters against the exact circuit grant's path.
/// Compose with current handshake admission for identity and expiry checks.
pub fn admit_circuit_carrier(
    configuration: &NodeConfiguration,
    authorization: CircuitAuthorization,
    path: &[NodeId],
    peer: NodeId,
    carrier: &Carrier,
) -> Result<(), AdmissionError> {
    if !configuration.enabled {
        return Err(AdmissionError::Configuration);
    }
    let permit = circuit_permit(configuration, authorization)?;
    if path.first() != Some(&permit.source) {
        return Err(AdmissionError::Permit);
    }
    let selected = permit
        .paths
        .iter()
        .find(|candidate| candidate.nodes == path)
        .ok_or(AdmissionError::Path)?;
    if !path_carrier(selected, carrier) {
        return Err(AdmissionError::Path);
    }
    let local = configuration.node_id;
    if local == carrier.dialer && peer == carrier.listener {
        let key = format!("{}/{}", carrier.listener, carrier.listener_key);
        let dialer = configuration
            .dialers
            .get(&key)
            .ok_or(AdmissionError::Peer)?;
        if dialer.peer != peer
            || dialer.listener_key != carrier.listener_key
            || dialer.parameters.transport() != carrier.protocol
        {
            return Err(AdmissionError::Peer);
        }
    } else if local == carrier.listener && peer == carrier.dialer {
        let listener = configuration
            .listeners
            .get(&carrier.listener_key)
            .ok_or(AdmissionError::Peer)?;
        let (protocol, enabled) = match listener.declaration() {
            Listener::NativeUdp(value) => (TransportProtocol::NativeUdp, value.enabled),
            Listener::NativeTcp(value) => (TransportProtocol::NativeTcp, value.enabled),
            Listener::Vless(value) => (TransportProtocol::Vless, value.enabled),
            Listener::Trojan(value) => (TransportProtocol::Trojan, value.enabled),
        };
        if !enabled || protocol != carrier.protocol {
            return Err(AdmissionError::Peer);
        }
    } else {
        return Err(AdmissionError::Peer);
    }
    Ok(())
}

/// A borrowed circuit-only projection; never grants IP packet admission.
pub struct CircuitPermit<'a> {
    pub source: NodeId,
    /// None denotes an external business destination.
    pub target: Option<NodeId>,
    pub paths: &'a [LogicalPath],
    pub expires_at_ms: u64,
}

pub fn circuit_permit(
    configuration: &NodeConfiguration,
    authorization: CircuitAuthorization,
) -> Result<CircuitPermit<'_>, AdmissionError> {
    match authorization {
        CircuitAuthorization::Business(id) => {
            let permit = configuration
                .permits
                .get(&id)
                .ok_or(AdmissionError::Permit)?;
            if permit.demand_id != id {
                return Err(AdmissionError::Permit);
            }
            Ok(CircuitPermit {
                source: permit.source,
                target: match permit.target {
                    BusinessTarget::Mesh { node } => Some(node),
                    _ => None,
                },
                paths: &permit.paths,
                expires_at_ms: permit.expires_at_ms,
            })
        }
        CircuitAuthorization::Diagnostic(id) => {
            let permit = configuration
                .diagnostics
                .get(&id)
                .ok_or(AdmissionError::Permit)?;
            if permit.observation_id != id
                || permit.replies != DiagnosticReplyPolicy::AuthenticatedExactReversePath
            {
                return Err(AdmissionError::Permit);
            }
            Ok(CircuitPermit {
                source: permit.source,
                target: Some(permit.target),
                paths: &permit.paths,
                expires_at_ms: permit.expires_at_ms,
            })
        }
    }
}
