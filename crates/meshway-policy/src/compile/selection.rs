//! A Node's routing profile selection over a configuration: the one
//! computation the Server (management views and the selection commit) and
//! the Node (its installed netmap) share. The applied profile follows the
//! same rule as `effective_profile`, which every egress plan and decision
//! uses (`matching::profile_allowed`).
use super::matching::{self, EgressProfile};
use super::*;
use meshway_configuration::selection::{
    AutoBasis, AutoResolution, ProfileKind, ProfileSummary, RoutingSelectionState,
    SelectionUnavailable,
};
use meshway_model::Version;

/// `selected` and `version` are the Node's persistent selection under its
/// current identity (none for a stale identity).
pub fn routing_selection(
    configuration: &NetworkConfiguration,
    subjects: &SubjectContext,
    selected: Option<&str>,
    version: Option<Version>,
    node: NodeId,
) -> Result<RoutingSelectionState, CompileError> {
    let assignment = matching::assignment(configuration, subjects, node)?;
    let requested_unavailable = selected.and_then(|profile| {
        if !assignment.allowed.iter().any(|allowed| allowed == profile) {
            Some(SelectionUnavailable::NotAllowed)
        } else if assignment.locked && profile != assignment.default {
            Some(SelectionUnavailable::Locked)
        } else {
            None
        }
    });
    let applied = match selected {
        Some(profile) if requested_unavailable.is_none() => profile.to_owned(),
        _ => assignment.default.clone(),
    };
    let mut names: Vec<&String> = Vec::new();
    for name in &assignment.allowed {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let profiles = names
        .into_iter()
        .map(|name| summary(configuration, name))
        .collect::<Result<_, _>>()?;
    let auto = if applied == "auto" {
        match matching::resolve_profile(configuration, "auto")? {
            EgressProfile::Auto(auto) => Some(AutoResolution {
                profile: auto.fallback.clone(),
                basis: AutoBasis::NoTrustedRegion,
            }),
            _ => return Err(CompileError::InvalidInput),
        }
    } else {
        None
    };
    Ok(RoutingSelectionState {
        requested_profile: selected.map(str::to_owned),
        version,
        applied_profile: applied,
        requested_unavailable,
        default_profile: assignment.default,
        locked: assignment.locked,
        profiles,
        auto,
    })
}

fn summary(
    configuration: &NetworkConfiguration,
    name: &str,
) -> Result<ProfileSummary, CompileError> {
    let mut exits = Vec::new();
    let mut add = |selectors: &[String]| -> Result<(), CompileError> {
        for selector in selectors {
            let node = matching::node_selector(selector)?;
            if !exits.contains(&node) {
                exits.push(node);
            }
        }
        Ok(())
    };
    let (kind, rules, fallback) = match matching::resolve_profile(configuration, name)? {
        EgressProfile::Mesh => (ProfileKind::Mesh, 0, Some(RouteAction::Unmanaged)),
        EgressProfile::Global(node) => {
            let fallback = RouteAction::Exit {
                nodes: vec![format!("node:{node}")],
                via: vec![],
            };
            add(&[format!("node:{node}")])?;
            (ProfileKind::Global, 0, Some(fallback))
        }
        EgressProfile::Auto(_) => (ProfileKind::Auto, 0, None),
        EgressProfile::Custom(profile) => {
            for rule in &profile.rules {
                if let RuleAction::Exit { nodes, .. } = &rule.action {
                    add(nodes)?;
                }
            }
            if let RouteAction::Exit { nodes, .. } = &profile.fallback {
                add(nodes)?;
            }
            (
                ProfileKind::Custom,
                u32::try_from(profile.rules.len()).map_err(|_| CompileError::InvalidInput)?,
                Some(profile.fallback.clone()),
            )
        }
    };
    Ok(ProfileSummary {
        name: name.to_owned(),
        kind,
        rules,
        exits,
        fallback,
    })
}
