//! Observation decisions on one source netmap: the single implementation used
//! by the Server when a task is created or renewed, by the source Node before
//! it probes, and by Control when it reports a result.
use super::*;
use meshway_configuration::execution::{BusinessProtocol, ObservationServiceScope};
use std::net::IpAddr;

/// Complete paths an observation of `target` may measure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationPaths {
    /// Observable candidate paths; every allowed path when none is observable.
    pub paths: Vec<Vec<NodeId>>,
    /// Whether every Node of `paths` declares path observation.
    pub observable: bool,
}

/// The decision does not permit the service toward `target`, or an explicit
/// path is not among the permitted ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservationDenied;

/// Decide an observation of `service` from the netmap's own Node to `target`
/// exactly like a first packet of that service, optionally pinned to an
/// explicit complete path. Denied and unobservable are distinct: an allowed
/// pair without an observable path returns `observable: false`.
pub fn observation_paths(
    netmap: &std::sync::Arc<NodeNetmap>,
    target: NodeId,
    service: &ObservationServiceScope,
    explicit: Option<&[NodeId]>,
) -> Result<ObservationPaths, ObservationDenied> {
    if target == netmap.node_id {
        return Err(ObservationDenied);
    }
    let addresses = &netmap
        .projection
        .configuration
        .nodes
        .get(&target)
        .ok_or(ObservationDenied)?
        .addresses;
    let destination = addresses
        .ipv4
        .map(IpAddr::V4)
        .or(addresses.ipv6.map(IpAddr::V6))
        .ok_or(ObservationDenied)?;
    let (protocol, port) = match service {
        ObservationServiceScope::Tcp { port } => (BusinessProtocol::Tcp, Some(*port)),
        ObservationServiceScope::Udp { port } => (BusinessProtocol::Udp, Some(*port)),
        ObservationServiceScope::IcmpEcho => (BusinessProtocol::IcmpEcho, None),
    };
    let decision = match compile_local(netmap, netmap.node_id, destination, (protocol, port)) {
        Ok(LocalDecision::Permitted(decision)) => decision,
        _ => return Err(ObservationDenied),
    };
    if decision.target != (Target::Mesh { node: target })
        || !decision.services.contains(protocol, port)
    {
        return Err(ObservationDenied);
    }
    let mut paths: Vec<Vec<NodeId>> = decision.paths.into_iter().map(|p| p.nodes).collect();
    if let Some(path) = explicit {
        if !paths.iter().any(|candidate| candidate == path) {
            return Err(ObservationDenied);
        }
        paths = vec![path.to_vec()];
    }
    if paths.is_empty() {
        return Err(ObservationDenied);
    }
    let inventory = &netmap.projection.inventory;
    let supported: Vec<_> = paths
        .iter()
        .filter(|path| observable(inventory, path))
        .cloned()
        .collect();
    Ok(if supported.is_empty() {
        ObservationPaths {
            paths,
            observable: false,
        }
    } else {
        ObservationPaths {
            paths: supported,
            observable: true,
        }
    })
}
