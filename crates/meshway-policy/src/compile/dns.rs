//! Single-exit selection precedes resolution. Answers never decide which
//! resolver was authorized to obtain them.
use super::*;

pub(super) fn external_access(
    configuration: &NetworkConfiguration,
    subjects: &SubjectContext,
    source: NodeId,
) -> bool {
    configuration.access.iter().any(|rule| {
        !rule.ip.is_empty()
            && matching::selectors(configuration, subjects, &rule.src, source)
            && rule.dst.iter().any(|selector| {
                selector == "autogroup:internet"
                    || selector.starts_with("domain:")
                    || selector.parse::<ipnet::IpNet>().is_ok()
            })
    })
}

/// Require one identical exit and ordered via list across all effective
/// rules/fallbacks `source` executes as exits; one naming the source itself
/// is its direct egress. Auto and domain-dependent capture stay unavailable.
pub(super) fn single_exit(
    cfg: &NetworkConfiguration,
    profile: &str,
    source: NodeId,
) -> Result<Option<(NodeId, Vec<NodeId>)>, CompileError> {
    let mut actions: Vec<RouteAction> = cfg
        .routing
        .egress
        .rules
        .iter()
        .map(|r| r.action.clone().into())
        .collect();
    match matching::resolve_profile(cfg, profile)? {
        matching::EgressProfile::Mesh => {}
        matching::EgressProfile::Global(node) => actions.push(RouteAction::Exit {
            nodes: vec![format!("node:{node}")],
            via: vec![],
        }),
        matching::EgressProfile::Custom(profile) => {
            actions.extend(profile.rules.iter().map(|r| r.action.clone().into()));
            actions.push(profile.fallback.clone());
        }
        matching::EgressProfile::Auto(_) => return Ok(None),
    }
    let mut selected = None;
    for action in actions {
        let RouteAction::Exit { nodes, via } = matching::executed(action, source)? else {
            continue;
        };
        if nodes.len() != 1 {
            return Ok(None);
        }
        let candidate = (
            matching::node_selector(&nodes[0])?,
            via.iter()
                .map(|s| matching::node_selector(s))
                .collect::<Result<Vec<_>, _>>()?,
        );
        if selected.as_ref().is_some_and(|old| old != &candidate) {
            return Ok(None);
        }
        selected = Some(candidate);
    }
    Ok(selected)
}
