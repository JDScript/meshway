//! Pure business-packet predicates over an authenticated, installed target.
//!
//! The caller must bind `configuration.network_id` and `configuration.node_id`
//! to its own authenticated network/node, check credentials and lease, and pass
//! the authenticated arrival peer. Neither a RequestId nor these predicates
//! prove authentication or establish a flow. Replies require the caller's own
//! established forward state, never a forward tuple supplied by the wire.
use crate::compile::{MAX_EXIT_CANDIDATES, MAX_HOPS, MAX_PATHS};
use meshway_configuration::{NodeAddresses, execution::*};
use meshway_model::{BoundNodeKey, KeyAlgorithm, NodeId, RequestId};
use meshway_transport_spec::Port;
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
    #[error("business permit is absent or inconsistent")]
    Permit,
    #[error("business permit has expired")]
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

/// Validate the complete original forward path and tuple, independent of the
/// direction of this packet. No decisions, diagnostic grants or neighbors can
/// substitute for the current exact business permit.
fn admitted_path(
    configuration: &NodeConfiguration,
    demand_id: RequestId,
    path: &[NodeId],
    origin: NodeId,
    traffic: &Traffic,
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
    let permit = configuration
        .permits
        .get(&demand_id)
        .ok_or(AdmissionError::Permit)?;
    if permit.demand_id != demand_id || permit.source != origin {
        return Err(AdmissionError::Permit);
    }
    if now_ms >= permit.expires_at_ms {
        return Err(AdmissionError::Expired);
    }
    if path.is_empty()
        || path.len() > MAX_HOPS
        || path.first() != Some(&origin)
        || path
            .iter()
            .enumerate()
            .any(|(index, node)| path[..index].contains(node))
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
    let source = addresses(configuration, origin)?;
    if source != &permit.source_addresses
        || !contains(source, traffic.source)
        || !valid_traffic(traffic)
        || traffic.protocol != permit.protocol
        || traffic.destination_port != permit.port
    {
        return Err(AdmissionError::Traffic);
    }
    let external = match &permit.target {
        BusinessTarget::Mesh { node } => {
            if path.last() != Some(node)
                || !contains(addresses(configuration, *node)?, traffic.destination)
            {
                return Err(AdmissionError::Traffic);
            }
            false
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
            true
        }
    };
    if external && position + 1 == path.len() && !configuration.exit_enabled {
        return Err(AdmissionError::ExitDisabled);
    }
    Ok((position, external))
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
    let (position, external) =
        admitted_path(configuration, demand_id, path, origin, traffic, now_ms)?;
    let expected = position.checked_sub(1).map(|index| path[index]);
    if previous != expected {
        return Err(AdmissionError::Peer);
    }
    Ok(match path.get(position + 1) {
        Some(next) => Hop::Forward(*next),
        None if external => Hop::Exit,
        None => Hop::Deliver,
    })
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
    let (position, _) = admitted_path(configuration, demand_id, path, origin, forward, now_ms)?;
    if previous != path.get(position + 1).copied() {
        return Err(AdmissionError::Peer);
    }
    if !valid_traffic(reply)
        || reply.protocol != forward.protocol
        || reply.source != forward.destination
        || reply.destination != forward.source
        || reply.source_port != forward.destination_port
        || reply.destination_port != forward.source_port
    {
        return Err(AdmissionError::Traffic);
    }
    Ok(match position.checked_sub(1) {
        Some(index) => Hop::Forward(path[index]),
        None => Hop::Deliver,
    })
}
