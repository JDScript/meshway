use super::*;
use std::{collections::BTreeSet, net::IpAddr};

pub(super) fn node_selector(s: &str) -> Result<NodeId, CompileError> {
    s.strip_prefix("node:")
        .ok_or(CompileError::InvalidInput)?
        .parse()
        .map_err(|_| CompileError::InvalidInput)
}
/// Exit eligibility (enabled with `exitEnabled`) as the full configuration
/// defines it. A projection lacks Nodes outside the visible set, so a Node
/// reads the list the Server derived from the full configuration
/// (`ProjectedConfiguration::eligible_exits`); absence from a projection
/// never means ineligible.
#[derive(Clone, Copy)]
pub enum Exits<'a> {
    Full(&'a NetworkConfiguration),
    Projected(&'a [NodeId]),
}

impl Exits<'_> {
    pub fn eligible(self, node: NodeId) -> bool {
        match self {
            Self::Full(cfg) => cfg
                .nodes
                .get(&node)
                .is_some_and(|settings| settings.enabled && settings.exit_enabled),
            Self::Projected(exits) => exits.contains(&node),
        }
    }
}

/// What a projection carries for [`Exits::Projected`]: eligible exits listed
/// ahead of another Node in some exit set of `routing` (only those can stand
/// before a source naming itself), ascending.
pub(super) fn eligible_exits(cfg: &NetworkConfiguration, routing: &EgressRouting) -> Vec<NodeId> {
    let ahead = |nodes: &[String]| -> Vec<NodeId> {
        nodes
            .split_last()
            .map_or(&[][..], |(_, ahead)| ahead)
            .iter()
            .filter_map(|selector| node_selector(selector).ok())
            .collect()
    };
    let mut exits = BTreeSet::new();
    for rule in routing
        .rules
        .iter()
        .chain(routing.profiles.values().flat_map(|profile| &profile.rules))
    {
        if let RuleAction::Exit { nodes, .. } = &rule.action {
            exits.extend(ahead(nodes));
        }
    }
    for profile in routing.profiles.values() {
        if let RouteAction::Exit { nodes, .. } = &profile.fallback {
            exits.extend(ahead(nodes));
        }
    }
    exits
        .into_iter()
        .filter(|node| Exits::Full(cfg).eligible(*node))
        .collect()
}

/// The action `source` executes for `route`. Exit sets are ordered
/// primary/backup; the source itself in such a set is its own direct egress,
/// always available at its position: from the source on, nothing later is
/// tried. Before it, the set keeps, in order, only the exits the full
/// configuration makes eligible ([`Exits`]); with none, the source is
/// direct. `[self, other]` is direct; `[other, self]` exits through `other`
/// while it is eligible, else direct. Its `via` is then an empty segment.
/// Every executed partition, plan, DNS plan and preview of a source goes
/// through here; Access is unaffected. Runtime exit readiness and
/// reachability are not inputs: an eligible exit that is not ready, isolated
/// or not enrolled fails closed like any exit.
pub(super) fn executed(
    exits: Exits<'_>,
    route: RouteAction,
    source: NodeId,
) -> Result<RouteAction, CompileError> {
    let RouteAction::Exit { nodes, via } = route else {
        return Ok(route);
    };
    for (at, selector) in nodes.iter().enumerate() {
        if node_selector(selector)? != source {
            continue;
        }
        let before: Vec<String> = nodes[..at]
            .iter()
            .filter(|exit| node_selector(exit).is_ok_and(|exit| exits.eligible(exit)))
            .cloned()
            .collect();
        if before.is_empty() {
            return Ok(RouteAction::Direct);
        }
        return Ok(RouteAction::Exit { nodes: before, via });
    }
    Ok(RouteAction::Exit { nodes, via })
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
