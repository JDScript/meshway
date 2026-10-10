//! Circuit-level admission on the installed netmap: the exact path, the
//! path authorization digest a handshake binds, the hop and carrier checks.
use super::{CircuitGrant, Error, codec::Context};
use crate::{
    authority::{View, netmap_addresses},
    netmap::{Decisions, Outcome, Purpose, path_authorization},
};
use meshway_configuration::{
    execution::{BusinessProtocol, LogicalPath},
    netmap::{CompiledNetmap, ServiceSet},
};
use meshway_model::{Digest, NodeId};
use meshway_policy::{
    admission::{
        Carrier, Hop,
        netmap::{self as admission, Leased},
        path_carrier,
    },
    compile::netmap::{Decision, Target},
};
use std::{net::IpAddr, sync::Arc};

/// Result of admitting one circuit frame direction on the current content.
pub(super) struct Admitted {
    pub hop: Hop,
    /// Digest the handshake binds; compared with the wire context.
    pub authorization: Digest,
    /// The exact admitted path, retained by the circuit.
    pub selected: Arc<LogicalPath>,
}

/// The circuit-level decision of a Mesh or exit pair at an endpoint. Its
/// service set is empty: the circuit binds no service, every packet is
/// admitted against the packet's own decision.
fn circuit_decision(
    netmap: &CompiledNetmap,
    origin: NodeId,
    target: &Target,
    paths: &[LogicalPath],
    selection: Option<Digest>,
) -> Decision {
    let destination = match target {
        Target::Mesh { node } => netmap_addresses(&netmap.netmap, *node)
            .and_then(|a| a.ipv4.map(IpAddr::V4).or(a.ipv6.map(IpAddr::V6))),
        Target::Exit { .. } => None,
    }
    .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    Decision {
        source: origin,
        destination,
        target: target.clone(),
        services: ServiceSet::default(),
        paths: paths.to_vec(),
        selection,
    }
}

fn mesh_address(netmap: &CompiledNetmap, node: NodeId) -> Option<IpAddr> {
    netmap_addresses(&netmap.netmap, node)
        .and_then(|a| a.ipv4.map(IpAddr::V4).or(a.ipv6.map(IpAddr::V6)))
}

pub(super) fn paths(
    decisions: &mut Decisions,
    netmap: &CompiledNetmap,
    origin: NodeId,
    grant: &CircuitGrant,
) -> Result<Arc<Vec<LogicalPath>>, Error> {
    if grant.purpose == Purpose::Dns {
        let service = decisions
            .dns_service(netmap, origin)
            .map_err(|_| Error::Policy)?;
        if grant.target != (Target::Mesh { node: service.exit }) {
            return Err(Error::Policy);
        }
        Ok(Arc::new(vec![service.path.clone()]))
    } else {
        decisions
            .transit(netmap, origin, &grant.target)
            .map_err(|_| Error::Policy)
    }
}

/// Admit a frame of a netmap circuit. Endpoints use `admit_handshake` (or
/// `admit_observation` for diagnostics), relays `admit_transit`.
#[allow(clippy::too_many_arguments)]
pub(super) fn admit_netmap(
    decisions: &mut Decisions,
    netmap: &CompiledNetmap,
    leased: Leased<'_>,
    now_ms: u64,
    purpose: Purpose,
    target: &Target,
    path: &[NodeId],
    previous: Option<NodeId>,
    reply: bool,
) -> Result<Admitted, Error> {
    let origin = *path.first().ok_or(Error::Context)?;
    let local = netmap.netmap.node_id;
    let network = netmap.netmap.network_id;
    let paths = paths(
        decisions,
        netmap,
        origin,
        &CircuitGrant {
            purpose,
            target: target.clone(),
        },
    )?;
    let selected = paths
        .iter()
        .find(|candidate| candidate.nodes == path)
        .ok_or(Error::Policy)?;
    let dns = if purpose == Purpose::Dns {
        Some(
            decisions
                .dns_service(netmap, origin)
                .map_err(|_| Error::Policy)?,
        )
    } else {
        None
    };
    let selection = if let Some(service) = &dns {
        Some(meshway_configuration::digest(service.as_ref()).map_err(|_| Error::Policy)?)
    } else {
        match target {
            Target::Exit { .. } => Some(
                decisions
                    .selection(netmap, origin)
                    .map_err(|_| Error::Policy)?,
            ),
            Target::Mesh { .. } => None,
        }
    };
    let authorization = path_authorization(netmap, network, purpose, target, selected, selection)?;
    let endpoint = path.first() == Some(&local) || path.last() == Some(&local);
    let hop = if let Some(service) = &dns {
        admission::admit_dns(leased, service, path, origin, previous, reply, now_ms)
    } else if !endpoint {
        admission::admit_transit_paths(leased, &paths, origin, path, previous, reply, now_ms)
    } else if purpose == Purpose::Diagnostic {
        let Target::Mesh { node } = target else {
            return Err(Error::Policy);
        };
        let address = mesh_address(netmap, *node).ok_or(Error::Policy)?;
        let Outcome::Permitted(decision) = decisions
            .decide(netmap, origin, address, (BusinessProtocol::IcmpEcho, None))
            .map_err(|_| Error::Policy)?
        else {
            return Err(Error::Policy);
        };
        admission::admit_observation(
            leased,
            &decision,
            path,
            origin,
            previous,
            reply,
            (BusinessProtocol::IcmpEcho, None),
            now_ms,
        )
    } else {
        admission::admit_handshake(
            leased,
            &circuit_decision(netmap, origin, target, &paths, selection),
            path,
            origin,
            previous,
            reply,
            now_ms,
        )
    }
    .map_err(|_| Error::Policy)?;
    Ok(Admitted {
        hop,
        authorization,
        selected: Arc::new(selected.clone()),
    })
}

/// Admit one handshake-direction frame of a circuit grant.
pub(super) fn admit(
    decisions: &mut Decisions,
    view: &View<'_>,
    grant: &CircuitGrant,
    path: &[NodeId],
    previous: Option<NodeId>,
    reply: bool,
) -> Result<Admitted, Error> {
    admit_netmap(
        decisions,
        view.netmap(),
        view.leased(),
        view.now_ms,
        grant.purpose,
        &grant.target,
        path,
        previous,
        reply,
    )
}

/// The exact path a live circuit was admitted on.
pub(super) fn selected<'a>(
    context: &Context,
    retained: Option<&'a LogicalPath>,
) -> Result<&'a LogicalPath, Error> {
    retained
        .filter(|path| path.nodes == context.path)
        .ok_or(Error::Context)
}

/// An outgoing carrier: carriers were admitted as adjacencies when
/// installed, so only path membership remains.
pub(super) fn outgoing(
    selected: Option<&LogicalPath>,
    next: NodeId,
    carrier: &Carrier,
) -> Result<(), Error> {
    let selected = selected.ok_or(Error::Context)?;
    if (carrier.dialer != next && carrier.listener != next) || !path_carrier(selected, carrier) {
        return Err(Error::Policy);
    }
    Ok(())
}

/// An authenticated incoming carrier from `previous`.
pub(super) fn incoming(
    decisions: &mut Decisions,
    view: &View<'_>,
    selected: &LogicalPath,
    previous: NodeId,
    carrier: &Carrier,
) -> Result<(), Error> {
    if !path_carrier(selected, carrier) || !decisions.adjacency(view.netmap(), previous, carrier) {
        return Err(Error::Policy);
    }
    Ok(())
}

/// Current adjacency admission of a host-observed carrier.
pub(super) fn adjacency(
    decisions: &mut Decisions,
    view: &View<'_>,
    peer: NodeId,
    carrier: &Carrier,
) -> bool {
    decisions.adjacency(view.netmap(), peer, carrier)
}
