use super::super::{Report, plane::Plane};

/// An explicit measurement by the resource owner. Application observation time
/// remains immutable; sampling current sockets is distinct from receipt replay.
pub(super) fn sample_host(
    plane: &Plane,
    diagnostics: &super::super::diagnostics::Diagnostics,
    mut report: Report,
) -> Report {
    report.interface_epoch = *plane.host.network_changes().borrow();
    report.owned_interfaces = plane.host.owned_interfaces();
    report.known_peers = Some(plane.peers.clone());
    sample(&plane.installed, diagnostics, report)
}

fn sample(
    installed: &Option<super::super::plane::Installed>,
    diagnostics: &super::super::diagnostics::Diagnostics,
    mut report: Report,
) -> Report {
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("host.sample");
    let _profile = PROFILE.enter();
    let secure = installed.as_ref().and_then(|state| state.secure.as_ref());
    meshway_logging::profile::gauge(
        "secure.circuit_count",
        secure.map(|secure| secure.circuit_count() as u64),
    );
    meshway_logging::profile::gauge(
        "flow.count",
        secure.map(|secure| secure.flow_count() as u64),
    );
    meshway_logging::profile::gauge("secure.ready_circuits", None);
    report.facts = None;
    report.exit_flows = installed
        .as_ref()
        .and_then(|s| s.exit.as_ref())
        .map(|exit| exit.health.borrow().flows);
    report.dependencies.clear();
    report.observation_dependencies.clear();
    report.circuits.clear();
    report.routes.clear();
    report.relays.clear();
    report.idle_paths.clear();
    report.rtts = diagnostics.rtts().clone();
    report.facts_observed_at = chrono::Utc::now();
    if let Some(state) = installed
        && let (Some(secure), Some(manager)) = (&state.secure, &state.manager)
    {
        report.facts = manager.snapshot().ok();
        if let Ok(circuits) = secure.ready_circuits() {
            let node = state.desired.netmap().node_id;
            meshway_logging::profile::gauge("secure.ready_circuits", Some(circuits.len() as u64));
            for circuit in circuits {
                let far = match (circuit.path.first(), circuit.path.last()) {
                    (Some(first), Some(last)) if *first == node => Some(*last),
                    (Some(first), Some(last)) if *last == node => Some(*first),
                    _ => None,
                };
                if let Some(far) = far {
                    let mut path = circuit.path.clone();
                    if path.first() != Some(&node) {
                        path.reverse();
                    }
                    let shorter = report
                        .routes
                        .get(&far)
                        .is_none_or(|known| path.len() < known.len());
                    if shorter {
                        report.routes.insert(far, path);
                    }
                }
                if circuit.grant.purpose == meshway_dataplane::Purpose::Business {
                    use meshway_dataplane::secure::CircuitRole;
                    use meshway_model::state::ReporterRole;
                    let role = match circuit.role {
                        CircuitRole::Origin => ReporterRole::Origin,
                        CircuitRole::Relay => ReporterRole::Relay,
                        CircuitRole::Terminal => match circuit.grant.target {
                            meshway_dataplane::Target::Exit { .. } => ReporterRole::Exit,
                            meshway_dataplane::Target::Mesh { .. } => ReporterRole::Target,
                        },
                    };
                    report
                        .circuits
                        .insert(circuit.id, (circuit.path.clone(), role));
                }
                for pair in circuit.path.windows(2) {
                    let peer = if pair[0] == node {
                        Some(pair[1])
                    } else if pair[1] == node {
                        Some(pair[0])
                    } else {
                        None
                    };
                    let Some(peer) = peer else {
                        continue;
                    };
                    match circuit.grant.purpose {
                        meshway_dataplane::Purpose::Dns => {}
                        meshway_dataplane::Purpose::Business => {
                            *report.dependencies.entry(peer).or_default() += 1
                        }
                        meshway_dataplane::Purpose::Diagnostic => {
                            let meshway_dataplane::Target::Mesh { node } = &circuit.grant.target
                            else {
                                continue;
                            };
                            let ids = report.observation_dependencies.entry(peer).or_default();
                            for id in diagnostics.observations_toward(*node) {
                                if !ids.contains(&id) {
                                    ids.push(id);
                                }
                            }
                        }
                    }
                }
            }
            // A peer reached through several circuits shows the one a new
            // flow joins, in SecureNode's path order, not the first in ID order.
            for (far, path) in &mut report.routes {
                if let Some(joined) = secure.joined_route(*far) {
                    *path = joined;
                }
            }
            // Bounded by ID order, the same set on every sample.
            while report.circuits.len() > meshway_protocol::STATE_MAX_PATHS {
                report.circuits.pop_last();
            }
        }
        // Only needed peers without a channel or circuit; bounded by the
        // adjacency needs the Manager tracks.
        if let Some(facts) = &report.facts {
            for (peer, status) in &facts.adjacencies {
                if *status == meshway_model::state::AdjacencyStatus::Unreachable
                    && !report.routes.contains_key(peer)
                    && let Some(path) = secure.relay_route(*peer)
                {
                    report.relays.insert(*peer, path);
                }
            }
        }
        // Every visible peer without a channel or circuit: the netmap bounds them.
        let netmap = state.desired.netmap();
        let linked = |peer: &meshway_model::NodeId| {
            report
                .facts
                .as_ref()
                .is_some_and(|facts| facts.links.iter().any(|link| link.peer == *peer))
        };
        for peer in netmap.projection.configuration.nodes.keys() {
            if *peer != netmap.node_id
                && netmap.projection.inventory.contains_key(peer)
                && !linked(peer)
                && !report.routes.contains_key(peer)
            {
                report.idle_paths.insert(*peer, secure.idle_route(*peer));
            }
        }
    }
    report
}
