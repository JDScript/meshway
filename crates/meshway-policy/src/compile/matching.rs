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
pub(super) fn cidr(s: &str, address: IpAddr) -> bool {
    s.parse::<IpNet>().is_ok_and(|n| n.contains(&address))
}

/// Use the same address-set definition as configuration validation. IPv4-mapped
/// IPv6 uses the IPv4 classification instead of bypassing exclusions.
pub(super) fn internet(address: IpAddr) -> bool {
    if let IpAddr::V6(v6) = address
        && let Some(v4) = v6.to_ipv4_mapped()
    {
        return internet(v4.into());
    }
    meshway_configuration::internet_ranges()
        .iter()
        .any(|range| range.contains(&address))
}
/// A domain selector matches through the source's live DNS associations whose
/// name satisfies `matches` and whose addresses include the demand target.
fn domain_expiry(
    input: &CompilationInput<'_>,
    d: &Demand,
    matches: impl Fn(&str) -> bool,
) -> Option<u64> {
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
                && matches(&a.name)
        })
        .map(|a| a.expires_at_ms.min(d.expires_at_ms))
        .max()
}
fn suffix_expiry(input: &CompilationInput<'_>, d: &Demand, suffix: &str) -> Option<u64> {
    domain_expiry(input, d, |name| domain_suffix_matches(name, suffix))
}
fn exact_expiry(input: &CompilationInput<'_>, d: &Demand, domain: &str) -> Option<u64> {
    domain_expiry(input, d, |name| domain_name_matches(name, domain))
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
        return suffix_expiry(input, d, suffix);
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
/// The profile assignments that apply to one node, as validation guaranteed
/// them consistent: one default/locked pair, the union of allowed profiles.
pub(super) struct Assignment {
    pub allowed: Vec<String>,
    pub default: String,
    pub locked: bool,
    /// The first applying assignment; None when only the built-in `mesh` applies.
    pub index: Option<usize>,
}
pub(super) fn assignment(
    cfg: &NetworkConfiguration,
    subjects: &SubjectContext,
    node: NodeId,
) -> Result<Assignment, CompileError> {
    let mut allowed = Vec::new();
    let mut selected = None;
    let mut index = None;
    for (i, a) in cfg
        .routing
        .egress
        .assignments
        .iter()
        .enumerate()
        .filter(|(_, a)| selectors(cfg, subjects, &a.target, node))
    {
        let pair = (a.default.clone(), a.locked);
        if selected.as_ref().is_some_and(|p| p != &pair) {
            return Err(CompileError::InvalidInput);
        }
        selected = Some(pair);
        index.get_or_insert(i);
        allowed.extend(a.allowed.clone());
    }
    let (default, locked) = selected.unwrap_or_else(|| {
        allowed.push("mesh".into());
        ("mesh".into(), false)
    });
    if !allowed.contains(&default) {
        return Err(CompileError::InvalidInput);
    }
    Ok(Assignment {
        allowed,
        default,
        locked,
        index,
    })
}
fn profile_assignment(
    cfg: &NetworkConfiguration,
    subjects: &SubjectContext,
    node: NodeId,
) -> Result<(Vec<String>, String, bool), CompileError> {
    let Assignment {
        allowed,
        default,
        locked,
        ..
    } = assignment(cfg, subjects, node)?;
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
    let destinations = |ips: &[String], suffixes: &[String], exact: &[String]| {
        ips.iter()
            .any(|s| cidr(s, address.0))
            .then_some(d.expires_at_ms)
            .max(
                suffixes
                    .iter()
                    .filter_map(|s| suffix_expiry(input, d, s))
                    .max(),
            )
            .max(exact.iter().filter_map(|s| exact_expiry(input, d, s)).max())
    };
    let mut matched = destinations(&m.ip_cidr, &m.domain_suffix, &m.domain);
    for key in &m.rule_sets {
        let (ips, suffixes, exact) = match input.configuration.rule_sets.get(key) {
            Some(RuleSet::Inline {
                ip_cidr,
                domain_suffix,
                domain,
            }) => (ip_cidr, domain_suffix, domain),
            Some(RuleSet::Remote { .. }) => {
                let content = input
                    .rule_sets
                    .get(key)
                    .ok_or(CompileError::UnavailableDependency)?;
                (&content.ip_cidr, &content.domain_suffix, &content.domain)
            }
            None => return Err(CompileError::UnavailableDependency),
        };
        matched = matched.max(destinations(ips, suffixes, exact));
    }
    Ok(matched)
}
pub(super) enum EgressProfile<'a> {
    Mesh,
    Global(NodeId),
    Auto(&'a AutoProfile),
    Custom(&'a RoutingProfile),
}

pub(super) fn resolve_profile<'a>(
    config: &'a NetworkConfiguration,
    profile: &str,
) -> Result<EgressProfile<'a>, CompileError> {
    match profile {
        "mesh" => Ok(EgressProfile::Mesh),
        "auto" => match &config.routing.egress.auto {
            Input::Value(auto) => Ok(EgressProfile::Auto(auto)),
            Input::Omitted => Err(CompileError::UnavailableDependency),
        },
        _ => {
            if let Some(id) = profile.strip_prefix("global:") {
                return id
                    .parse()
                    .map(EgressProfile::Global)
                    .map_err(|_| CompileError::InvalidInput);
            }
            config
                .routing
                .egress
                .profiles
                .get(profile)
                .map(EgressProfile::Custom)
                .ok_or(CompileError::UnavailableDependency)
        }
    }
}

fn rules_may_exit(rules: &[EgressRule]) -> bool {
    rules.iter().any(|rule| match &rule.action {
        RuleAction::Exit { .. } => true,
        RuleAction::Direct => false,
    })
}

fn non_auto_may_exit(profile: EgressProfile<'_>) -> bool {
    match profile {
        EgressProfile::Mesh => false,
        EgressProfile::Global(_) => true,
        EgressProfile::Custom(profile) => {
            rules_may_exit(&profile.rules)
                || match &profile.fallback {
                    RouteAction::Exit { .. } => true,
                    RouteAction::Direct | RouteAction::Unmanaged => false,
                }
        }
        // Validation rejects auto -> auto. Never claim a proof for a cycle.
        EgressProfile::Auto(_) => true,
    }
}

/// Conservative classification over the complete source-effective routing
/// configuration, independent of demands, DNS samples and runtime inventory.
/// Shadowed exits still require a plan. Unknown dependencies are not a proof.
pub(super) fn dns_egress(config: &NetworkConfiguration, profile: &str) -> DnsEgress {
    let requires_plan = rules_may_exit(&config.routing.egress.rules)
        || match resolve_profile(config, profile) {
            Ok(EgressProfile::Auto(auto)) => auto
                .rules
                .iter()
                .map(|rule| &rule.profile)
                .chain(std::iter::once(&auto.fallback))
                .any(|choice| resolve_profile(config, choice).map_or(true, non_auto_may_exit)),
            Ok(profile) => non_auto_may_exit(profile),
            Err(_) => true,
        };
    if requires_plan {
        DnsEgress::Unavailable {
            reason: DnsEgressUnavailable::ExitPlanRequired,
        }
    } else {
        DnsEgress::NoExit
    }
}

pub(super) fn egress_action(
    input: &CompilationInput<'_>,
    d: &Demand,
) -> Result<(RouteAction, u64), CompileError> {
    let e = &input.configuration.routing.egress;
    for r in &e.rules {
        if let Some(expires) = matcher(input, d, &r.matcher)? {
            return Ok((r.action.clone().into(), expires));
        }
    }
    let mut profile = resolve_profile(input.configuration, &d.profile)?;
    if let EgressProfile::Auto(auto) = profile {
        // Region is a trusted external input, never guessed from controller IP.
        // No region input means the configured fallback, with the same allowed check.
        // Locking constrains the user's selected profile, not auto's internal
        // choice. Every choice must still belong to the same allowed union.
        let (allowed, _, _) = profile_assignment(input.configuration, input.subjects, d.source)?;
        if auto.fallback == "auto" || !allowed.contains(&auto.fallback) {
            return Err(CompileError::InvalidInput);
        }
        profile = resolve_profile(input.configuration, &auto.fallback)?;
    }
    let p = match profile {
        EgressProfile::Mesh => return Ok((RouteAction::Unmanaged, d.expires_at_ms)),
        EgressProfile::Global(node) => {
            return Ok((
                RouteAction::Exit {
                    nodes: vec![format!("node:{node}")],
                    via: vec![],
                },
                d.expires_at_ms,
            ));
        }
        EgressProfile::Custom(profile) => profile,
        EgressProfile::Auto(_) => return Err(CompileError::InvalidInput),
    };
    for r in &p.rules {
        if let Some(expires) = matcher(input, d, &r.matcher)? {
            return Ok((r.action.clone().into(), expires));
        }
    }
    Ok((p.fallback.clone(), d.expires_at_ms))
}
