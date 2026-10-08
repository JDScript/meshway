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
    pub rule_sets: &'a BTreeMap<String, RuleSetContent>,
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
        _ => external_route(&request, &destination, &profile.name)?,
    };

    let mut targets = Vec::new();
    let mut paths = Vec::new();
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
                        Err(routing::PathError::Compile(error)) => return Err(error),
                    }
                }
            };
            targets.push(TargetEvaluation {
                node_id: *target,
                problem,
                paths: u32::try_from(found.len()).unwrap_or(u32::MAX),
            });
            for path in found {
                paths.push(planned(&top, path));
            }
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

enum Match {
    Yes(Option<RuleSetMatch>),
    ResolutionDependent,
    No,
}

/// Whether a route matcher selects the planned destination. Rule-set content
/// is the bound artifact for remote sets; its absence is not a non-match.
fn plan_match(
    request: &PlanRequest<'_>,
    matcher: &RouteMatch,
    destination: &EvaluatedDestination,
) -> Result<Match, CompileError> {
    let cfg = request.topology.configuration;
    // The matcher's own entries first, then each referenced set's.
    struct Entries<'a> {
        key: Option<&'a str>,
        ips: &'a [String],
        suffixes: &'a [String],
        exact: &'a [String],
    }
    let mut sets = vec![Entries {
        key: None,
        ips: &matcher.ip_cidr,
        suffixes: &matcher.domain_suffix,
        exact: &matcher.domain,
    }];
    for key in &matcher.rule_sets {
        match cfg.rule_sets.get(key) {
            Some(RuleSet::Inline {
                ip_cidr,
                domain_suffix,
                domain,
            }) => sets.push(Entries {
                key: Some(key),
                ips: ip_cidr,
                suffixes: domain_suffix,
                exact: domain,
            }),
            Some(RuleSet::Remote { .. }) => {
                let content = request
                    .rule_sets
                    .get(key)
                    .ok_or(CompileError::UnavailableDependency)?;
                sets.push(Entries {
                    key: Some(key),
                    ips: &content.ip_cidr,
                    suffixes: &content.domain_suffix,
                    exact: &content.domain,
                });
            }
            None => return Err(CompileError::UnavailableDependency),
        }
    }
    let found = |key: Option<&str>, entry: &str| {
        Match::Yes(key.map(|key| RuleSetMatch {
            key: key.to_owned(),
            entry: entry.to_owned(),
        }))
    };
    let mut other_kind = false;
    match destination {
        EvaluatedDestination::Address { address, .. } => {
            let address: IpAddr = address.parse().map_err(|_| CompileError::InvalidInput)?;
            for set in &sets {
                if let Some(entry) = set.ips.iter().find(|s| matching::cidr(s, address)) {
                    return Ok(found(set.key, entry));
                }
                other_kind |= !set.suffixes.is_empty() || !set.exact.is_empty();
            }
        }
        EvaluatedDestination::Domain { name } => {
            for set in &sets {
                if let Some(entry) = set
                    .exact
                    .iter()
                    .find(|d| crate::domain_name_matches(name, d))
                    .or_else(|| {
                        set.suffixes
                            .iter()
                            .find(|s| crate::domain_suffix_matches(name, s))
                    })
                {
                    return Ok(found(set.key, entry));
                }
                other_kind |= !set.ips.is_empty();
            }
        }
        // Defined as the destination no rule names.
        EvaluatedDestination::Internet | EvaluatedDestination::Node { .. } => {}
    }
    Ok(if other_kind {
        Match::ResolutionDependent
    } else {
        Match::No
    })
}

fn route_of(
    kind_action: &RouteAction,
    rule: Option<String>,
    profile: Option<String>,
    rule_set: Option<RuleSetMatch>,
    resolution_dependent: Vec<String>,
) -> Result<RouteEvaluation, CompileError> {
    let (kind, targets, via) = match kind_action {
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

/// The first rule selecting the destination, with rules skipped for the other
/// destination kind appended to `skipped` as pointers under `prefix`.
fn first_match(
    request: &PlanRequest<'_>,
    destination: &EvaluatedDestination,
    rules: &[EgressRule],
    prefix: &str,
    skipped: &mut Vec<String>,
) -> Result<Option<(usize, Option<RuleSetMatch>)>, CompileError> {
    for (index, rule) in rules.iter().enumerate() {
        match plan_match(request, &rule.matcher, destination)? {
            Match::Yes(rule_set) => return Ok(Some((index, rule_set))),
            Match::ResolutionDependent => skipped.push(format!("{prefix}/{index}")),
            Match::No => {}
        }
    }
    Ok(None)
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
    let mut skipped = Vec::new();
    let public = match first_match(
        request,
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
                &rule.action.clone().into(),
                Some(format!("/routing/egress/rules/{index}")),
                None,
                rule_set,
                skipped,
            )?),
            None,
        ));
    }
    let mut name = profile.to_owned();
    let mut resolved = match matching::resolve_profile(cfg, profile) {
        Ok(resolved) => resolved,
        Err(CompileError::UnavailableDependency) => return Ok(missing_dependency()),
        Err(error) => return Err(error),
    };
    if let EgressProfile::Auto(auto) = resolved {
        // The plan knows no region: auto falls back, as compilation does
        // without a trusted region input.
        let assignment = matching::assignment(cfg, request.topology.subjects, request.source)?;
        if auto.fallback == "auto" || !assignment.allowed.contains(&auto.fallback) {
            return Err(CompileError::InvalidInput);
        }
        name = auto.fallback.clone();
        resolved = match matching::resolve_profile(cfg, &name) {
            Ok(resolved) => resolved,
            Err(CompileError::UnavailableDependency) => return Ok(missing_dependency()),
            Err(error) => return Err(error),
        };
    }
    let custom = match resolved {
        EgressProfile::Mesh => {
            return Ok((
                Some(route_of(
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
    let matched = match first_match(request, destination, &custom.rules, &prefix, &mut skipped) {
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
