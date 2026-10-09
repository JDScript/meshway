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

fn prefixes(
    cfg: &NetworkConfiguration,
    rule_sets: &BTreeMap<String, RuleSetContent>,
    matcher: &RouteMatch,
    may_exit: bool,
) -> Result<Vec<IpNet>, CompileError> {
    if matcher.ip_cidr.len() > MAX_CAPTURE_PREFIXES
        || matcher.rule_sets.len() > MAX_CAPTURE_PREFIXES
    {
        return Err(CompileError::Budget);
    }
    let mut ips = matcher.ip_cidr.clone();
    let mut has_names = !matcher.domain.is_empty() || !matcher.domain_suffix.is_empty();
    for key in &matcher.rule_sets {
        let (ip, domain, suffix) = match cfg.rule_sets.get(key) {
            Some(RuleSet::Inline {
                ip_cidr,
                domain,
                domain_suffix,
            }) => (ip_cidr, domain, domain_suffix),
            Some(RuleSet::Remote { .. }) => {
                let set = rule_sets
                    .get(key)
                    .ok_or(CompileError::UnavailableDependency)?;
                (&set.ip_cidr, &set.domain, &set.domain_suffix)
            }
            None => return Err(CompileError::UnavailableDependency),
        };
        if ips.len() + ip.len() > MAX_CAPTURE_PREFIXES {
            return Err(CompileError::Budget);
        }
        ips.extend(ip.iter().cloned());
        has_names |= !domain.is_empty() || !suffix.is_empty();
    }
    // An IP-only route table cannot represent the identity of a queried name.
    if has_names && may_exit {
        return Err(CompileError::InvalidInput);
    }
    ips.iter()
        .map(|ip| ip.parse().map_err(|_| CompileError::InvalidInput))
        .collect()
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

/// Compile ownership independently of Access or exit readiness. A failed
/// dependency retains a full negative boundary and publishes no local bypass.
pub fn egress_capture_plan(
    input: &CompilationInput<'_>,
    node: NodeId,
    selection: u64,
) -> Result<EgressCapturePlan, CompileError> {
    let profile = selected_profile(input, node)?;
    let seed = digest(&(
        input.network,
        node,
        input.revision,
        &profile,
        selection,
        &input.configuration.routing.egress,
    ))?;
    capture(input.configuration, input.rule_sets, &profile, seed).map(|(plan, _)| plan)
}

/// One disjoint prefix of the first-match partition with its selected action.
pub(super) type Route = (IpNet, RouteAction);

/// Shared by publication capture and netmap egress. Routes are empty when the
/// plan is unavailable; the generation binds `seed` and the resulting ranges.
pub(super) fn capture(
    cfg: &NetworkConfiguration,
    rule_sets: &BTreeMap<String, RuleSetContent>,
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
    rule_sets: &BTreeMap<String, RuleSetContent>,
    profile: &str,
    plan: &mut EgressCapturePlan,
    routes: &mut Vec<Route>,
) -> Result<(), CompileError> {
    let may_exit = !matches!(matching::dns_egress(cfg, profile), DnsEgress::NoExit);
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
        for prefix in prefixes(cfg, rule_sets, &rule.matcher, may_exit)? {
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
