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
    input: &CompilationInput<'_>,
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
        let (ip, domain, suffix) = match input.configuration.rule_sets.get(key) {
            Some(RuleSet::Inline {
                ip_cidr,
                domain,
                domain_suffix,
            }) => (ip_cidr, domain, domain_suffix),
            Some(RuleSet::Remote { .. }) => {
                let set = input
                    .rule_sets
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
    input: &'a CompilationInput<'_>,
    profile: &str,
) -> Result<EgressProfile<'a>, CompileError> {
    match matching::resolve_profile(input.configuration, profile)? {
        EgressProfile::Auto(auto) => {
            if auto.fallback == "auto" {
                return Err(CompileError::InvalidInput);
            }
            matching::resolve_profile(input.configuration, &auto.fallback)
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
    let generation = digest(&(
        input.network,
        node,
        input.revision,
        &profile,
        selection,
        &input.configuration.routing.egress,
    ))?;
    let mut plan = EgressCapturePlan {
        generation,
        managed_ipv4: Vec::new(),
        default_capture: false,
        direct_ipv4: Vec::new(),
        unmanaged_ipv4: Vec::new(),
        sink_ipv6: Vec::new(),
        unavailable: None,
    };
    if let Err(error) = compile_ranges(input, &profile, &mut plan) {
        plan.managed_ipv4 = vec!["0.0.0.0/0".into()];
        plan.default_capture = true;
        plan.direct_ipv4.clear();
        plan.unmanaged_ipv4.clear();
        plan.sink_ipv6 = vec!["::/0".into()];
        plan.unavailable = Some(match error {
            CompileError::Budget => UnavailableReason::RoutingLimit,
            CompileError::UnavailableDependency => UnavailableReason::MissingDependency,
            _ => UnavailableReason::EgressConflict,
        });
    }
    plan.generation = digest(&(
        generation,
        &plan.managed_ipv4,
        plan.default_capture,
        &plan.direct_ipv4,
        &plan.unmanaged_ipv4,
        &plan.sink_ipv6,
        plan.unavailable,
    ))?;
    Ok(plan)
}

fn append(plan: &mut EgressCapturePlan, nets: Vec<IpNet>, action: &RouteAction) {
    for net in nets {
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
    input: &CompilationInput<'_>,
    profile: &str,
    plan: &mut EgressCapturePlan,
) -> Result<(), CompileError> {
    let may_exit = !matches!(
        matching::dns_egress(input.configuration, profile),
        DnsEgress::NoExit
    );
    let mut rules: Vec<_> = input.configuration.routing.egress.rules.iter().collect();
    let fallback = match resolved(input, profile)? {
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
        for prefix in prefixes(input, &rule.matcher, may_exit)? {
            let mut rest = Vec::new();
            let mut hit = Vec::new();
            for net in remaining {
                subtract(net, prefix, &mut rest, &mut hit, &mut work)?;
            }
            total += hit.len();
            if total + rest.len() > MAX_CAPTURE_PREFIXES {
                return Err(CompileError::Budget);
            }
            append(plan, hit, &rule.action.clone().into());
            remaining = rest;
        }
    }
    plan.default_capture = matches!(fallback, RouteAction::Exit { .. })
        && remaining.iter().any(|net| net.addr().is_ipv4());
    append(plan, remaining, &fallback);
    plan.default_capture |= !plan.managed_ipv4.is_empty()
        && plan.direct_ipv4.is_empty()
        && plan.unmanaged_ipv4.is_empty();
    Ok(())
}
