//! Declaration-only checks for routing-required constrained edges. Dynamic
//! inventory/capability failures remain per-demand unavailable decisions.
use super::*;
use std::collections::BTreeSet;

/// One validation names at most this many routing rule / edge pairs; the
/// first ones in Node ID order are kept.
pub const MAX_EDGE_TRANSPORT_ISSUES: usize = 64;

pub(crate) fn validate_edge_transports(
    configuration: &NetworkConfiguration,
    context: &ValidationContext<'_>,
    report: &mut ValidationReport,
) {
    if !configuration
        .topology
        .iter()
        .any(|r| matches!(r.transports, Input::Value(_)))
    {
        return;
    }
    let subjects = SubjectContext {
        users: report.bindings.clone(),
        node_users: report.node_bindings.clone(),
        members: context.members.clone(),
    };
    let inventory = BTreeMap::new();
    let previous_subjects = SubjectContext {
        users: context.previous_user_bindings.clone(),
        node_users: context.previous_node_user_bindings.clone(),
        members: context.members.clone(),
    };
    let mut seen = BTreeSet::new();
    'pairs: for (&source, settings) in &configuration.nodes {
        if !settings.enabled {
            continue;
        }
        for (&target, settings) in &configuration.nodes {
            if source == target || !settings.enabled {
                continue;
            }
            let Some(index) = configuration.routing.mesh.iter().position(|rule| {
                matching::selectors(configuration, &subjects, &rule.src, source)
                    && matching::selectors(configuration, &subjects, &rule.dst, target)
            }) else {
                continue;
            };
            let Ok(graph) = paths::Graph::declarations(Topology {
                configuration,
                subjects: &subjects,
                inventory: &inventory,
            }) else {
                continue;
            };
            if !matches!(
                graph.paths(source, target, None, 1),
                Err(routing::PathError::Compile(
                    CompileError::EdgeTransportUnavailable { .. }
                ))
            ) {
                continue;
            }
            for (&(a, b), &edge) in graph.transport_failures.borrow().iter() {
                if seen.len() == MAX_EDGE_TRANSPORT_ISSUES {
                    break 'pairs;
                }
                if seen.insert((edge, a, b, index)) {
                    let rule = &configuration.topology[edge];
                    let retained = context.previous.is_some_and(|old| {
                        crate::topology_decision(old, &previous_subjects, a, b)
                            .flatten()
                            .is_some_and(|i| &old.topology[i] == rule)
                            && old.routing.mesh.iter().find(|r| {
                                matching::selectors(old, &previous_subjects, &r.src, source)
                                    && matching::selectors(old, &previous_subjects, &r.dst, target)
                            }) == Some(&configuration.routing.mesh[index])
                    });
                    report.dependency(
                        ConfigurationIssue::new(
                            format!("/topology/{edge}/transports"),
                            IssueCode::InvalidTransport,
                        )
                        .with("reason", "edge_transport_unavailable")
                        .with("a", a)
                        .with("b", b)
                        .with("routingRule", format!("/routing/mesh/{index}")),
                        retained,
                    );
                }
            }
        }
    }
}
