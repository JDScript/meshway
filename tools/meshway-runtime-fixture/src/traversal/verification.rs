//! Original offer authority and present netmap are separate proofs. A
//! content update never changes the immutable installation provenance.
use super::{
    Event, Observations,
    installation::{Installation, Installations},
};
use crate::{Result, signals::Role};
use meshway_model::{
    InstanceId, NetmapRef, NodeId, Sequence, UdpTraversalId,
    observations::{EndpointKind, LinkAcquisition},
    state::{ApplyPhase, EndpointSource, LinkPhase, ResourceRef, SocketNetwork},
};
use meshway_protocol::{
    connectivity::*,
    state::{Freshness, StateView, VisibleReport},
};
use serde::Serialize;
use std::collections::BTreeMap;

pub(super) struct Current {
    pub view: StateView,
    pub installations: Installations,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Continuity {
    attempt_id: UdpTraversalId,
    installation: Installation,
    current_instance: InstanceId,
    current_netmap: NetmapRef,
    current_report_sequence: Sequence,
    current_socket_generation: Sequence,
    current_resource: ResourceRef,
}

pub(super) fn verify(
    observations: &Observations,
    current: &BTreeMap<NodeId, Current>,
    observer: &str,
    nodes: &BTreeMap<Role, NodeId>,
) -> Result<(Vec<UdpTraversalOffer>, Vec<Continuity>)> {
    if observations.failed {
        return Err("traversal HTTP audit incomplete".into());
    }
    let mut offers = Vec::new();
    let mut continuity = Vec::new();
    for (left, right) in [(Role::A, Role::R), (Role::R, Role::B)] {
        let before = offers.len();
        for event in &observations.events {
            let Event::Prepare {
                request,
                response: PrepareUdpTraversalResponse::Offer { offer },
            } = event
            else {
                continue;
            };
            let local = &offer.local.candidate.socket;
            let peer = &offer.peer.candidate.socket;
            if !((local.node_id == nodes[&left] && peer.node_id == nodes[&right])
                || (local.node_id == nodes[&right] && peer.node_id == nodes[&left]))
                || !original_request(request, offer)
            {
                continue;
            }
            let mut proofs = Vec::new();
            for (participant, other) in [(&offer.local, &offer.peer), (&offer.peer, &offer.local)] {
                if !original_candidate(observations, participant, offer, observer) {
                    break;
                }
                let Some(accepted_at) = accepted(observations, participant, offer) else {
                    break;
                };
                let Some(proof) = current_channel(current, participant, other, offer, accepted_at)
                else {
                    break;
                };
                proofs.push(proof);
            }
            if proofs.len() == 2
                && active(observations, offer)
                && proofs[0].installation.channel.link_id == proofs[1].installation.channel.link_id
            {
                offers.push(offer.clone());
                continuity.extend(proofs);
            }
        }
        if offers.len() == before {
            return Err("missing original offer authority, exact authenticated installation, or current channel continuity".into());
        }
    }
    Ok((offers, continuity))
}

fn original_request(request: &PrepareUdpTraversalRequest, offer: &UdpTraversalOffer) -> bool {
    let local = &offer.local.candidate.socket;
    let peer = &offer.peer.candidate.socket;
    offer.compute_digest().ok() == Some(offer.digest)
        && request.local_candidate == offer.local.candidate
        && request.peer_candidate == offer.peer.candidate
        && request.instance == local.instance
        && request.netmap == local.netmap
        && request.state_epoch == offer.state_epoch
        && request.request_id == offer.request_id
        && request.peer_id == peer.node_id
        && request.carrier == offer.carrier
        && offer.carrier.protocol == meshway_transport_spec::TransportProtocol::NativeUdp
        && local.instance.instance_id != peer.instance.instance_id
        && offer.expires_at_ms <= request.expires_at_ms
}

fn original_candidate(
    observations: &Observations,
    participant: &UdpTraversalParticipant,
    offer: &UdpTraversalOffer,
    observer: &str,
) -> bool {
    let socket = &participant.candidate.socket;
    participant.source == EndpointKind::Stun
        && observations.events.iter().any(|event| {
            let Event::Publication { instance_id, applied, sockets, .. } = event else { return false; };
            let Some(published) = sockets.get(&socket.socket_id) else { return false; };
            *instance_id == socket.instance.instance_id
                && published.netmap.as_ref().or(applied.as_ref()) == Some(&socket.netmap)
                && published.generation == socket.socket_generation && published.resource == socket.resource
                && published.network == SocketNetwork::Udp
                && published.bound_address == "0.0.0.0:35000".parse::<std::net::SocketAddr>().expect("fixed address")
                && published.endpoints.iter().any(|candidate| {
                    candidate.address == participant.address
                        && matches!(&candidate.origin, EndpointSource::Stun { observer: actual } if actual == observer)
                        && candidate.observed_at < candidate.expires_at
                        && candidate.observed_at.timestamp_millis() <= i64::try_from(offer.expires_at_ms).unwrap_or(i64::MIN)
                        && candidate.expires_at.timestamp_millis() >= i64::try_from(offer.expires_at_ms).unwrap_or(i64::MAX)
                        && CandidateReference::from_observation(socket.clone(), published.network, published.bound_address, candidate).ok().as_ref() == Some(&participant.candidate)
                })
        })
        && observations.events.iter().any(|event| matches!(event,
            Event::Query { request, response: QueryUdpTraversalResponse::Current { stun_servers, .. } }
            if request.instance == socket.instance && stun_servers.as_slice().iter().any(|server| server.id == "fixture")))
}

fn acceptance(
    event: &Event,
    participant: &UdpTraversalParticipant,
    offer: &UdpTraversalOffer,
) -> Option<(u64, bool)> {
    let Event::Accept {
        request,
        response,
        received_at_ms,
    } = event
    else {
        return None;
    };
    let (accepted, active) = match response {
        AcceptUdpTraversalResponse::Waiting { offer } => (offer, false),
        AcceptUdpTraversalResponse::Active { offer } => (offer, true),
        _ => return None,
    };
    let socket = &participant.candidate.socket;
    (accepted == offer
        && request.instance == socket.instance
        && request.netmap == socket.netmap
        && request.state_epoch == offer.state_epoch
        && request.version == offer.version
        && request.digest == offer.digest
        && *received_at_ms < offer.expires_at_ms)
        .then_some((*received_at_ms, active))
}

fn accepted(
    observations: &Observations,
    participant: &UdpTraversalParticipant,
    offer: &UdpTraversalOffer,
) -> Option<u64> {
    observations
        .events
        .iter()
        .filter_map(|event| acceptance(event, participant, offer).map(|(at, _)| at))
        .min()
}

fn active(observations: &Observations, offer: &UdpTraversalOffer) -> bool {
    [&offer.local, &offer.peer].iter().any(|participant| {
        observations
            .events
            .iter()
            .any(|event| acceptance(event, participant, offer).is_some_and(|(_, active)| active))
    })
}

fn current_channel(
    current: &BTreeMap<NodeId, Current>,
    participant: &UdpTraversalParticipant,
    other: &UdpTraversalParticipant,
    offer: &UdpTraversalOffer,
    accepted_at: u64,
) -> Option<Continuity> {
    let socket = &participant.candidate.socket;
    let context = current.get(&socket.node_id)?;
    let peer_context = current.get(&other.candidate.socket.node_id)?;
    let identity = context.installations.status.identity.as_ref()?;
    let peer_identity = peer_context.installations.status.identity.as_ref()?;
    let instance = context.installations.status.instance.as_ref()?;
    let state = context.view.nodes.get(&socket.node_id)?;
    let VisibleReport::Full { report } = state.report.as_ref()? else {
        return None;
    };
    let applied = report.configuration.applied.as_ref()?;
    let physical = report.sockets.get(&socket.socket_id)?;
    if identity.node_id != socket.node_id
        || identity.network_id != offer.network_id
        || peer_identity.node_id != other.candidate.socket.node_id
        || peer_identity.network_id != offer.network_id
        || instance.instance_id != socket.instance.instance_id
        || instance.generation != socket.instance.generation
        || context.installations.status.applied.as_ref() != Some(applied)
        || context.installations.status.saved.as_ref() != Some(applied)
        || context
            .installations
            .status
            .netmap
            .as_ref()
            .map(|lease| &lease.netmap)
            != Some(applied)
        || !context.installations.status.authenticated
        || !context.installations.status.desired_up
        || context.installations.status.phase != meshway_ipc::Phase::Running
        || context.installations.status.data_plane != meshway_ipc::DataPlane::Ready
        || context.view.network_id != offer.network_id
        || context.view.state_epoch != offer.state_epoch
        || context.view.configuration_revision != socket.netmap.revision
        || state.freshness != Freshness::Current
        || state.instance_id != Some(socket.instance.instance_id)
        || state
            .report_expires_at
            .is_none_or(|expiry| expiry <= context.view.as_of)
        || state.observed_at.is_none_or(|at| at > context.view.as_of)
        || report.configuration.saved.as_ref() != Some(applied)
        || applied.revision != socket.netmap.revision
        || report
            .configuration
            .progress
            .as_ref()
            .is_none_or(|progress| {
                progress.target != *applied
                    || progress.phase != ApplyPhase::Applied
                    || progress.problem.is_some()
            })
        || physical.netmap.as_ref().or(Some(applied)) != Some(applied)
        || physical.generation != socket.socket_generation
        || physical.resource != socket.resource
        || physical.network != SocketNetwork::Udp
        || physical.bound_address != "0.0.0.0:35000".parse::<std::net::SocketAddr>().ok()?
    {
        return None;
    }
    for installation in &context.installations.installed {
        let channel = &installation.channel;
        if installation.node != socket.node_id
            || installation.network != offer.network_id
            || installation.instance != socket.instance
            || installation.netmap != socket.netmap
            || installation.control_key != identity.keys.control
            || installation.dataplane_key != identity.keys.dataplane
            || installation.peer_control_key != peer_identity.keys.control
            || installation.peer_dataplane_key != peer_identity.keys.dataplane
            || installation.carrier != offer.carrier
            || channel.transport != offer.carrier.protocol
            || installation.observed_at_ms < accepted_at
            || installation.observed_at_ms >= offer.expires_at_ms
            || i64::try_from(installation.observed_at_ms).ok()?
                > state.observed_at?.timestamp_millis()
            || channel.socket_id.to_string() != socket.socket_id
            || channel.peer_node_id != other.candidate.socket.node_id
            || channel.local_address.get() != physical.bound_address
            || channel.remote_address.get() != other.address.parse::<std::net::SocketAddr>().ok()?
            || channel.acquisition
                != (LinkAcquisition::CoordinatedUdp {
                    attempt_id: offer.attempt_id,
                    endpoint_source: EndpointKind::Stun,
                })
        {
            continue;
        }
        if report
            .links
            .get(&channel.peer_node_id)
            .is_some_and(|links| {
                links.iter().any(|link| {
                    link.link_id == channel.link_id.to_string()
                        && link.socket_id == socket.socket_id
                        && link.initiator_node_id == offer.carrier.dialer
                        && link.phase == LinkPhase::Authenticated
                        && link.business_dependencies > 0
                        && link.transport == offer.carrier.protocol.as_str()
                        && link.observed_at <= context.view.as_of
                        && link.observed_at.timestamp_millis()
                            >= i64::try_from(installation.observed_at_ms).unwrap_or(i64::MAX)
                        && link.expires_at > context.view.as_of
                })
            })
        {
            return Some(Continuity {
                attempt_id: offer.attempt_id,
                installation: installation.clone(),
                current_instance: socket.instance.instance_id,
                current_netmap: *applied,
                current_report_sequence: state.report_sequence?,
                current_socket_generation: physical.generation,
                current_resource: physical.resource.clone(),
            });
        }
    }
    None
}
