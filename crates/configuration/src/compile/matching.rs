use super::*;
use ipnet::IpNet;
use std::net::IpAddr;

pub(super) fn node_selector(s: &str) -> Result<NodeId, CompileError> {
    s.strip_prefix("node:")
        .ok_or(CompileError::InvalidInput)?
        .parse()
        .map_err(|_| CompileError::InvalidInput)
}
pub(super) fn selector(cfg: &NetworkConfiguration, s: &str, id: NodeId) -> bool {
    let Some(n) = cfg.nodes.get(&id) else {
        return false;
    };
    s == "*"
        || node_selector(s).ok() == Some(id)
        || match &n.identity {
            NodeIdentity::User { user } => {
                s == user || cfg.groups.get(s).is_some_and(|g| g.contains(user))
            }
            NodeIdentity::Tag { tags } => tags.iter().any(|t| t == s),
        }
}
pub(super) fn selectors(cfg: &NetworkConfiguration, ss: &[String], id: NodeId) -> bool {
    ss.iter().any(|s| selector(cfg, s, id))
}
fn ports(s: &str, d: &Demand) -> bool {
    if s == "*" {
        return true;
    }
    let Some((protocol, port)) = s.split_once(':') else {
        return false;
    };
    if !matches!(
        (protocol, d.protocol),
        ("tcp", BusinessProtocol::Tcp)
            | ("udp", BusinessProtocol::Udp)
            | ("icmp", BusinessProtocol::IcmpEcho)
    ) {
        return false;
    }
    if port == "*" {
        return true;
    }
    let Some(p) = d.port else { return false };
    let p = p.get();
    match port.split_once('-') {
        Some((a, b)) => a
            .parse::<u16>()
            .ok()
            .zip(b.parse::<u16>().ok())
            .is_some_and(|(a, b)| a <= p && p <= b),
        None => port.parse::<u16>() == Ok(p),
    }
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
    crate::internet_ranges()
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
    cfg.access
        .iter()
        .filter(|r| r.ip.iter().any(|s| ports(s, d)))
        .filter_map(|r| {
            if r.bidirectional
                && matches!(d.target, BusinessTarget::Mesh { node }
            if selectors(cfg,&r.dst,d.source) && selectors(cfg,&r.src,node))
            {
                return Some(d.expires_at_ms);
            }
            if !selectors(cfg, &r.src, d.source) {
                return None;
            }
            match d.target {
                BusinessTarget::Mesh { node } => {
                    selectors(cfg, &r.dst, node).then_some(d.expires_at_ms)
                }
                _ => r
                    .dst
                    .iter()
                    .filter_map(|s| external_selector(input, s, d))
                    .max(),
            }
        })
        .max()
}
fn profile_assignment(
    cfg: &NetworkConfiguration,
    node: NodeId,
) -> Result<(Vec<String>, String, bool), CompileError> {
    let mut allowed = Vec::new();
    let mut selected = None;
    for a in cfg
        .routing
        .egress
        .assignments
        .iter()
        .filter(|a| selectors(cfg, &a.target, node))
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
    node: NodeId,
) -> Result<String, CompileError> {
    profile_assignment(cfg, node).map(|(_, d, _)| d)
}
pub fn profile_allowed(
    cfg: &NetworkConfiguration,
    node: NodeId,
    profile: &str,
) -> Result<bool, CompileError> {
    let (allowed, default, locked) = profile_assignment(cfg, node)?;
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
        if profile == "auto" || !profile_allowed(input.configuration, d.source, profile)? {
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
