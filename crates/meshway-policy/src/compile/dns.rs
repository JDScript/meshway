//! Single-exit selection is decided before resolving a name. Never use an
//! answer to decide which resolver was authorized to obtain that answer.
use super::*;

pub(super) fn install(
    input: &CompilationInput<'_>,
    graph: &paths::Graph,
    runtime: &ExitInputs,
    nodes: &mut BTreeMap<NodeId, NodeConfiguration>,
) -> Result<(), CompileError> {
    // Incomplete instance/deadline inputs cannot create a resolver plan.
    if runtime.instances.is_empty() || runtime.instance_expires_at_ms.is_empty() {
        return Ok(());
    }
    let mut grants = Vec::new();
    for (&source, cfg) in nodes.iter_mut() {
        if matches!(cfg.dns.egress, DnsEgress::NoExit) {
            continue;
        }
        let result = compile_service(input, graph, runtime, cfg)?;
        match result {
            Ok(service) => {
                cfg.dns.egress = DnsEgress::ViaExit {
                    exit: service.exit,
                    generation: service.generation,
                    service: Box::new(service.clone()),
                };
                grants.push((source, service));
            }
            Err(reason) => cfg.dns.egress = DnsEgress::Unavailable { reason },
        }
    }
    if grants.len() > MAX_DEMANDS {
        return Err(CompileError::Budget);
    }
    for (_, service) in grants {
        let exit = nodes
            .get_mut(&service.exit)
            .ok_or(CompileError::InvalidInput)?;
        match &mut exit.dns.egress {
            DnsEgress::NoExit => {
                exit.dns.egress = DnsEgress::ServeExitResolver {
                    permit: vec![service.clone()],
                };
            }
            DnsEgress::ServeExitResolver { permit } => permit.push(service.clone()),
            _ => return Err(CompileError::InvalidInput),
        }
        for id in &service.path.nodes {
            let node = nodes.get_mut(id).ok_or(CompileError::InvalidInput)?;
            for peer in service.path.nodes.iter().filter(|peer| *peer != id) {
                let settings = &input.configuration.nodes[peer];
                let trusted = &input.inventory[peer];
                node.peers.insert(
                    *peer,
                    PeerConfiguration {
                        identity: settings.identity.clone(),
                        addresses: settings.addresses.clone(),
                        control_key: trusted.control_key.clone(),
                        dataplane_key: trusted.dataplane_key.clone(),
                    },
                );
            }
            if let Some(keys) = service.path.dial_candidates.get(id) {
                for key in keys {
                    node.dialers
                        .insert(key.clone(), graph.candidates.borrow()[id][key].clone());
                }
            }
        }
    }
    Ok(())
}

fn compile_service(
    input: &CompilationInput<'_>,
    graph: &paths::Graph,
    runtime: &ExitInputs,
    cfg: &NodeConfiguration,
) -> Result<Result<DnsServicePermit, DnsEgressUnavailable>, CompileError> {
    let unavailable = |reason| Ok(Err(reason));
    let Some((exit, via)) = single_exit(input.configuration, &cfg.dns.profile)? else {
        return unavailable(DnsEgressUnavailable::ExitPlanRequired);
    };
    // The exit's own system resolution must never recurse into another Mesh
    // exit, including itself. No fallback to a public or bootstrap resolver.
    let exit_profile = selected_profile(input, exit)?;
    if exit == cfg.node_id
        || cfg.egress_capture.unavailable.is_some()
        || !matches!(
            matching::dns_egress(input.configuration, &exit_profile),
            DnsEgress::NoExit
        )
    {
        return unavailable(DnsEgressUnavailable::ExitPlanRequired);
    }
    if !cfg.enabled || !external_access(input, cfg.node_id) {
        return unavailable(DnsEgressUnavailable::Unauthorized);
    }
    let Some(source_instance_id) = runtime.instances.get(&cfg.node_id).copied() else {
        return unavailable(DnsEgressUnavailable::Unauthorized);
    };
    let Some(source_expiry) = runtime
        .instance_expires_at_ms
        .get(&cfg.node_id)
        .copied()
        .filter(|expiry| *expiry > input.now_ms)
    else {
        return unavailable(DnsEgressUnavailable::Unauthorized);
    };
    let Some(role) = exit::role(input, runtime, exit)? else {
        return unavailable(DnsEgressUnavailable::ExitUnavailable);
    };
    if !exit::available(input.topology(), runtime, exit, input.now_ms) {
        return unavailable(DnsEgressUnavailable::ExitUnavailable);
    }
    let Some(address) = input
        .configuration
        .nodes
        .get(&exit)
        .and_then(|n| n.addresses.ipv4)
    else {
        return unavailable(DnsEgressUnavailable::ExitUnavailable);
    };
    if cfg.addresses.ipv4.is_none() {
        return unavailable(DnsEgressUnavailable::NoPath);
    }
    let path = match graph.paths(cfg.node_id, exit, Some(&via), 1) {
        Ok(paths) => paths.into_iter().next(),
        Err(routing::PathError::Compile(error @ CompileError::Serialization)) => return Err(error),
        Err(_) => None,
    };
    let Some(path) = path else {
        return unavailable(DnsEgressUnavailable::NoPath);
    };
    Ok(Ok(DnsServicePermit {
        source: cfg.node_id,
        source_instance_id,
        source_addresses: cfg.addresses.clone(),
        exit,
        address: ExecutionAddress(address.into()),
        profile: cfg.dns.profile.clone(),
        generation: cfg.egress_capture.generation,
        role,
        path,
        transports: vec![DnsTransport::Udp, DnsTransport::Tcp],
        max_pending: 128,
        timeout_ms: 5_000,
        expires_at_ms: runtime.ready_until_ms[&exit].min(source_expiry),
    }))
}

fn external_access(input: &CompilationInput<'_>, source: NodeId) -> bool {
    input.configuration.access.iter().any(|rule| {
        !rule.ip.is_empty()
            && matching::selectors(input.configuration, input.subjects, &rule.src, source)
            && rule.dst.iter().any(|selector| {
                selector == "autogroup:internet"
                    || selector.starts_with("domain:")
                    || selector.parse::<ipnet::IpNet>().is_ok()
            })
    })
}

/// Conservatively require one identical exit and ordered via list across all
/// effective rules/fallbacks. Auto and domain-dependent capture stay unavailable.
fn single_exit(
    cfg: &NetworkConfiguration,
    profile: &str,
) -> Result<Option<(NodeId, Vec<NodeId>)>, CompileError> {
    let mut actions: Vec<RouteAction> = cfg
        .routing
        .egress
        .rules
        .iter()
        .map(|r| r.action.clone().into())
        .collect();
    match matching::resolve_profile(cfg, profile)? {
        matching::EgressProfile::Mesh => {}
        matching::EgressProfile::Global(node) => actions.push(RouteAction::Exit {
            nodes: vec![format!("node:{node}")],
            via: vec![],
        }),
        matching::EgressProfile::Custom(profile) => {
            actions.extend(profile.rules.iter().map(|r| r.action.clone().into()));
            actions.push(profile.fallback.clone());
        }
        matching::EgressProfile::Auto(_) => return Ok(None),
    }
    let mut selected = None;
    for action in actions {
        let RouteAction::Exit { nodes, via } = action else {
            continue;
        };
        if nodes.len() != 1 {
            return Ok(None);
        }
        let candidate = (
            matching::node_selector(&nodes[0])?,
            via.iter()
                .map(|s| matching::node_selector(s))
                .collect::<Result<Vec<_>, _>>()?,
        );
        if selected.as_ref().is_some_and(|old| old != &candidate) {
            return Ok(None);
        }
        selected = Some(candidate);
    }
    Ok(selected)
}
