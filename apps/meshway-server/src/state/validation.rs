use super::{
    CLOCK_SKEW_SECONDS, ConfigurationAdmission, MAX_REPORT_BYTES, REPORT_TTL_SECONDS,
    ReportAdmission,
};
use crate::error::Error;
use chrono::{DateTime, Duration, Utc};
use meshway_model::NodeId;
use meshway_model::state::*;
use meshway_protocol::state::StateReport;
use std::{collections::BTreeSet, net::SocketAddr};

fn text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
fn time(observed: DateTime<Utc>, expires: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    observed <= now + Duration::seconds(CLOCK_SKEW_SECONDS)
        && expires > observed
        && expires - observed <= Duration::minutes(10)
}
pub(super) fn report(
    admission: &ReportAdmission,
    report: &StateReport,
    now: DateTime<Utc>,
) -> Result<(), Error> {
    if admission.instance_id != report.instance_id || admission.lease_expires_at <= now {
        return Err(Error::InvalidSession);
    }
    if report.sequence.get() == 0
        || report.observed_at > now + Duration::seconds(CLOCK_SKEW_SECONDS)
        || report.observed_at + Duration::seconds(REPORT_TTL_SECONDS) <= now
    {
        return Err(Error::InvalidRequest);
    }
    if serde_json::to_vec(report)
        .map_err(|_| Error::encode("state report"))?
        .len()
        > MAX_REPORT_BYTES
    {
        return Err(Error::StateLimit);
    }
    let state = &report.state;
    if [
        &state.system.hostname,
        &state.system.os,
        &state.system.arch,
        &state.system.agent_version,
    ]
    .iter()
    .any(|s| !text(s))
        || state.capabilities.len() > 64
        || state.listeners.len() > 64
        || state.sockets.len() > 256
        || state.links.len() > 256
        || state.paths.len() > 256
        || state.probes.len() > 64
        || state.problems.len() > 64
    {
        return Err(Error::StateLimit);
    }
    for reference in state
        .configuration
        .saved
        .iter()
        .chain(state.configuration.applied.iter())
        .chain(state.configuration.progress.iter().map(|p| &p.target))
    {
        if !admission.configurations.contains(reference) {
            return Err(Error::InvalidRequest);
        }
    }
    for (key, capability) in &state.capabilities {
        match capability {
            Capability::Transport { roles, streams }
                if key.starts_with("transport:")
                    && text(key)
                    && !roles.is_empty()
                    && roles.len() <= 2
                    && !streams.is_empty()
                    && streams.len() <= 3 =>
            {
                if roles
                    .iter()
                    .enumerate()
                    .any(|(i, role)| roles[..i].contains(role))
                    || streams
                        .iter()
                        .enumerate()
                        .any(|(i, stream)| streams[..i].contains(stream))
                {
                    return Err(Error::InvalidRequest);
                }
            }
            Capability::Feature { version }
                if key.starts_with("feature:") && text(key) && *version > 0 => {}
            _ => return Err(Error::InvalidRequest),
        }
    }
    // Validate all facts against their known immutable context first. Scope
    // rejection is deferred so malformed facts never become a benign retry.
    let mut scope_changed = false;
    if !state.links.is_empty() || !state.paths.is_empty() {
        let reference = state
            .configuration
            .applied
            .as_ref()
            .ok_or(Error::InvalidRequest)?;
        admission.context(reference)?;
        scope_changed |= Some(reference) != admission.execution.as_ref();
    }
    for (key, listener) in &state.listeners {
        let resource = ResourceRef::Listener { key: key.clone() };
        let reference = listener
            .netmap
            .as_ref()
            .or(state.configuration.applied.as_ref())
            .ok_or(Error::InvalidRequest)?;
        let context = admission.context(reference)?;
        if !context.resources.contains_key(&resource) || listener.socket_ids.len() > 32 {
            return Err(Error::InvalidRequest);
        }
        if Some(reference) != admission.execution.as_ref()
            && (!context.installation_allowed
                || !matches!(
                    listener.phase,
                    ListenerPhase::Preparing | ListenerPhase::Failed | ListenerPhase::Stopped
                )
                || !installation_reference(state, reference))
        {
            scope_changed = true;
        }
        for socket in &listener.socket_ids {
            if state.sockets.get(socket).is_none_or(|s| {
                s.resource != resource
                    || s.netmap.as_ref().or(state.configuration.applied.as_ref()) != Some(reference)
            }) {
                return Err(Error::InvalidRequest);
            }
        }
        if listener
            .socket_ids
            .iter()
            .filter_map(|id| state.sockets.get(id))
            .map(|socket| socket.local_endpoints.as_ref().map_or(0, Vec::len))
            .sum::<usize>()
            > MAX_LOCAL_ENDPOINTS
        {
            return Err(Error::StateLimit);
        }
    }
    for (socket_id, socket) in &state.sockets {
        let reference = socket
            .netmap
            .as_ref()
            .or(state.configuration.applied.as_ref())
            .ok_or(Error::InvalidRequest)?;
        let context = admission.context(reference)?;
        if Some(reference) != admission.execution.as_ref()
            && (!context.installation_allowed || !installation_reference(state, reference))
        {
            scope_changed = true;
        }
        let admitted = context
            .resources
            .get(&socket.resource)
            .ok_or(Error::InvalidRequest)?;
        if socket.bound_address.port() == 0
            || !text(socket_id)
            || socket.generation.get() == 0
            || socket.network != admitted.network
            || socket.endpoints.len() > 32
        {
            return Err(Error::InvalidRequest);
        }
        if let Some(local) = &socket.local_endpoints {
            if !matches!(socket.resource, ResourceRef::Listener { .. })
                || local.len() > MAX_LOCAL_ENDPOINTS
            {
                return Err(Error::InvalidRequest);
            }
            let mut addresses = BTreeSet::new();
            for endpoint in local {
                check_address(&endpoint.address.to_string(), socket, admission)?;
                super::connectivity::eligible_address(
                    &endpoint.address.to_string(),
                    &context.mesh_address_pools,
                )
                .map_err(|_| Error::InvalidRequest)?;
                if endpoint.address.port() != socket.bound_address.port()
                    || !socket.bound_address.ip().is_unspecified()
                        && endpoint.address.ip() != socket.bound_address.ip()
                    || !addresses.insert(endpoint.address)
                {
                    return Err(Error::InvalidRequest);
                }
            }
        }
        for candidate in &socket.endpoints {
            if !time(candidate.observed_at, candidate.expires_at, now) {
                return Err(Error::InvalidRequest);
            }
            match &candidate.origin {
                EndpointSource::Configured { config_path }
                    if admitted.configured_endpoints.get(config_path)
                        == Some(&candidate.address) => {}
                EndpointSource::Interface { interface } => {
                    if !text(interface) {
                        return Err(Error::InvalidRequest);
                    }
                    check_address(&candidate.address, socket, admission)?;
                    let address: SocketAddr = candidate
                        .address
                        .parse()
                        .map_err(|_| Error::InvalidRequest)?;
                    if address.port() != socket.bound_address.port()
                        || !socket.bound_address.ip().is_unspecified()
                            && address.ip() != socket.bound_address.ip()
                    {
                        return Err(Error::InvalidRequest);
                    }
                }
                EndpointSource::Stun { observer } => {
                    if socket.network != SocketNetwork::Udp
                        || observer.len() > 256
                        || observer.parse::<SocketAddr>().is_err()
                    {
                        return Err(Error::InvalidRequest);
                    }
                    check_address(&candidate.address, socket, admission)?;
                }
                _ => return Err(Error::InvalidRequest),
            }
        }
    }
    let mut links = BTreeSet::new();
    for (peer, records) in &state.links {
        let reference = state
            .configuration
            .applied
            .as_ref()
            .ok_or(Error::InvalidRequest)?;
        let context = admission.context(reference)?;
        if *peer == admission.node_id || !context.peers.contains(peer) || records.len() > 8 {
            return Err(Error::InvalidRequest);
        }
        for link in records {
            let socket = state
                .sockets
                .get(&link.socket_id)
                .ok_or(Error::InvalidRequest)?;
            if socket
                .netmap
                .as_ref()
                .or(state.configuration.applied.as_ref())
                != Some(reference)
            {
                return Err(Error::InvalidRequest);
            }
            let resource = context
                .resources
                .get(&socket.resource)
                .ok_or(Error::InvalidRequest)?;
            if resource.transport != link.transport {
                return Err(Error::InvalidRequest);
            }
            if !text(&link.link_id)
                || !links.insert(&link.link_id)
                || !text(&link.transport)
                || !state.sockets.contains_key(&link.socket_id)
                || ![admission.node_id, *peer].contains(&link.initiator_node_id)
                || !time(link.observed_at, link.expires_at, now)
                || link.observation_ids.len() > 16
                || link.since.is_some_and(|since| since > link.observed_at)
            {
                return Err(Error::InvalidRequest);
            }
        }
    }
    // Complete paths are decided locally on each netmap and are not State.
    if !state.paths.is_empty() {
        return Err(Error::InvalidRequest);
    }
    if let Some(profile) = &state.selection.applied_profile {
        let reference = state
            .configuration
            .applied
            .as_ref()
            .or(admission.execution.as_ref())
            .ok_or(Error::InvalidRequest)?;
        if !admission.context(reference)?.profiles.contains(profile) {
            return Err(Error::InvalidRequest);
        }
        scope_changed |= Some(reference) != admission.execution.as_ref();
    }
    let mut measurements = BTreeSet::new();
    for probe in &state.probes {
        if !measurements.insert(probe.measurement_id) {
            return Err(Error::InvalidRequest);
        }
        let reference = state
            .configuration
            .applied
            .as_ref()
            .or(admission.execution.as_ref())
            .ok_or(Error::InvalidRequest)?;
        validate_probe_context(
            admission.node_id,
            admission.context(reference)?,
            probe,
            now,
            None,
        )?;
        scope_changed |= Some(reference) != admission.execution.as_ref();
    }
    if scope_changed {
        Err(Error::StateScopeChanged)
    } else {
        Ok(())
    }
}
fn installation_reference(state: &NodeState, reference: &NetmapRef) -> bool {
    state.configuration.saved.as_ref() == Some(reference)
        || state
            .configuration
            .progress
            .as_ref()
            .is_some_and(|progress| &progress.target == reference)
}
fn check_address(
    address: &str,
    socket: &SocketState,
    admission: &ReportAdmission,
) -> Result<(), Error> {
    let address: SocketAddr = address.parse().map_err(|_| Error::InvalidRequest)?;
    if address.port() == 0
        || address.ip().is_unspecified()
        || address.ip().is_loopback()
        || address.ip().is_multicast()
        || address.is_ipv4() != socket.bound_address.is_ipv4()
        || admission.mesh_addresses.contains(&address.ip())
    {
        return Err(Error::InvalidRequest);
    }
    Ok(())
}
/// `paths`: the complete paths an observation task may measure; path probes
/// outside a task are not authorized.
pub(crate) fn validate_probe(
    admission: &ReportAdmission,
    probe: &ProbeObservation,
    now: DateTime<Utc>,
    paths: Option<&[Vec<NodeId>]>,
) -> Result<(), Error> {
    validate_probe_context(admission.node_id, admission.current()?, probe, now, paths)
}
pub(crate) fn validate_probe_context(
    node_id: NodeId,
    context: &ConfigurationAdmission,
    probe: &ProbeObservation,
    now: DateTime<Utc>,
    paths: Option<&[Vec<NodeId>]>,
) -> Result<(), Error> {
    if probe.kind == ProbeKind::Path && probe.path.len() > 2 && probe.transport.is_some() {
        // A source's first-hop carrier is not an end-to-end path transport.
        return Err(Error::InvalidRequest);
    }
    if let Some(channels) = &probe.channels {
        if probe.outcome == ProbeOutcome::Success && channels.reply.is_none()
            || probe.outcome == ProbeOutcome::Pending && channels.reply.is_some()
            || probe
                .transport
                .as_ref()
                .is_some_and(|transport| transport != channels.request.transport.as_str())
        {
            return Err(Error::InvalidRequest);
        }
        for channel in std::iter::once(&channels.request).chain(channels.reply.iter()) {
            let local = channel.local_address.get();
            let remote = channel.remote_address.get();
            if probe.path.get(1) != Some(&channel.peer_node_id)
                || local.is_ipv4() != remote.is_ipv4()
                || !context
                    .neighbor_transports
                    .get(&channel.peer_node_id)
                    .is_some_and(|allowed| allowed.contains(channel.transport.as_str()))
                || matches!(
                    channel.acquisition,
                    meshway_model::observations::LinkAcquisition::CoordinatedUdp { .. }
                ) && channel.transport != meshway_transport_spec::TransportProtocol::NativeUdp
            {
                return Err(Error::InvalidRequest);
            }
            // An unconnected shared UDP socket truthfully reports its wildcard
            // bind. The remote endpoint must always be concrete; never invent
            // the local egress interface address from a remote observation.
            for address in [
                Some(remote),
                (!local.ip().is_unspecified()
                    || channel.transport != meshway_transport_spec::TransportProtocol::NativeUdp)
                    .then_some(local),
            ]
            .into_iter()
            .flatten()
            {
                crate::state::connectivity::eligible_address(
                    &address.to_string(),
                    &context.mesh_address_pools,
                )
                .map_err(|_| Error::InvalidRequest)?;
            }
        }
    }
    if probe.link_id.as_ref().is_some_and(|id| !text(id))
        || probe.transport.as_ref().is_some_and(|transport| {
            !context
                .resources
                .values()
                .any(|resource| resource.transport == *transport)
        })
        || probe.kind == ProbeKind::Neighbor
            && (probe.link_id.is_none() || probe.transport.is_none())
    {
        return Err(Error::InvalidRequest);
    }
    let authorized = match probe.kind {
        ProbeKind::Path => paths.is_some_and(|paths| paths.contains(&probe.path)),
        ProbeKind::Neighbor | ProbeKind::Connect => {
            probe.path.len() == 2
                && probe.transport.as_ref().is_some_and(|transport| {
                    context
                        .neighbor_transports
                        .get(&probe.path[1])
                        .is_some_and(|allowed| allowed.contains(transport))
                })
        }
        // Server-originated probes have no Mesh peer/path. No current producer or
        // authorization contract exists for them yet.
        ProbeKind::Controller => false,
    };
    if probe.path.len() < 2
        || probe.path.len() > 16
        || probe.path.first() != Some(&node_id)
        || !authorized
        || context.configuration != probe.netmap
        || probe.started_at > now + Duration::seconds(CLOCK_SKEW_SECONDS)
        || probe.completed_at.is_some_and(|end| {
            end < probe.started_at || end > now + Duration::seconds(CLOCK_SKEW_SECONDS)
        })
    {
        return Err(Error::InvalidRequest);
    }
    match probe.outcome {
        ProbeOutcome::Pending if probe.completed_at.is_none() && probe.duration_us.is_none() => {
            Ok(())
        }
        ProbeOutcome::Success if probe.completed_at.is_some() && probe.duration_us.is_some() => {
            Ok(())
        }
        ProbeOutcome::Timeout | ProbeOutcome::Failed if probe.completed_at.is_some() => Ok(()),
        _ => Err(Error::InvalidRequest),
    }
}
