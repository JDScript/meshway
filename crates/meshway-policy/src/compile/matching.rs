use super::*;
use meshway_configuration::netmap::EligibleExit;
use meshway_model::exit::ExitFamily;
use std::{collections::BTreeSet, net::IpAddr};

pub(super) fn node_selector(s: &str) -> Result<NodeId, CompileError> {
    s.strip_prefix("node:")
        .ok_or(CompileError::InvalidInput)?
        .parse()
        .map_err(|_| CompileError::InvalidInput)
}
/// Exit eligibility (enabled with `exitEnabled`) and the families an exit
/// carries (`exitFamilies`) as the full configuration defines them. A
/// projection lacks Nodes outside the visible set, so a Node reads the list
/// the Server derived from the full configuration
/// (`ProjectedConfiguration::eligible_exits`); absence from a projection
/// never means ineligible.
#[derive(Clone, Copy)]
pub enum Exits<'a> {
    Full(&'a NetworkConfiguration),
    Projected(&'a [EligibleExit]),
}

impl Exits<'_> {
    /// Whether `node` is an eligible exit carrying `family`.
    pub fn eligible(self, node: NodeId, family: ExitFamily) -> bool {
        match self {
            Self::Full(cfg) => cfg.nodes.get(&node).is_some_and(|settings| {
                settings.enabled
                    && settings.exit_enabled
                    && settings.exit_families.contains(&family)
            }),
            Self::Projected(exits) => exits
                .iter()
                .any(|exit| exit.node == node && exit.families.contains(&family)),
        }
    }
}

/// What a projection carries for [`Exits::Projected`]: every eligible exit
/// the retained `routing` names (exit sets, `global:<node>` profiles of its
/// assignments and `auto`, and `selected` profiles), ascending, with its
/// configured families.
pub(super) fn eligible_exits<'a>(
    cfg: &NetworkConfiguration,
    routing: &'a EgressRouting,
    selected: impl IntoIterator<Item = &'a str>,
) -> Vec<EligibleExit> {
    let mut named = BTreeSet::new();
    let mut exit_set = |nodes: &[String]| {
        named.extend(
            nodes
                .iter()
                .filter_map(|selector| node_selector(selector).ok()),
        );
    };
    for rule in routing
        .rules
        .iter()
        .chain(routing.profiles.values().flat_map(|profile| &profile.rules))
    {
        if let RuleAction::Exit { nodes, .. } = &rule.action {
            exit_set(nodes);
        }
    }
    for profile in routing.profiles.values() {
        if let RouteAction::Exit { nodes, .. } = &profile.fallback {
            exit_set(nodes);
        }
    }
    let mut profiles: Vec<&str> = routing
        .assignments
        .iter()
        .flat_map(|assignment| assignment.allowed.iter().chain([&assignment.default]))
        .map(String::as_str)
        .chain(selected)
        .collect();
    if let Input::Value(auto) = &routing.auto {
        profiles.extend(auto.rules.iter().map(|rule| rule.profile.as_str()));
        profiles.push(auto.fallback.as_str());
    }
    named.extend(
        profiles
            .into_iter()
            .filter_map(|profile| profile.strip_prefix("global:")?.parse::<NodeId>().ok()),
    );
    named
        .into_iter()
        .filter_map(|node| {
            let settings = cfg.nodes.get(&node)?;
            (settings.enabled && settings.exit_enabled).then(|| EligibleExit {
                node,
                families: settings.exit_families.clone(),
            })
        })
        .collect()
}

/// The action `source` executes for `route` toward destinations of
/// `family`. Exit sets are ordered primary/backup; the source itself in such
/// a set is its own direct egress, always available at its position: from
/// the source on, nothing later is tried. Before it, the set keeps, in
/// order, only the exits the full configuration makes eligible for `family`
/// ([`Exits`]); with none, the source is direct. `[self, other]` is direct;
/// `[other, self]` exits through `other` while it is eligible for the
/// family, else direct. Its `via` is then an empty segment. Every executed
/// partition, plan, DNS plan and preview of a source goes through here;
/// Access is unaffected. Runtime exit readiness and reachability are not
/// inputs: an eligible exit that is not ready, isolated or not enrolled
/// fails closed like any exit.
pub(super) fn executed(
    exits: Exits<'_>,
    route: RouteAction,
    source: NodeId,
    family: ExitFamily,
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
            .filter(|exit| node_selector(exit).is_ok_and(|exit| exits.eligible(exit, family)))
            .cloned()
            .collect();
        if before.is_empty() {
            return Ok(RouteAction::Direct);
        }
        return Ok(RouteAction::Exit { nodes: before, via });
    }
    Ok(RouteAction::Exit { nodes, via })
}

/// Whether some exit of `nodes` carries `family` by configuration: an IPv6
/// destination of an exit range none of whose exits carries IPv6 is sunk
/// statically, never handed to the packet owner.
pub(super) fn carries(exits: Exits<'_>, nodes: &[NodeId], family: ExitFamily) -> bool {
    nodes.iter().any(|node| exits.eligible(*node, family))
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
/// What `auto` applies for `source`: the first rule listing its declared
/// region, else the fallback. Without a source (a profile-level preview) no
/// region is known. The only `auto` resolution; every plan, decision,
/// selection view and selection identity uses it.
pub(super) fn auto_resolution(
    config: &NetworkConfiguration,
    auto: &AutoProfile,
    source: Option<NodeId>,
) -> meshway_configuration::selection::AutoResolution {
    use meshway_configuration::selection::{AutoBasis, AutoResolution};
    let region = source
        .and_then(|node| config.nodes.get(&node))
        .and_then(|settings| settings.region.clone());
    let Some(region) = region else {
        return AutoResolution {
            profile: auto.fallback.clone(),
            basis: AutoBasis::NoTrustedRegion,
            region: None,
        };
    };
    match auto
        .rules
        .iter()
        .find(|rule| rule.countries.contains(&region))
    {
        Some(rule) => AutoResolution {
            profile: rule.profile.clone(),
            basis: AutoBasis::Label,
            region: Some(region),
        },
        None => AutoResolution {
            profile: auto.fallback.clone(),
            basis: AutoBasis::NoRuleMatched,
            region: Some(region),
        },
    }
}

/// `profile` with `auto` resolved for `source`: a non-`auto` profile, its
/// name, and the resolution when `profile` is `auto`.
pub(super) fn concrete_profile<'a>(
    config: &'a NetworkConfiguration,
    profile: &str,
    source: Option<NodeId>,
) -> Result<
    (
        EgressProfile<'a>,
        String,
        Option<meshway_configuration::selection::AutoResolution>,
    ),
    CompileError,
> {
    match resolve_profile(config, profile)? {
        EgressProfile::Auto(auto) => {
            let resolution = auto_resolution(config, auto, source);
            match resolve_profile(config, &resolution.profile)? {
                EgressProfile::Auto(_) => Err(CompileError::InvalidInput),
                resolved => Ok((resolved, resolution.profile.clone(), Some(resolution))),
            }
        }
        resolved => Ok((resolved, profile.to_owned(), None)),
    }
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
