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

/// Global-unicast approximation with explicit non-public special-use exclusions.
/// IPv4-mapped IPv6 uses the IPv4 classification instead of bypassing exclusions.
fn internet(address: IpAddr) -> bool {
    if let IpAddr::V6(v6) = address {
        if let Some(v4) = v6.to_ipv4_mapped() {
            return internet(v4.into());
        }
        return cidr("2000::/3", address)
            && !["2001::/23", "2001:db8::/32", "2002::/16", "3fff::/20"]
                .iter()
                .any(|s| cidr(s, address));
    }
    ![
        "0.0.0.0/8",
        "10.0.0.0/8",
        "100.64.0.0/10",
        "127.0.0.0/8",
        "169.254.0.0/16",
        "172.16.0.0/12",
        "192.0.0.0/24",
        "192.0.2.0/24",
        "192.168.0.0/16",
        "198.18.0.0/15",
        "198.51.100.0/24",
        "203.0.113.0/24",
        "224.0.0.0/3",
    ]
    .iter()
    .any(|s| cidr(s, address))
}
fn external_selector(input: &CompilationInput<'_>, s: &str, d: &Demand) -> bool {
    let address = match d.target {
        BusinessTarget::Ip { address } | BusinessTarget::Domain { address, .. } => address,
        _ => return false,
    };
    if s == "autogroup:internet" {
        return internet(address.0);
    }
    if let Some(suffix) = s.strip_prefix("domain:") {
        return input.dns.iter().any(|a| {
            a.source == d.source
                && a.expires_at_ms > input.now_ms
                && a.addresses.contains(&address)
                && domain_suffix_matches(&a.name, suffix)
        });
    }
    cidr(s, address.0)
}
pub(super) fn access(input: &CompilationInput<'_>, d: &Demand) -> bool {
    let cfg = input.configuration;
    cfg.access.iter().any(|r| {
        r.ip.iter().any(|s| ports(s, d))
            && (selectors(cfg, &r.src, d.source)
                && match d.target {
                    BusinessTarget::Mesh { node } => selectors(cfg, &r.dst, node),
                    _ => r.dst.iter().any(|s| external_selector(input, s, d)),
                }
                || r.bidirectional
                    && matches!(d.target, BusinessTarget::Mesh { node }
            if selectors(cfg,&r.dst,d.source) && selectors(cfg,&r.src,node)))
    })
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
fn matcher(input: &CompilationInput<'_>, d: &Demand, m: &RouteMatch) -> Result<bool, CompileError> {
    let address = match d.target {
        BusinessTarget::Ip { address } | BusinessTarget::Domain { address, .. } => address,
        _ => return Ok(false),
    };
    let domain = |suffix: &str| matches!(&d.target, BusinessTarget::Domain { name, .. } if domain_suffix_matches(name,suffix));
    let mut matched =
        m.ip_cidr.iter().any(|s| cidr(s, address.0)) || m.domain_suffix.iter().any(|s| domain(s));
    for key in &m.rule_sets {
        let (ips, domains) = match input.configuration.rule_sets.get(key) {
            Some(RuleSet::Inline {
                ip_cidr,
                domain_suffix,
            }) => (ip_cidr, domain_suffix),
            Some(RuleSet::Remote { .. }) => {
                let content = input.rule_sets.get(key).ok_or(CompileError::InvalidInput)?;
                (&content.ip_cidr, &content.domain_suffix)
            }
            None => return Err(CompileError::InvalidInput),
        };
        matched |= ips.iter().any(|s| cidr(s, address.0)) || domains.iter().any(|s| domain(s));
    }
    Ok(matched)
}
pub(super) fn egress_action(
    input: &CompilationInput<'_>,
    d: &Demand,
) -> Result<RouteAction, CompileError> {
    let e = &input.configuration.routing.egress;
    for r in &e.rules {
        if matcher(input, d, &r.matcher)? {
            return Ok(r.action.clone());
        }
    }
    let mut profile = d.profile.as_str();
    if profile == "auto" {
        // Region is a trusted external input, never guessed from controller IP.
        // No region input means the configured fallback, with the same allowed check.
        profile = match &e.auto {
            Input::Value(a) => &a.fallback,
            Input::Omitted => return Err(CompileError::InvalidInput),
        };
        if profile == "auto" || !profile_allowed(input.configuration, d.source, profile)? {
            return Err(CompileError::InvalidInput);
        }
    }
    if profile == "mesh" {
        return Ok(RouteAction::Unmanaged);
    }
    if let Some(id) = profile.strip_prefix("global:") {
        let node: NodeId = id.parse().map_err(|_| CompileError::InvalidInput)?;
        return Ok(RouteAction::Exit {
            nodes: vec![format!("node:{node}")],
            via: vec![],
        });
    }
    let p = e.profiles.get(profile).ok_or(CompileError::InvalidInput)?;
    for r in &p.rules {
        if matcher(input, d, &r.matcher)? {
            return Ok(r.action.clone());
        }
    }
    Ok(p.fallback.clone())
}
