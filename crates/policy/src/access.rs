//! Authorization intersection with ordered routing intent, compiled once per
//! coherent configuration. No graph search, adjacency inference or packet I/O.

use std::{collections::BTreeSet, io};

use meshway_protocol::{
    CONFIGURATION_SCHEMA_VERSION, DestinationSelectorKind, FlowQuery, MAX_CONFIGURATION_BYTES,
    MAX_INVENTORY_NODES, MAX_MESH_ACCESS_BYTES, MAX_MESH_ACCESS_GRANTS, MAX_MESH_ACCESS_PATH_NODES,
    MeshAccessPolicy, MeshNode, NodeConfiguration, PolicyDiagnostic, RouteDecisionAction,
};

use crate::{diagnostic, matches_rule, prepare, preview, resolve_flow};

/// Validate full directional paths against the entire active inventory. The
/// controller additionally requires immutable public-key bindings for each node.
pub fn validate_mesh_access(
    policy: &MeshAccessPolicy,
    nodes: &[MeshNode],
) -> Vec<PolicyDiagnostic> {
    let mut diagnostics = Vec::new();
    struct BoundedWriter(usize);
    impl io::Write for BoundedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > MAX_MESH_ACCESS_BYTES - self.0 {
                return Err(io::Error::other("access size limit exceeded"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    if serde_json::to_writer(BoundedWriter(0), policy).is_err() {
        diagnostic(
            &mut diagnostics,
            None,
            "policy",
            "Access policy exceeds 128 KiB",
        );
        return diagnostics;
    }
    if policy.grants.len() > MAX_MESH_ACCESS_GRANTS {
        diagnostic(
            &mut diagnostics,
            None,
            "grants",
            "Access policy exceeds 128 grants",
        );
    }
    let mut inventory = BTreeSet::new();
    if nodes.len() > MAX_INVENTORY_NODES
        || nodes
            .iter()
            .any(|node| node.id.trim().is_empty() || !inventory.insert(node.id.as_str()))
    {
        diagnostic(
            &mut diagnostics,
            None,
            "nodes",
            "Active inventory must be bounded with nonempty unique node IDs",
        );
    }
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for grant in &policy.grants {
        let valid_id = !grant.id.is_empty()
            && grant.id.len() <= 64
            && grant
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte));
        // A count cap alone cannot bound diagnostics that repeat an unchecked ID.
        let id = valid_id.then_some(grant.id.as_str());
        if !valid_id {
            diagnostic(
                &mut diagnostics,
                id,
                "id",
                "Grant ID must be 1..64 ASCII letters, digits, '.', '_' or '-'",
            );
        }
        if !ids.insert(&grant.id) {
            diagnostic(&mut diagnostics, id, "id", "Grant IDs must be unique");
        }
        if !(2..=MAX_MESH_ACCESS_PATH_NODES).contains(&grant.path.len()) {
            diagnostic(
                &mut diagnostics,
                id,
                "path",
                "Full paths must contain 2..8 nodes including endpoints",
            );
        }
        if !paths.insert(&grant.path) {
            diagnostic(
                &mut diagnostics,
                id,
                "path",
                "Duplicate full paths are not allowed",
            );
        }
        let mut visited = BTreeSet::new();
        for node in &grant.path {
            if !inventory.contains(node.as_str()) || !visited.insert(node) {
                diagnostic(
                    &mut diagnostics,
                    id,
                    "path",
                    "Every path node must be active and appear exactly once",
                );
            }
        }
    }
    diagnostics
}

/// Intersect exact grants with first-match mesh IPv4 routing decisions, including
/// default direct. Every compiled path requires its own exact directional grant;
/// routing alone, rejects and ungranted reverse paths or subpaths grant nothing.
/// A potentially matching domain-only condition before the known first match
/// withdraws that flow: raw IPv4 packets carry no trusted domain evidence. A
/// known matching OR selector, later domain rules and Internet-only rules do
/// not introduce uncertainty. This intentionally denies even an uncertain rule
/// that happens to resolve to the same path, rather than infer DNS context.
pub fn compile_mesh_paths(
    configuration: &NodeConfiguration,
    access: &MeshAccessPolicy,
) -> Result<Vec<Vec<String>>, Vec<PolicyDiagnostic>> {
    let nodes: Vec<_> = std::iter::once(configuration.node.clone())
        .chain(configuration.peers.iter().cloned())
        .collect();
    let mut diagnostics = validate_mesh_access(access, &nodes);
    if !configuration.policy_valid {
        diagnostic(
            &mut diagnostics,
            None,
            "policy_valid",
            "Routing source is invalid",
        );
    }
    if configuration.schema_version != CONFIGURATION_SCHEMA_VERSION || configuration.revision == 0 {
        diagnostic(
            &mut diagnostics,
            None,
            "configuration",
            "Unsupported schema or invalid revision",
        );
    }
    if serde_json::to_vec(configuration).map_or(true, |bytes| bytes.len() > MAX_CONFIGURATION_BYTES)
    {
        diagnostic(
            &mut diagnostics,
            None,
            "configuration",
            "Configuration exceeds publication budget",
        );
    }
    let (report, inventory) = prepare(
        &configuration.routing_policy,
        &configuration.network,
        &nodes,
    );
    for entry in report.diagnostics {
        diagnostic(
            &mut diagnostics,
            entry.rule_id.as_deref(),
            entry.field,
            entry.message,
        );
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    let mut paths = Vec::new();
    for grant in &access.grants {
        // Structural validation above guarantees both endpoints exist.
        let target = inventory
            .nodes
            .get(grant.path.last().expect("validated path").as_str())
            .expect("validated inventory reference");
        let flow = FlowQuery {
            source_node_id: grant.path[0].clone(),
            destination_node_id: Some(target.id.to_owned()),
            destination_ip: Some(target.ipv4.to_string()),
            domain: None,
            destination_country: None,
        };
        let context = resolve_flow(&flow, &inventory).expect("validated mesh endpoints");
        let mut uncertain = false;
        for rule in &report.normalized_policy.rules {
            if matches_rule(rule, &context) {
                break;
            }
            if rule
                .destination_any
                .iter()
                .any(|selector| selector.kind == DestinationSelectorKind::DomainSuffix)
            {
                let mut potential = rule.clone();
                potential.destination_any.clear();
                if matches_rule(&potential, &context) {
                    uncertain = true;
                    break;
                }
            }
        }
        if uncertain {
            continue;
        }
        let result = preview(
            &report.normalized_policy,
            &configuration.network,
            &nodes,
            &flow,
        );
        if result.valid
            && result.decision.action == RouteDecisionAction::Route
            && result.decision.exit_node_id.is_none()
            && result.decision.path == grant.path
        {
            paths.push(grant.path.clone());
        }
    }
    Ok(paths)
}

#[cfg(test)]
mod tests;
