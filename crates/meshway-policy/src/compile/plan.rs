//! Planning: the first-match, profile, Access, topology and path rules that
//! compilation applies to a demand, applied to a declared destination and
//! reported with the configuration element that decided each step. No demand,
//! DNS sample or publication takes part; a domain is judged by its name and an
//! address by its value, and rules that would need the other are listed as
//! resolution dependent rather than guessed.
use super::matching::{self, EgressProfile};
use super::*;
use crate::topology_decision;
use meshway_configuration::evaluation::*;
use std::net::IpAddr;

pub enum PlanDestination {
    Node(NodeId),
    Address(IpAddr),
    Domain(String),
    Internet,
}

pub struct PlanRequest<'a> {
    pub topology: Topology<'a>,
    pub rule_sets: &'a rules::RuleSets,
    pub selected_profiles: &'a BTreeMap<NodeId, String>,
    pub source: NodeId,
    pub destination: PlanDestination,
    /// A profile the caller names; it must be allowed for the source.
    pub profile: Option<&'a str>,
}

/// `InvalidInput` names an unknown source or node, a malformed domain, or a
/// profile the source may not use. Missing remote rule-set content is an
/// `unavailable` outcome, not an error.
pub fn plan(request: PlanRequest<'_>) -> Result<Evaluation, CompileError> {
    plan_inner(request, None, 0)
}

pub fn plan_with_exit(
    request: PlanRequest<'_>,
    runtime: &ExitInputs,
    now_ms: u64,
) -> Result<Evaluation, CompileError> {
    plan_inner(request, Some(runtime), now_ms)
}

fn plan_inner(
    request: PlanRequest<'_>,
    runtime: Option<&ExitInputs>,
    now_ms: u64,
) -> Result<Evaluation, CompileError> {
    let top = request.topology;
    let cfg = top.configuration;
    let source = request.source;
    if !cfg.nodes.contains_key(&source) {
        return Err(CompileError::InvalidInput);
    }
    let destination = resolve_destination(cfg, &request.destination)?;

    let assignment = matching::assignment(cfg, top.subjects, source)?;
    let (name, requested) = match request.profile {
        Some(profile) => {
            if !matching::profile_allowed(cfg, top.subjects, source, profile)? {
                return Err(CompileError::InvalidInput);
            }
            (profile.to_owned(), true)
        }
        None => (
            match request.selected_profiles.get(&source) {
                Some(profile) if matching::profile_allowed(cfg, top.subjects, source, profile)? => {
                    profile.clone()
                }
                _ => assignment.default.clone(),
            },
            false,
        ),
    };
    let profile = ProfileEvaluation {
        capture: capture::summary(
            cfg,
            matching::Exits::Full(cfg),
            request.rule_sets,
            &name,
            Some(source),
        ),
        name,
        assignment: assignment
            .index
            .map(|index| format!("/routing/egress/assignments/{index}")),
        allowed: assignment.allowed,
        locked: assignment.locked,
        requested,
    };

    let access = access_evaluation(&top, source, &destination);
    let source_active = cfg.nodes[&source].enabled && top.inventory.contains_key(&source);

    let (route, mut outcome) = match &destination {
        EvaluatedDestination::Node { node_id } => mesh_route(&top, source, *node_id),
        EvaluatedDestination::Address {
            class: AddressClass::Mesh,
            ..
        } => (
            None,
            Some(EvaluationOutcome::Unavailable {
                reason: UnavailableReason::NoPath,
            }),
        ),
        _ => {
            let (route, outcome) = external_route(&request, &destination, &profile.name)?;
            // A source without its partition (its Node installs the full
            // fail-closed capture instead) fails every external destination
            // closed, as `LocalCompiler::decide` does.
            let unavailable = profile
                .capture
                .unavailable
                .filter(|_| source_active)
                .map(|reason| EvaluationOutcome::Unavailable { reason });
            (route, unavailable.or(outcome))
        }
    };

    let mut targets = Vec::new();
    let mut paths = Vec::new();
    let mut transport_error = None;
    if let Some(route) = &route
        && matches!(route.kind, RouteKind::Mesh | RouteKind::Exit)
    {
        let graph = paths::Graph::new(top)?;
        let mut reason = UnavailableReason::NoPath;
        for target in &route.targets {
            let settings = cfg.nodes.get(target).ok_or(CompileError::InvalidInput)?;
            let problem = if !settings.enabled {
                Some(TargetProblem::Disabled)
            } else if route.kind == RouteKind::Exit && !settings.exit_enabled {
                Some(TargetProblem::ExitDisabled)
            } else if !top.inventory.contains_key(target) {
                Some(TargetProblem::NotEnrolled)
            } else if route.kind == RouteKind::Exit
                && matches!(&destination, EvaluatedDestination::Address { address, .. } if address.parse::<IpAddr>().is_ok_and(|ip| ip.is_ipv6()))
            {
                reason = UnavailableReason::UnsupportedFamily;
                Some(TargetProblem::UnsupportedFamily)
            } else if route.kind == RouteKind::Exit
                && runtime.is_some_and(|runtime| !exit::available(top, runtime, *target, now_ms))
            {
                reason = UnavailableReason::ExitUnavailable;
                Some(TargetProblem::ExitUnavailable)
            } else {
                None
            };
            let mut found = Vec::new();
            let problem = match problem {
                Some(problem) => Some(problem),
                None => {
                    let via = (route.kind == RouteKind::Exit).then_some(route.via.as_slice());
                    match graph.paths(source, *target, via, MAX_PATHS) {
                        Ok(candidates) if candidates.is_empty() => Some(TargetProblem::NoPath),
                        Ok(candidates) => {
                            found = candidates;
                            None
                        }
                        Err(routing::PathError::Cycle) => {
                            reason = UnavailableReason::RoutingCycle;
                            Some(TargetProblem::RoutingCycle)
                        }
                        Err(routing::PathError::Limit) => {
                            reason = UnavailableReason::RoutingLimit;
                            Some(TargetProblem::RoutingLimit)
                        }
                        Err(routing::PathError::Compile(
                            error @ CompileError::EdgeTransportUnavailable { .. },
                        )) => {
                            transport_error.get_or_insert(error);
                            Some(TargetProblem::NoPath)
                        }
                        Err(routing::PathError::Compile(error)) => return Err(error),
                    }
                }
            };
            targets.push(TargetEvaluation {
                node_id: *target,
                problem,
                paths: u32::try_from(found.len()).unwrap_or(u32::MAX),
                exit_unavailable_reason: if route.kind == RouteKind::Exit {
                    runtime.and_then(|runtime| runtime.unavailable_reasons.get(target).copied())
                } else {
                    None
                },
            });
            for path in found {
                paths.push(planned(&top, path));
            }
        }
        if outcome.is_none()
            && paths.is_empty()
            && source_active
            && access.allowed
            && let Some(error) = transport_error
        {
            return Err(error);
        }
        if outcome.is_none() {
            outcome = Some(if !source_active {
                EvaluationOutcome::Unavailable {
                    reason: UnavailableReason::InactiveNode,
                }
            } else if !access.allowed {
                EvaluationOutcome::Unavailable {
                    reason: UnavailableReason::Unauthorized,
                }
            } else if paths.is_empty() {
                EvaluationOutcome::Unavailable { reason }
            } else {
                EvaluationOutcome::Permitted
            });
        }
    }
    let outcome = outcome.unwrap_or_else(|| {
        if !source_active {
            return EvaluationOutcome::Unavailable {
                reason: UnavailableReason::InactiveNode,
            };
        }
        match route.as_ref().map(|r| r.kind) {
            Some(RouteKind::Direct) => EvaluationOutcome::Direct,
            Some(RouteKind::Unmanaged) => EvaluationOutcome::Unmanaged,
            _ => EvaluationOutcome::Unavailable {
                reason: UnavailableReason::NoPath,
            },
        }
    });
    Ok(Evaluation {
        source,
        destination,
        profile,
        access,
        route,
        targets,
        paths,
        outcome,
    })
}

fn resolve_destination(
    cfg: &NetworkConfiguration,
    destination: &PlanDestination,
) -> Result<EvaluatedDestination, CompileError> {
    Ok(match destination {
        PlanDestination::Node(node_id) => {
            if !cfg.nodes.contains_key(node_id) {
                return Err(CompileError::InvalidInput);
            }
            EvaluatedDestination::Node { node_id: *node_id }
        }
        PlanDestination::Address(address) => {
            let address = *address;
            let canonical = address.to_canonical();
            if let Some((node_id, _)) = cfg.nodes.iter().find(|(_, n)| {
                n.addresses.ipv4.map(IpAddr::V4) == Some(canonical)
                    || n.addresses.ipv6.map(IpAddr::V6) == Some(canonical)
            }) {
                EvaluatedDestination::Node { node_id: *node_id }
            } else {
                let in_pool = cfg
                    .address_pools
                    .ipv4
                    .iter()
                    .chain(&cfg.address_pools.ipv6)
                    .any(|p| {
                        p.parse::<ipnet::IpNet>()
                            .is_ok_and(|p| p.contains(&canonical))
                    });
                EvaluatedDestination::Address {
                    address: address.to_string(),
                    class: if in_pool {
                        AddressClass::Mesh
                    } else if matching::internet(canonical) {
                        AddressClass::Public
                    } else {
                        AddressClass::Private
                    },
                }
            }
        }
        PlanDestination::Domain(name) => EvaluatedDestination::Domain {
            name: meshway_configuration::normalize_dns_name(name)
                .map_err(|_| CompileError::InvalidInput)?,
        },
        PlanDestination::Internet => EvaluatedDestination::Internet,
    })
}

/// Whether one Access destination selector covers the planned destination:
/// `Some(true)` by its own kind, `Some(false)` only through a resolution the
/// plan does not know, `None` not at all.
fn selector_covers(selector: &str, destination: &EvaluatedDestination) -> Option<bool> {
    let domain = selector.strip_prefix("domain:");
    let internet = selector == "autogroup:internet";
    let net = selector.parse::<ipnet::IpNet>().ok();
    match destination {
        EvaluatedDestination::Address { address, class } => {
            let address: IpAddr = address.parse().ok()?;
            if domain.is_some() {
                return Some(false);
            }
            if internet {
                return (*class == AddressClass::Public).then_some(true);
            }
            net.filter(|n| n.contains(&address.to_canonical()))
                .map(|_| true)
        }
        EvaluatedDestination::Domain { name } => {
            if let Some(suffix) = domain {
                return crate::domain_suffix_matches(name, suffix).then_some(true);
            }
            // Address-set semantics, as validation accepts it: a public
            // resolution is inside the Internet grant; a CIDR may or may not
            // contain what the name resolves to.
            if internet {
                return Some(true);
            }
            net.map(|_| false)
        }
        EvaluatedDestination::Internet => {
            if internet {
                return Some(true);
            }
            (domain.is_some() || net.is_some()).then_some(false)
        }
        EvaluatedDestination::Node { .. } => None,
    }
}

fn access_evaluation(
    top: &Topology<'_>,
    source: NodeId,
    destination: &EvaluatedDestination,
) -> AccessEvaluation {
    let cfg = top.configuration;
    let matches =
        |selectors: &[String], node| matching::selectors(cfg, top.subjects, selectors, node);
    let mut grants = Vec::new();
    let mut active = true;
    match destination {
        EvaluatedDestination::Node { node_id } => {
            active = top.subjects.node_is_active(cfg, source)
                && top.subjects.node_is_active(cfg, *node_id);
            for (index, rule) in cfg.access.iter().enumerate() {
                let forward = matches(&rule.src, source) && matches(&rule.dst, *node_id);
                let reverse = rule.bidirectional
                    && matches(&rule.dst, source)
                    && matches(&rule.src, *node_id);
                if forward || reverse {
                    grants.push(AccessGrantEvaluation {
                        rule: format!("/access/{index}"),
                        traffic: rule.ip.clone(),
                        reverse: !forward,
                        resolution_dependent: false,
                    });
                }
            }
        }
        EvaluatedDestination::Address {
            class: AddressClass::Mesh,
            ..
        } => {}
        _ => {
            for (index, rule) in cfg.access.iter().enumerate() {
                if !matches(&rule.src, source) {
                    continue;
                }
                let mut covered = None;
                for selector in &rule.dst {
                    match selector_covers(selector, destination) {
                        Some(true) => covered = Some(true),
                        Some(false) => covered = covered.or(Some(false)),
                        None => {}
                    }
                }
                if let Some(definite) = covered {
                    grants.push(AccessGrantEvaluation {
                        rule: format!("/access/{index}"),
                        traffic: rule.ip.clone(),
                        reverse: false,
                        resolution_dependent: !definite,
                    });
                }
            }
        }
    }
    AccessEvaluation {
        allowed: active && grants.iter().any(|g| !g.resolution_dependent),
        grants,
    }
}

fn mesh_route(
    top: &Topology<'_>,
    source: NodeId,
    target: NodeId,
) -> (Option<RouteEvaluation>, Option<EvaluationOutcome>) {
    let mut routes = routing::Routes::new(*top);
    let (via, outcome) = match routes.segment(source, target) {
        Ok(required) => (
            required.into_iter().filter(|n| *n != target).collect(),
            None,
        ),
        Err(routing::PathError::Cycle) => (
            Vec::new(),
            Some(EvaluationOutcome::Unavailable {
                reason: UnavailableReason::RoutingCycle,
            }),
        ),
        Err(routing::PathError::Limit | routing::PathError::Compile(_)) => (
            Vec::new(),
            Some(EvaluationOutcome::Unavailable {
                reason: UnavailableReason::RoutingLimit,
            }),
        ),
    };
    let rule = routes
        .mesh_rules
        .get(&(source, target))
        .copied()
        .flatten()
        .map(|index| format!("/routing/mesh/{index}"));
    (
        Some(RouteEvaluation {
            kind: RouteKind::Mesh,
            rule,
            profile: None,
            rule_set: None,
            targets: vec![target],
            via,
            resolution_dependent: Vec::new(),
        }),
        outcome,
    )
}

/// The route `source` executes for the deciding action ([`matching::executed`]).
fn route_of(
    cfg: &NetworkConfiguration,
    source: NodeId,
    kind_action: &RouteAction,
    rule: Option<String>,
    profile: Option<String>,
    rule_set: Option<RuleSetMatch>,
    resolution_dependent: Vec<String>,
) -> Result<RouteEvaluation, CompileError> {
    let (kind, targets, via) =
        match &matching::executed(matching::Exits::Full(cfg), kind_action.clone(), source)? {
            RouteAction::Exit { nodes, via } => (
                RouteKind::Exit,
                nodes
                    .iter()
                    .map(|s| matching::node_selector(s))
                    .collect::<Result<Vec<_>, _>>()?,
                via.iter()
                    .map(|s| matching::node_selector(s))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            RouteAction::Direct => (RouteKind::Direct, Vec::new(), Vec::new()),
            RouteAction::Unmanaged => (RouteKind::Unmanaged, Vec::new(), Vec::new()),
        };
    Ok(RouteEvaluation {
        kind,
        rule,
        profile,
        rule_set,
        targets,
        via,
        resolution_dependent,
    })
}

fn missing_dependency() -> (Option<RouteEvaluation>, Option<EvaluationOutcome>) {
    (
        None,
        Some(EvaluationOutcome::Unavailable {
            reason: UnavailableReason::MissingDependency,
        }),
    )
}

/// The planned destination as egress rules judge it; `None` for destinations
/// no rule names.
fn judged(
    destination: &EvaluatedDestination,
) -> Result<Option<rules::Destination<'_>>, CompileError> {
    Ok(match destination {
        EvaluatedDestination::Address { address, .. } => Some(rules::Destination::Address(
            address.parse().map_err(|_| CompileError::InvalidInput)?,
        )),
        EvaluatedDestination::Domain { name } => Some(rules::Destination::Name(name)),
        EvaluatedDestination::Internet | EvaluatedDestination::Node { .. } => None,
    })
}

/// The first rule selecting the destination, with rules skipped for the other
/// destination kind appended to `skipped` as pointers under `prefix`.
fn first_match(
    request: &PlanRequest<'_>,
    matchers: &rules::Matchers,
    profile: Option<&str>,
    destination: &EvaluatedDestination,
    rules_list: &[EgressRule],
    prefix: &str,
    skipped: &mut Vec<String>,
) -> Result<Option<(usize, Option<RuleSetMatch>)>, CompileError> {
    let Some(destination) = judged(destination)? else {
        // Rule-set references are still dependencies of the decision.
        for rule in rules_list {
            if rule
                .matcher
                .rule_sets
                .iter()
                .any(|key| !request.rule_sets.contains_key(key))
            {
                return Err(CompileError::UnavailableDependency);
            }
        }
        return Ok(None);
    };
    rules::first_match(
        rules_list,
        matchers.of(profile)?,
        request.rule_sets,
        destination,
        prefix,
        skipped,
    )
}

fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn external_route(
    request: &PlanRequest<'_>,
    destination: &EvaluatedDestination,
    profile: &str,
) -> Result<(Option<RouteEvaluation>, Option<EvaluationOutcome>), CompileError> {
    let cfg = request.topology.configuration;
    let matchers = rules::Matchers::new(cfg)?;
    let mut skipped = Vec::new();
    let public = match first_match(
        request,
        &matchers,
        None,
        destination,
        &cfg.routing.egress.rules,
        "/routing/egress/rules",
        &mut skipped,
    ) {
        Ok(result) => result,
        Err(CompileError::UnavailableDependency) => return Ok(missing_dependency()),
        Err(error) => return Err(error),
    };
    if let Some((index, rule_set)) = public {
        let rule = &cfg.routing.egress.rules[index];
        return Ok((
            Some(route_of(
                cfg,
                request.source,
                &rule.action.clone().into(),
                Some(format!("/routing/egress/rules/{index}")),
                None,
                rule_set,
                skipped,
            )?),
            None,
        ));
    }
    let (resolved, name, auto) =
        match matching::concrete_profile(cfg, profile, Some(request.source)) {
            Ok(resolved) => resolved,
            Err(CompileError::UnavailableDependency) => return Ok(missing_dependency()),
            Err(error) => return Err(error),
        };
    if auto.is_some()
        && !matching::assignment(cfg, request.topology.subjects, request.source)?
            .allowed
            .contains(&name)
    {
        return Err(CompileError::InvalidInput);
    }
    let custom = match resolved {
        EgressProfile::Mesh => {
            return Ok((
                Some(route_of(
                    cfg,
                    request.source,
                    &RouteAction::Unmanaged,
                    None,
                    Some(name),
                    None,
                    skipped,
                )?),
                None,
            ));
        }
        EgressProfile::Global(node) => {
            return Ok((
                Some(route_of(
                    cfg,
                    request.source,
                    &RouteAction::Exit {
                        nodes: vec![format!("node:{node}")],
                        via: vec![],
                    },
                    None,
                    Some(name),
                    None,
                    skipped,
                )?),
                None,
            ));
        }
        EgressProfile::Custom(custom) => custom,
        EgressProfile::Auto(_) => return Err(CompileError::InvalidInput),
    };
    let prefix = format!("/routing/egress/profiles/{}/rules", escape(&name));
    let matched = match first_match(
        request,
        &matchers,
        Some(&name),
        destination,
        &custom.rules,
        &prefix,
        &mut skipped,
    ) {
        Ok(result) => result,
        Err(CompileError::UnavailableDependency) => return Ok(missing_dependency()),
        Err(error) => return Err(error),
    };
    let (action, rule, rule_set) = match matched {
        Some((index, rule_set)) => (
            custom.rules[index].action.clone().into(),
            format!("{prefix}/{index}"),
            rule_set,
        ),
        None => (
            custom.fallback.clone(),
            format!("/routing/egress/profiles/{}/fallback", escape(&name)),
            None,
        ),
    };
    Ok((
        Some(route_of(
            cfg,
            request.source,
            &action,
            Some(rule),
            Some(name),
            rule_set,
            skipped,
        )?),
        None,
    ))
}

fn planned(top: &Topology<'_>, path: LogicalPath) -> PlannedPath {
    let edges = path
        .nodes
        .windows(2)
        .map(|pair| PlannedEdge {
            a: pair[0],
            b: pair[1],
            topology: topology_decision(top.configuration, top.subjects, pair[0], pair[1])
                .flatten()
                .map(|index| format!("/topology/{index}")),
        })
        .collect();
    PlannedPath {
        nodes: path.nodes,
        edges,
    }
}
