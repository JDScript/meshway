//! Circuit-level admission of either authority kind: the exact path, the
//! authorization digest a handshake binds, the hop and carrier checks.
use super::{CircuitAuthorization, CircuitGrant, Error, codec::Context};
use crate::{
    authority::{Auth, View, netmap_addresses},
    netmap::{Decisions, Outcome, Purpose, path_authorization},
};
use meshway_authority::ExecutionSnapshot;
use meshway_configuration::{
    execution::{BusinessProtocol, LogicalPath, NodeConfiguration},
    netmap::{CompiledNetmap, ServiceSet},
};
use meshway_model::{Digest, NodeId};
use meshway_policy::{
    admission::{
        Carrier, Hop, admit_circuit_carrier, admit_circuit_handshake, circuit_permit,
        netmap::{self as admission, Leased},
        path_carrier,
    },
    compile::netmap::{Decision, LocalCompiler, Target},
};
use std::{net::IpAddr, sync::Arc};

/// Result of admitting one circuit frame direction on the current content.
pub(super) struct Admitted {
    pub hop: Hop,
    /// Digest the handshake binds; compared with the wire context.
    pub authorization: Digest,
    /// Netmap: the exact admitted path, retained by the circuit.
    pub selected: Option<Arc<LogicalPath>>,
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
    let paths = decisions
        .transit(netmap, origin, target)
        .map_err(|_| Error::Policy)?;
    let selected = paths
        .iter()
        .find(|candidate| candidate.nodes == path)
        .ok_or(Error::Policy)?;
    let selection = match target {
        Target::Exit { .. } => Some(
            decisions
                .selection(netmap, origin)
                .map_err(|_| Error::Policy)?,
        ),
        Target::Mesh { .. } => None,
    };
    let authorization = path_authorization(netmap, network, purpose, target, selected, selection)?;
    let endpoint = path.first() == Some(&local) || path.last() == Some(&local);
    let hop = if !endpoint {
        admission::admit_transit(
            leased,
            &LocalCompiler::new(&netmap.netmap),
            origin,
            target,
            path,
            previous,
            reply,
            now_ms,
        )
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
        selected: Some(Arc::new(selected.clone())),
    })
}

/// Admit one handshake-direction frame of either grant kind.
pub(super) fn admit(
    decisions: &mut Decisions,
    snapshot: &ExecutionSnapshot,
    view: &View<'_>,
    grant: &CircuitGrant,
    path: &[NodeId],
    previous: Option<NodeId>,
    reply: bool,
) -> Result<Admitted, Error> {
    let origin = *path.first().ok_or(Error::Context)?;
    match (grant, view.auth) {
        (CircuitGrant::Legacy(authorization), Auth::Legacy(cfg)) => {
            source_instance(snapshot, cfg, *authorization)?;
            let hop = admit_circuit_handshake(
                cfg,
                *authorization,
                path,
                origin,
                previous,
                reply,
                view.now_ms,
            )
            .map_err(|_| Error::Policy)?;
            Ok(Admitted {
                hop,
                authorization: digest(cfg, *authorization)?,
                selected: None,
            })
        }
        (CircuitGrant::Netmap { purpose, target }, Auth::Netmap { netmap, .. }) => admit_netmap(
            decisions,
            netmap,
            view.leased().ok_or(Error::Authority)?,
            view.now_ms,
            *purpose,
            target,
            path,
            previous,
            reply,
        ),
        _ => Err(Error::Context),
    }
}

/// The exact path a live circuit was admitted on.
pub(super) fn selected<'a>(
    view: &View<'a>,
    context: &Context,
    retained: Option<&'a LogicalPath>,
) -> Result<&'a LogicalPath, Error> {
    match (&context.grant, view.auth) {
        (CircuitGrant::Legacy(authorization), Auth::Legacy(cfg)) => {
            circuit_permit(cfg, *authorization)
                .map_err(|_| Error::Context)?
                .paths
                .iter()
                .find(|path| path.nodes == context.path)
                .ok_or(Error::Context)
        }
        (CircuitGrant::Netmap { .. }, Auth::Netmap { .. }) => retained
            .filter(|path| path.nodes == context.path)
            .ok_or(Error::Context),
        _ => Err(Error::Context),
    }
}

/// An outgoing carrier: legacy checks its permit path; netmap carriers were
/// admitted as adjacencies when installed, so only path membership remains.
pub(super) fn outgoing(
    view: &View<'_>,
    context: &Context,
    selected: Option<&LogicalPath>,
    next: NodeId,
    carrier: &Carrier,
) -> Result<(), Error> {
    match (&context.grant, view.auth) {
        (CircuitGrant::Legacy(authorization), Auth::Legacy(cfg)) => {
            admit_circuit_carrier(cfg, *authorization, &context.path, next, carrier)
                .map_err(|_| Error::Policy)
        }
        (CircuitGrant::Netmap { .. }, Auth::Netmap { .. }) => {
            let selected = selected.ok_or(Error::Context)?;
            if (carrier.dialer != next && carrier.listener != next)
                || !path_carrier(selected, carrier)
            {
                return Err(Error::Policy);
            }
            Ok(())
        }
        _ => Err(Error::Context),
    }
}

/// An authenticated incoming carrier from `previous`.
pub(super) fn incoming(
    decisions: &mut Decisions,
    view: &View<'_>,
    context: &Context,
    selected: &LogicalPath,
    previous: NodeId,
    carrier: &Carrier,
) -> Result<(), Error> {
    match (&context.grant, view.auth) {
        (CircuitGrant::Legacy(authorization), Auth::Legacy(cfg)) => {
            admit_circuit_carrier(cfg, *authorization, &context.path, previous, carrier)
                .map_err(|_| Error::Policy)
        }
        (CircuitGrant::Netmap { .. }, Auth::Netmap { netmap, .. }) => {
            if !path_carrier(selected, carrier) || !decisions.adjacency(netmap, previous, carrier) {
                return Err(Error::Policy);
            }
            Ok(())
        }
        _ => Err(Error::Context),
    }
}

/// Current adjacency admission of a host-observed carrier.
pub(super) fn adjacency(
    decisions: &mut Decisions,
    view: &View<'_>,
    peer: NodeId,
    carrier: &Carrier,
) -> bool {
    match view.auth {
        Auth::Legacy(cfg) => {
            meshway_policy::admission::admit_adjacency(cfg, peer, carrier, view.now_ms).is_ok()
        }
        Auth::Netmap { netmap, .. } => decisions.adjacency(netmap, peer, carrier),
    }
}

pub(super) fn digest(
    cfg: &NodeConfiguration,
    authorization: CircuitAuthorization,
) -> Result<Digest, Error> {
    match authorization {
        CircuitAuthorization::Business(id) => {
            meshway_policy::permit_authorization_digest(cfg.permits.get(&id).ok_or(Error::Context)?)
        }
        CircuitAuthorization::Diagnostic(id) => meshway_policy::diagnostic_authorization_digest(
            cfg.diagnostics.get(&id).ok_or(Error::Context)?,
        ),
    }
    .map_err(|_| Error::Context)
}

pub(super) fn source_instance(
    snapshot: &ExecutionSnapshot,
    cfg: &NodeConfiguration,
    authorization: CircuitAuthorization,
) -> Result<(), Error> {
    if let CircuitAuthorization::Diagnostic(id) = authorization {
        let permit = cfg.diagnostics.get(&id).ok_or(Error::Context)?;
        if permit.source == cfg.node_id
            && Some(permit.instance_id)
                != snapshot
                    .instance_binding()
                    .map(|instance| instance.instance_id)
        {
            return Err(Error::Context);
        }
    }
    Ok(())
}

/// Legacy retention of a circuit across a publication replacement.
pub(super) fn unchanged(
    old: &NodeConfiguration,
    new: &NodeConfiguration,
    authorization: CircuitAuthorization,
    path: &[NodeId],
    now_ms: u64,
) -> Result<bool, Error> {
    if let CircuitAuthorization::Business(id) = authorization {
        return crate::flow::authorization_unchanged(old, new, id, path, now_ms)
            .map_err(|_| Error::Context);
    }
    let (Ok(before), Ok(after)) = (
        circuit_permit(old, authorization),
        circuit_permit(new, authorization),
    ) else {
        return Ok(false);
    };
    if now_ms >= before.expires_at_ms
        || now_ms >= after.expires_at_ms
        || digest(old, authorization)? != digest(new, authorization)?
        || path
            .iter()
            .any(|node| *node != old.node_id && old.peers.get(node) != new.peers.get(node))
    {
        return Ok(false);
    }
    let Some(selected) = before
        .paths
        .iter()
        .find(|candidate| candidate.nodes == path)
    else {
        return Ok(false);
    };
    let position = path
        .iter()
        .position(|node| *node == new.node_id)
        .ok_or(Error::Context)?;
    if admit_circuit_handshake(
        new,
        authorization,
        path,
        before.source,
        position.checked_sub(1).map(|i| path[i]),
        false,
        now_ms,
    )
    .is_err()
    {
        return Ok(false);
    }
    Ok(crate::flow::carrier_bindings_unchanged(old, new, selected))
}
