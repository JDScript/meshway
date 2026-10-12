use super::*;
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

fn rules_may_exit(rules: &[EgressRule]) -> bool {
    rules
        .iter()
        .any(|rule| matches!(rule.action, RuleAction::Exit { .. }))
}
fn non_auto_may_exit(profile: EgressProfile<'_>) -> bool {
    match profile {
        EgressProfile::Mesh => false,
        EgressProfile::Global(_) | EgressProfile::Auto(_) => true,
        EgressProfile::Custom(profile) => {
            rules_may_exit(&profile.rules) || matches!(profile.fallback, RouteAction::Exit { .. })
        }
    }
}

/// Conservative classification, including shadowed exits and unknown
/// dependencies. Neither runtime facts nor an answer can relax it.
pub(super) fn dns_requires_exit(config: &NetworkConfiguration, profile: &str) -> bool {
    rules_may_exit(&config.routing.egress.rules)
        || match resolve_profile(config, profile) {
            Ok(EgressProfile::Auto(auto)) => auto
                .rules
                .iter()
                .map(|rule| &rule.profile)
                .chain(std::iter::once(&auto.fallback))
                .any(|choice| resolve_profile(config, choice).map_or(true, non_auto_may_exit)),
            Ok(profile) => non_auto_may_exit(profile),
            Err(_) => true,
        }
}
