use super::*;
use std::net::IpAddr;

pub(super) fn node_selector(s: &str) -> Result<NodeId, CompileError> {
    s.strip_prefix("node:")
        .ok_or(CompileError::InvalidInput)?
        .parse()
        .map_err(|_| CompileError::InvalidInput)
}
/// The action `source` executes for `route`. An exit set naming the source
/// itself is that source's own direct egress, wherever it stands in the set:
/// the source is the exit, its traffic leaves this host and its `via` is an
/// empty segment. Every executed partition, plan, DNS plan and preview of a
/// source goes through here; Access is unaffected.
pub(super) fn executed(route: RouteAction, source: NodeId) -> Result<RouteAction, CompileError> {
    if let RouteAction::Exit { nodes, .. } = &route {
        for selector in nodes {
            if node_selector(selector)? == source {
                return Ok(RouteAction::Direct);
            }
        }
    }
    Ok(route)
}
pub(super) fn selectors(
    cfg: &NetworkConfiguration,
    subjects: &SubjectContext,
    selectors: &[String],
    id: NodeId,
) -> bool {
    selectors
        .iter()
        .any(|selector| subjects.matches(cfg, selector, id))
}
pub(super) fn internet(address: IpAddr) -> bool {
    crate::is_internet_address(address)
}

/// Validated assignments: one default/locked pair, union of allowed profiles.
pub(super) struct Assignment {
    pub allowed: Vec<String>,
    pub default: String,
    pub locked: bool,
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
    for (i, assignment) in cfg
        .routing
        .egress
        .assignments
        .iter()
        .enumerate()
        .filter(|(_, a)| selectors(cfg, subjects, &a.target, node))
    {
        let pair = (assignment.default.clone(), assignment.locked);
        if selected.as_ref().is_some_and(|previous| previous != &pair) {
            return Err(CompileError::InvalidInput);
        }
        selected = Some(pair);
        index.get_or_insert(i);
        allowed.extend(assignment.allowed.clone());
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
pub(super) fn default_profile(
    cfg: &NetworkConfiguration,
    subjects: &SubjectContext,
    node: NodeId,
) -> Result<String, CompileError> {
    Ok(assignment(cfg, subjects, node)?.default)
}
pub fn profile_allowed(
    cfg: &NetworkConfiguration,
    subjects: &SubjectContext,
    node: NodeId,
    profile: &str,
) -> Result<bool, CompileError> {
    let assignment = assignment(cfg, subjects, node)?;
    Ok(assignment.allowed.iter().any(|p| p == profile)
        && (!assignment.locked || profile == assignment.default))
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
                id.parse()
                    .map(EgressProfile::Global)
                    .map_err(|_| CompileError::InvalidInput)
            } else {
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
}

/// Whether `source` executes `route` as an exit; an unparsable set counts.
fn may_exit(route: RouteAction, source: NodeId) -> bool {
    executed(route, source).map_or(true, |route| matches!(route, RouteAction::Exit { .. }))
}
fn rules_may_exit(rules: &[EgressRule], source: NodeId) -> bool {
    rules
        .iter()
        .any(|rule| may_exit(rule.action.clone().into(), source))
}
fn non_auto_may_exit(profile: EgressProfile<'_>, source: NodeId) -> bool {
    match profile {
        EgressProfile::Mesh => false,
        EgressProfile::Global(node) => node != source,
        EgressProfile::Auto(_) => true,
        EgressProfile::Custom(profile) => {
            rules_may_exit(&profile.rules, source) || may_exit(profile.fallback.clone(), source)
        }
    }
}

/// Conservative classification for `source`, including shadowed exits and
/// unknown dependencies. Neither runtime facts nor an answer can relax it;
/// an exit naming the source itself is its direct egress ([`executed`]).
pub(super) fn dns_requires_exit(
    config: &NetworkConfiguration,
    profile: &str,
    source: NodeId,
) -> bool {
    rules_may_exit(&config.routing.egress.rules, source)
        || match resolve_profile(config, profile) {
            Ok(EgressProfile::Auto(auto)) => auto
                .rules
                .iter()
                .map(|rule| &rule.profile)
                .chain(std::iter::once(&auto.fallback))
                .any(|choice| {
                    resolve_profile(config, choice)
                        .map_or(true, |profile| non_auto_may_exit(profile, source))
                }),
            Ok(profile) => non_auto_may_exit(profile, source),
            Err(_) => true,
        }
}
