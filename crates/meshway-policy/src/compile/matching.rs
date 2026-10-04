use super::*;
use ipnet::IpNet;
use std::net::IpAddr;

pub(super) fn node_selector(s: &str) -> Result<NodeId, CompileError> {
    s.strip_prefix("node:")
        .ok_or(CompileError::InvalidInput)?
        .parse()
        .map_err(|_| CompileError::InvalidInput)
}
pub(super) fn selectors(
    cfg: &NetworkConfiguration,
    subjects: &SubjectContext,
    ss: &[String],
    id: NodeId,
) -> bool {
    ss.iter().any(|s| subjects.matches(cfg, s, id))
}
fn cidr(s: &str, address: IpAddr) -> bool {
    s.parse::<IpNet>().is_ok_and(|n| n.contains(&address))
}

/// Use the same address-set definition as configuration validation. IPv4-mapped
/// IPv6 uses the IPv4 classification instead of bypassing exclusions.
fn internet(address: IpAddr) -> bool {
    if let IpAddr::V6(v6) = address
        && let Some(v4) = v6.to_ipv4_mapped()
    {
        return internet(v4.into());
    }
    meshway_configuration::internet_ranges()
        .iter()
        .any(|range| range.contains(&address))
}
fn domain_expiry(input: &CompilationInput<'_>, d: &Demand, suffix: &str) -> Option<u64> {
    let address = match d.target {
        BusinessTarget::Ip { address } | BusinessTarget::Domain { address, .. } => address,
        _ => return None,
    };
    input
        .dns
        .iter()
        .filter(|a| {
            a.source == d.source
                && a.expires_at_ms > input.now_ms
                && a.addresses.contains(&address)
                && domain_suffix_matches(&a.name, suffix)
        })
        .map(|a| a.expires_at_ms.min(d.expires_at_ms))
        .max()
}
fn external_selector(input: &CompilationInput<'_>, s: &str, d: &Demand) -> Option<u64> {
    let address = match d.target {
        BusinessTarget::Ip { address } | BusinessTarget::Domain { address, .. } => address,
        _ => return None,
    };
    if s == "autogroup:internet" {
        return internet(address.0).then_some(d.expires_at_ms);
    }
    if let Some(suffix) = s.strip_prefix("domain:") {
        return domain_expiry(input, d, suffix);
    }
    cidr(s, address.0).then_some(d.expires_at_ms)
}
/// Grants are a union. A DNS-backed grant may only outlive its association when
/// another independently matching grant authorizes that same traffic longer.
pub(super) fn access_expiry(input: &CompilationInput<'_>, d: &Demand) -> Option<u64> {
    let cfg = input.configuration;
    let traffic = crate::access::TrafficQuery::Exact {
        protocol: d.protocol,
        port: d.port,
    };
    if let BusinessTarget::Mesh { node } = d.target {
        return crate::access::allows_mesh(cfg, input.subjects, d.source, node, traffic)
            .then_some(d.expires_at_ms);
    }
    cfg.access
        .iter()
        .filter(|r| traffic.matches(&r.ip))
        .filter_map(|r| {
            if !selectors(cfg, input.subjects, &r.src, d.source) {
                return None;
            }
            r.dst
                .iter()
                .filter_map(|s| external_selector(input, s, d))
                .max()
        })
        .max()
}
fn profile_assignment(
    cfg: &NetworkConfiguration,
    subjects: &SubjectContext,
    node: NodeId,
) -> Result<(Vec<String>, String, bool), CompileError> {
    let mut allowed = Vec::new();
    let mut selected = None;
    for a in cfg
        .routing
        .egress
        .assignments
        .iter()
        .filter(|a| selectors(cfg, subjects, &a.target, node))
    {
        let pair = (a.default.clone(), a.locked);
        if selected.as_ref().is_some_and(|p| p != &pair) {
            return Err(CompileError::InvalidInput);
        }
        selected = Some(pair);
        allowed.extend(a.allowed.clone());
    }
    let (default, locked) = selected.unwrap_or_else(|| {
        allowed.push("mesh".into());
        ("mesh".into(), false)
    });
    if !allowed.contains(&default) {
        return Err(CompileError::InvalidInput);
    }
    Ok((allowed, default, locked))
}
pub(super) fn default_profile(
    cfg: &NetworkConfiguration,
    subjects: &SubjectContext,
    node: NodeId,
) -> Result<String, CompileError> {
    profile_assignment(cfg, subjects, node).map(|(_, d, _)| d)
}
pub fn profile_allowed(
    cfg: &NetworkConfiguration,
    subjects: &SubjectContext,
    node: NodeId,
    profile: &str,
) -> Result<bool, CompileError> {
    let (allowed, default, locked) = profile_assignment(cfg, subjects, node)?;
    Ok(allowed.iter().any(|p| p == profile) && (!locked || profile == default))
}
fn matcher(
    input: &CompilationInput<'_>,
    d: &Demand,
    m: &RouteMatch,
) -> Result<Option<u64>, CompileError> {
    let address = match d.target {
        BusinessTarget::Ip { address } | BusinessTarget::Domain { address, .. } => address,
        _ => return Ok(None),
    };
    let domain = |suffix: &str| domain_expiry(input, d, suffix);
    let mut matched = m
        .ip_cidr
        .iter()
        .any(|s| cidr(s, address.0))
        .then_some(d.expires_at_ms)
        .max(m.domain_suffix.iter().filter_map(|s| domain(s)).max());
    for key in &m.rule_sets {
        let (ips, domains) = match input.configuration.rule_sets.get(key) {
            Some(RuleSet::Inline {
                ip_cidr,
                domain_suffix,
            }) => (ip_cidr, domain_suffix),
            Some(RuleSet::Remote { .. }) => {
                let content = input
                    .rule_sets
                    .get(key)
                    .ok_or(CompileError::UnavailableDependency)?;
                (&content.ip_cidr, &content.domain_suffix)
            }
            None => return Err(CompileError::UnavailableDependency),
        };
        matched = matched
            .max(
                ips.iter()
                    .any(|s| cidr(s, address.0))
                    .then_some(d.expires_at_ms),
            )
            .max(domains.iter().filter_map(|s| domain(s)).max());
    }
    Ok(matched)
}
pub(super) fn egress_action(
    input: &CompilationInput<'_>,
    d: &Demand,
) -> Result<(RouteAction, u64), CompileError> {
    let e = &input.configuration.routing.egress;
    for r in &e.rules {
        if let Some(expires) = matcher(input, d, &r.matcher)? {
            return Ok((r.action.clone(), expires));
        }
    }
    let mut profile = d.profile.as_str();
    if profile == "auto" {
        // Region is a trusted external input, never guessed from controller IP.
        // No region input means the configured fallback, with the same allowed check.
        profile = match &e.auto {
            Input::Value(a) => &a.fallback,
            Input::Omitted => return Err(CompileError::UnavailableDependency),
        };
        if profile == "auto"
            || !profile_allowed(input.configuration, input.subjects, d.source, profile)?
        {
            return Err(CompileError::InvalidInput);
        }
    }
    if profile == "mesh" {
        return Ok((RouteAction::Unmanaged, d.expires_at_ms));
    }
    if let Some(id) = profile.strip_prefix("global:") {
        let node: NodeId = id.parse().map_err(|_| CompileError::InvalidInput)?;
        return Ok((
            RouteAction::Exit {
                nodes: vec![format!("node:{node}")],
                via: vec![],
            },
            d.expires_at_ms,
        ));
    }
    let p = e
        .profiles
        .get(profile)
        .ok_or(CompileError::UnavailableDependency)?;
    for r in &p.rules {
        if let Some(expires) = matcher(input, d, &r.matcher)? {
            return Ok((r.action.clone(), expires));
        }
    }
    Ok((p.fallback.clone(), d.expires_at_ms))
}
