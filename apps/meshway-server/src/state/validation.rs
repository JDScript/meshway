use super::*;
use chrono::{DateTime, Duration, Utc};
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
    if (!state.links.is_empty() || !state.paths.is_empty())
        && (admission.execution.is_none()
            || state.configuration.applied.as_ref() != admission.execution.as_ref())
    {
        return Err(Error::InvalidRequest);
    }
    for (key, listener) in &state.listeners {
        let resource = ResourceRef::Listener { key: key.clone() };
        let reference = listener
            .configuration
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
            return Err(Error::InvalidRequest);
        }
        for socket in &listener.socket_ids {
            if state.sockets.get(socket).is_none_or(|s| {
                s.resource != resource
                    || s.configuration
                        .as_ref()
                        .or(state.configuration.applied.as_ref())
                        != Some(reference)
            }) {
                return Err(Error::InvalidRequest);
            }
        }
    }
    for (socket_id, socket) in &state.sockets {
        let reference = socket
            .configuration
            .as_ref()
            .or(state.configuration.applied.as_ref())
            .ok_or(Error::InvalidRequest)?;
        let context = admission.context(reference)?;
        if Some(reference) != admission.execution.as_ref()
            && (!context.installation_allowed || !installation_reference(state, reference))
        {
            return Err(Error::InvalidRequest);
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
        let context = admission.current()?;
        if *peer == admission.node_id || !context.peers.contains(peer) || records.len() > 8 {
            return Err(Error::InvalidRequest);
        }
        for link in records {
            let socket = state
                .sockets
                .get(&link.socket_id)
                .ok_or(Error::InvalidRequest)?;
            if socket
                .configuration
                .as_ref()
                .or(state.configuration.applied.as_ref())
                != admission.execution.as_ref()
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
    for path in state.paths.values() {
        if !admission.current()?.paths.contains(&path.nodes)
            || !path.nodes.contains(&admission.node_id)
            || admission
                .execution
                .as_ref()
                .is_none_or(|execution| execution.execution_publication != path.publication_id)
            || !time(path.observed_at, path.expires_at, now)
        {
            return Err(Error::InvalidRequest);
        }
        let index = path
            .nodes
            .iter()
            .position(|id| *id == admission.node_id)
            .ok_or(Error::InvalidRequest)?;
        match path.reporter_role {
            ReporterRole::Origin if index == 0 => {}
            ReporterRole::Target | ReporterRole::Exit if index + 1 == path.nodes.len() => {}
            ReporterRole::Relay
                if index > 0
                    && index + 1 < path.nodes.len()
                    && path.evidence != PathEvidence::Delivered => {}
            _ => return Err(Error::InvalidRequest),
        }
    }
    if state.selection.applied_profile.as_ref().is_some_and(|p| {
        admission
            .current()
            .map_or(true, |context| !context.profiles.contains(p))
    }) {
        return Err(Error::InvalidRequest);
    }
    let mut measurements = BTreeSet::new();
    for probe in &state.probes {
        if !measurements.insert(probe.measurement_id) {
            return Err(Error::InvalidRequest);
        }
        validate_probe(admission, probe, now)?;
    }
    Ok(())
}
fn installation_reference(state: &NodeState, reference: &ConfigurationRef) -> bool {
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
pub(crate) fn validate_probe(
    admission: &ReportAdmission,
    probe: &ProbeObservation,
    now: DateTime<Utc>,
) -> Result<(), Error> {
    validate_probe_context(admission.node_id, admission.current()?, probe, now)
}
pub(crate) fn validate_probe_context(
    node_id: NodeId,
    context: &ConfigurationAdmission,
    probe: &ProbeObservation,
    now: DateTime<Utc>,
) -> Result<(), Error> {
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
        ProbeKind::Path => {
            context.paths.contains(&probe.path)
                && context
                    .path_deadlines
                    .get(&probe.path)
                    .is_some_and(|expiry| {
                        probe.started_at < *expiry
                            && probe.completed_at.is_none_or(|end| end <= *expiry)
                    })
        }
        ProbeKind::Neighbor | ProbeKind::Connect => {
            probe.path.len() == 2
                && probe.transport.as_ref().is_some_and(|transport| {
                    context
                        .neighbor_transports
                        .get(&probe.path[1])
                        .is_some_and(|allowed| allowed.contains(transport))
                })
        }
        // Controller probes have no Mesh peer/path. No current producer or
        // authorization contract exists for them yet.
        ProbeKind::Controller => false,
    };
    if probe.path.len() < 2
        || probe.path.len() > 16
        || probe.path.first() != Some(&node_id)
        || !authorized
        || context.configuration.execution_publication != probe.publication_id
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
