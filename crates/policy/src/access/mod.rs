//! Authorization intersection with ordered routing intent, compiled once per
//! coherent configuration. No graph search, adjacency inference or packet I/O.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use meshway_protocol::{
    AccessAction, AccessMatchedDirection, CONFIGURATION_SCHEMA_VERSION, FlowAuthorizationAction,
    FlowAuthorizationDecision, MAX_CONFIGURATION_BYTES, MAX_INVENTORY_NODES, MAX_MESH_ACCESS_BYTES,
    MAX_MESH_ACCESS_GRANTS, MAX_MESH_ACCESS_PATH_NODES, MAX_MESH_ACCESS_RULES,
    MAX_MESH_ACCESS_SELECTORS, MeshAccessPolicy, MeshNode, MeshPathAuthorization,
    NodeConfiguration, PolicyDiagnostic,
};

use crate::{diagnostic, prepare_routing};

fn valid_rule_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn valid_node_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn valid_tag_selector(value: &str) -> bool {
    let Some(name) = value.strip_prefix("tag:") else {
        return false;
    };
    !name.is_empty()
        && name.len() <= 48
        && name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (index != 0 && (byte == b'_' || byte == b'-'))
        })
}

fn validate_selectors(
    values: &[String],
    field: &str,
    rule_id: Option<&str>,
    inventory: &BTreeSet<&str>,
    tags: &BTreeSet<&str>,
    diagnostics: &mut Vec<PolicyDiagnostic>,
) {
    if values.is_empty() || values.len() > MAX_MESH_ACCESS_SELECTORS {
        diagnostic(
            diagnostics,
            rule_id,
            field,
            "Selector lists must contain 1..32 entries",
        );
    }
    if values.iter().any(|value| value == "*") && values != ["*"] {
        diagnostic(
            diagnostics,
            rule_id,
            field,
            "Wildcard '*' must be the only selector in its list",
        );
    }
    let mut unique = BTreeSet::new();
    for value in values {
        if !unique.insert(value.as_str()) {
            diagnostic(
                diagnostics,
                rule_id,
                field,
                "Selector entries must be unique",
            );
            continue;
        }
        if value == "*" {
            continue;
        }
        if let Some(node) = value.strip_prefix("node:") {
            if !valid_node_id(node) || !inventory.contains(node) {
                diagnostic(
                    diagnostics,
                    rule_id,
                    field,
                    "Node selectors must reference an active 64-character lowercase hexadecimal node ID",
                );
            }
        } else if !valid_tag_selector(value) || !tags.contains(value.as_str()) {
            diagnostic(
                diagnostics,
                rule_id,
                field,
                "Tag selectors must reference an existing lowercase tag",
            );
        }
    }
}

/// Validate saved endpoint ACL source against active nodes and the admin-owned
/// tag catalog. Missing endpoint public keys are deliberately not source errors.
pub fn validate_access_policy(
    policy: &MeshAccessPolicy,
    nodes: &[MeshNode],
    tags: &BTreeSet<String>,
) -> Vec<PolicyDiagnostic> {
    let mut diagnostics = Vec::new();
    if serde_json::to_vec(policy).map_or(true, |bytes| bytes.len() > MAX_MESH_ACCESS_BYTES) {
        diagnostic(
            &mut diagnostics,
            None,
            "policy",
            "Access policy exceeds 128 KiB",
        );
        return diagnostics;
    }
    if policy.rules.len() > MAX_MESH_ACCESS_RULES {
        diagnostic(
            &mut diagnostics,
            None,
            "rules",
            "Access policy exceeds 128 rules",
        );
    }
    let inventory: BTreeSet<_> = nodes.iter().map(|node| node.id.as_str()).collect();
    let tag_refs: BTreeSet<_> = tags.iter().map(String::as_str).collect();
    let mut ids = BTreeSet::new();
    for rule in &policy.rules {
        let id = valid_rule_id(&rule.id).then_some(rule.id.as_str());
        if id.is_none() {
            diagnostic(
                &mut diagnostics,
                None,
                "id",
                "Rule ID must be 1..64 ASCII letters, digits, '.', '_' or '-'",
            );
        }
        if !ids.insert(rule.id.as_str()) {
            diagnostic(&mut diagnostics, id, "id", "Rule IDs must be unique");
        }
        validate_selectors(
            &rule.source,
            "source",
            id,
            &inventory,
            &tag_refs,
            &mut diagnostics,
        );
        validate_selectors(
            &rule.destination,
            "destination",
            id,
            &inventory,
            &tag_refs,
            &mut diagnostics,
        );
    }
    diagnostics
}

fn selector_matches(values: &[String], node: &str, tags: &BTreeSet<String>) -> bool {
    values.iter().any(|value| {
        value == "*"
            || value
                .strip_prefix("node:")
                .is_some_and(|candidate| candidate == node)
            || (value.starts_with("tag:") && tags.contains(value))
    })
}

/// Evaluate ordered first-match endpoint authorization. Bidirectionality tests
/// the reversed predicate at the same rule position; it is not connection state.
pub fn evaluate_access(
    policy: &MeshAccessPolicy,
    source: &str,
    destination: &str,
    assignments: &BTreeMap<String, BTreeSet<String>>,
) -> FlowAuthorizationDecision {
    let empty = BTreeSet::new();
    let source_tags = assignments.get(source).unwrap_or(&empty);
    let destination_tags = assignments.get(destination).unwrap_or(&empty);
    for rule in &policy.rules {
        let forward = selector_matches(&rule.source, source, source_tags)
            && selector_matches(&rule.destination, destination, destination_tags);
        let reverse = !forward
            && rule.bidirectional
            && selector_matches(&rule.source, destination, destination_tags)
            && selector_matches(&rule.destination, source, source_tags);
        if !forward && !reverse {
            continue;
        }
        return FlowAuthorizationDecision {
            action: match rule.action {
                AccessAction::Allow => FlowAuthorizationAction::Allow,
                AccessAction::Reject => FlowAuthorizationAction::Reject,
            },
            rule_id: Some(rule.id.clone()),
            matched_direction: Some(if forward {
                AccessMatchedDirection::Forward
            } else {
                AccessMatchedDirection::Reverse
            }),
            reason: match rule.action {
                AccessAction::Allow => "Matched allow rule",
                AccessAction::Reject => "Matched reject rule",
            }
            .into(),
        };
    }
    FlowAuthorizationDecision {
        action: match policy.default_action {
            AccessAction::Allow => FlowAuthorizationAction::Allow,
            AccessAction::Reject => FlowAuthorizationAction::Reject,
        },
        rule_id: None,
        matched_direction: None,
        reason: match policy.default_action {
            AccessAction::Allow => "No rule matched; default allow",
            AccessAction::Reject => "No rule matched; default reject",
        }
        .into(),
    }
}

/// Validate compiled full directional paths against the active inventory.
pub fn validate_mesh_path_authorization(
    policy: &MeshPathAuthorization,
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
        let valid_id = valid_rule_id(&grant.id);
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
    access: &MeshPathAuthorization,
) -> Result<Vec<Vec<String>>, Vec<PolicyDiagnostic>> {
    let nodes: Vec<_> = std::iter::once(configuration.node.clone())
        .chain(configuration.peers.iter().cloned())
        .collect();
    let mut diagnostics = validate_mesh_path_authorization(access, &nodes);
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
    let prepared = prepare_routing(
        &configuration.routing_policy,
        &configuration.network,
        &nodes,
    );
    for entry in &prepared.report().diagnostics {
        diagnostic(
            &mut diagnostics,
            entry.rule_id.as_deref(),
            &entry.field,
            &entry.message,
        );
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    let mut paths = Vec::new();
    for grant in &access.grants {
        if prepared
            .compile_mesh_endpoint(&grant.path[0], grant.path.last().expect("validated path"))
            == Some(grant.path.clone())
        {
            paths.push(grant.path.clone());
        }
    }
    Ok(paths)
}

#[cfg(test)]
mod tests;
