//! First-match packet ownership expressed as disjoint longest-prefix routes.
use super::{matching::EgressProfile, *};
use ipnet::IpNet;

pub const MAX_CAPTURE_PREFIXES: usize = 4096;
const MAX_CAPTURE_WORK: usize = 131_072;

fn subtract(
    net: IpNet,
    cut: IpNet,
    rest: &mut Vec<IpNet>,
    hit: &mut Vec<IpNet>,
    work: &mut usize,
) -> Result<(), CompileError> {
    *work += 1;
    if *work > MAX_CAPTURE_WORK || rest.len() + hit.len() > MAX_CAPTURE_PREFIXES {
        return Err(CompileError::Budget);
    }
    if cut.contains(&net) {
        hit.push(net);
    } else if net.contains(&cut) {
        for child in net
            .subnets(net.prefix_len() + 1)
            .map_err(|_| CompileError::InvalidInput)?
        {
            subtract(child, cut, rest, hit, work)?;
        }
    } else {
        rest.push(net);
    }
    Ok(())
}

/// The IP part of one rule. Its names own no address: they decide only the
/// addresses of a trusted association (see `netmap::Associations`).
fn prefixes(sets: &rules::RuleSets, matcher: &RouteMatch) -> Result<Vec<IpNet>, CompileError> {
    if matcher.ip_cidr.len() > MAX_CAPTURE_PREFIXES
        || matcher.rule_sets.len() > MAX_CAPTURE_PREFIXES
    {
        return Err(CompileError::Budget);
    }
    Ok(rules::prefixes(matcher, sets, MAX_CAPTURE_PREFIXES)?.0)
}

fn resolved<'a>(
    cfg: &'a NetworkConfiguration,
    profile: &str,
) -> Result<EgressProfile<'a>, CompileError> {
    match matching::resolve_profile(cfg, profile)? {
        EgressProfile::Auto(auto) => {
            if auto.fallback == "auto" {
                return Err(CompileError::InvalidInput);
            }
            matching::resolve_profile(cfg, &auto.fallback)
        }
        other => Ok(other),
    }
}

/// One disjoint prefix of the first-match partition with its selected action.
pub(super) type Route = (IpNet, RouteAction);

/// Netmap packet ownership, independent of Access or readiness. Routes are empty when the
/// plan is unavailable; the generation binds `seed` and the resulting ranges.
pub(super) fn capture(
    cfg: &NetworkConfiguration,
    rule_sets: &rules::RuleSets,
    profile: &str,
    seed: meshway_model::Digest,
) -> Result<(EgressCapturePlan, Vec<Route>), CompileError> {
    let mut plan = EgressCapturePlan {
        generation: seed,
        managed_ipv4: Vec::new(),
        default_capture: false,
        direct_ipv4: Vec::new(),
        unmanaged_ipv4: Vec::new(),
        sink_ipv6: Vec::new(),
        unavailable: None,
    };
    let mut routes = Vec::new();
    if let Err(error) = compile_ranges(cfg, rule_sets, profile, &mut plan, &mut routes) {
        plan.managed_ipv4 = vec!["0.0.0.0/0".into()];
        plan.default_capture = true;
        plan.direct_ipv4.clear();
        plan.unmanaged_ipv4.clear();
        plan.sink_ipv6 = vec!["::/0".into()];
        routes.clear();
        plan.unavailable = Some(match error {
            CompileError::Budget => UnavailableReason::RoutingLimit,
            CompileError::UnavailableDependency => UnavailableReason::MissingDependency,
            _ => UnavailableReason::EgressConflict,
        });
    }
    plan.generation = digest(&(
        seed,
        &plan.managed_ipv4,
        plan.default_capture,
        &plan.direct_ipv4,
        &plan.unmanaged_ipv4,
        &plan.sink_ipv6,
        plan.unavailable,
    ))?;
    Ok((plan, routes))
}

fn append(
    plan: &mut EgressCapturePlan,
    routes: &mut Vec<Route>,
    nets: Vec<IpNet>,
    action: &RouteAction,
) {
    for net in nets {
        routes.push((net, action.clone()));
        let out = match (net, action) {
            (IpNet::V4(_), RouteAction::Exit { .. }) => &mut plan.managed_ipv4,
            (IpNet::V4(_), RouteAction::Direct) => &mut plan.direct_ipv4,
            (IpNet::V4(_), RouteAction::Unmanaged) => &mut plan.unmanaged_ipv4,
            (IpNet::V6(_), RouteAction::Exit { .. }) => &mut plan.sink_ipv6,
            _ => continue,
        };
        out.push(net.to_string());
    }
}

fn compile_ranges(
    cfg: &NetworkConfiguration,
    rule_sets: &rules::RuleSets,
    profile: &str,
    plan: &mut EgressCapturePlan,
    routes: &mut Vec<Route>,
) -> Result<(), CompileError> {
    let mut rules: Vec<_> = cfg.routing.egress.rules.iter().collect();
    let fallback = match resolved(cfg, profile)? {
        EgressProfile::Mesh => RouteAction::Unmanaged,
        EgressProfile::Global(node) => RouteAction::Exit {
            nodes: vec![format!("node:{node}")],
            via: vec![],
        },
        EgressProfile::Custom(profile) => {
            rules.extend(&profile.rules);
            profile.fallback.clone()
        }
        EgressProfile::Auto(_) => return Err(CompileError::InvalidInput),
    };
    let mut remaining: Vec<IpNet> = ["0.0.0.0/0", "::/0"]
        .iter()
        .map(|ip| ip.parse().expect("constant prefix"))
        .collect();
    let mut work = 0;
    let mut total = 0;
    for rule in rules {
        if remaining.is_empty() {
            break;
        }
        for prefix in prefixes(rule_sets, &rule.matcher)? {
            let mut rest = Vec::new();
            let mut hit = Vec::new();
            for net in remaining {
                subtract(net, prefix, &mut rest, &mut hit, &mut work)?;
            }
            total += hit.len();
            if total + rest.len() > MAX_CAPTURE_PREFIXES {
                return Err(CompileError::Budget);
            }
            append(plan, routes, hit, &rule.action.clone().into());
            remaining = rest;
        }
    }
    plan.default_capture = matches!(fallback, RouteAction::Exit { .. })
        && remaining.iter().any(|net| net.addr().is_ipv4());
    append(plan, routes, remaining, &fallback);
    plan.default_capture |= !plan.managed_ipv4.is_empty()
        && plan.direct_ipv4.is_empty()
        && plan.unmanaged_ipv4.is_empty();
    Ok(())
}
