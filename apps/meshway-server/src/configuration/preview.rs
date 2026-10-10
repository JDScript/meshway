//! Non-executable declaration planning. No generated credential, prepared
//! listener attestation or persistent configuration is manufactured here.
use super::{Snapshot, binding_changes, commit::Draft, listener_bindings, resources};
use crate::configuration::ConfigurationBindings;
use crate::error::Error;
use meshway_configuration::{
    ChangeKind, ConfigurationChange, PreparationKind, PreparationRequirement, ValidationContext,
    planned_configuration_changes,
};
use meshway_model::{NodeId, RequestId};
use meshway_policy::plan_configuration;
use meshway_protocol::configuration::{ConfigurationPreviewResponse, ListenerChange};
use meshway_transport::spec::MaterialRequirement;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn build(
    current: &Snapshot,
    draft: Draft,
    context: ValidationContext<'_>,
    resources: Option<&resources::Preparation>,
    request_id: RequestId,
) -> Result<ConfigurationPreviewResponse, Error> {
    let plan = plan_configuration(draft.input, &context)?;
    let mut changes = planned_configuration_changes(&current.document.configuration, &plan.input)?;
    let generated = |r: &PreparationRequirement| {
        matches!(
            r.kind,
            PreparationKind::Material(
                MaterialRequirement::Uuid
                    | MaterialRequirement::Password
                    | MaterialRequirement::Certificate
                    | MaterialRequirement::PrivateKey
            )
        )
    };
    let (will_generate, requirements): (Vec<_>, Vec<_>) =
        plan.requirements.into_iter().partition(generated);
    for requirement in &will_generate {
        let field = match requirement.kind {
            PreparationKind::Material(MaterialRequirement::Uuid) => "uuid",
            PreparationKind::Material(MaterialRequirement::Password) => "password",
            PreparationKind::Material(MaterialRequirement::Certificate) => "tls/certificate",
            PreparationKind::Material(MaterialRequirement::PrivateKey) => "tls/privateKey",
            _ => unreachable!("generation-only requirements"),
        };
        let path = format!("{}/{field}", requirement.path);
        if let Some(change) = changes.iter_mut().find(|change| change.path == path) {
            // Delete/recreate replaces an old value; omission is a generation
            // plan, not an instruction to remove the resulting credential.
            change.kind = ChangeKind::Changed;
        }
    }
    let bindings = ConfigurationBindings {
        users: plan.user_bindings,
        node_users: plan.node_user_bindings,
        listeners: listener_bindings(
            current,
            plan.input
                .nodes
                .iter()
                .map(|(id, node)| (*id, node.listeners.keys().cloned().collect())),
            &context.recreated_listeners,
        )?,
        rule_sets: draft.rule_sets,
    };
    changes.extend(binding_changes(&current.bindings, &bindings));
    if draft.new_content && !changes.iter().any(|c| c.path == "/bindings/ruleSets") {
        changes.push(ConfigurationChange {
            path: "/bindings/ruleSets".into(),
            kind: ChangeKind::Changed,
        });
    }
    let old = &current.document.configuration.nodes;
    let deleted_node_ids: Vec<NodeId> = old
        .keys()
        .filter(|id| !plan.view.nodes.contains_key(id))
        .copied()
        .collect();
    let address_changed_node_ids = old
        .iter()
        .filter(|(id, node)| {
            plan.view
                .nodes
                .get(id)
                .is_some_and(|new| new.addresses != node.addresses)
        })
        .map(|(id, _)| *id)
        .collect();
    let identity_changed_node_ids = plan
        .view
        .nodes
        .iter()
        .filter(|(id, node)| {
            old.get(id)
                .is_some_and(|previous| previous.identity != node.identity)
                || current.bindings.node_users.get(id) != bindings.node_users.get(id)
        })
        .map(|(id, _)| *id)
        .collect();
    let changed_listeners = listener_changes(&changes);
    let affected_node_ids = affected_nodes(&changes, &deleted_node_ids, &plan.view);
    Ok(ConfigurationPreviewResponse {
        request_id,
        base_revision: current.revision,
        preview: plan.view,
        requirements,
        will_generate,
        prepared_rule_sets: resources
            .map(|r| r.preview())
            .transpose()?
            .unwrap_or_default(),
        unavailable_dependencies: plan.unavailable_dependencies,
        warnings: plan.warnings,
        change_count: u32::try_from(changes.len())
            .map_err(|_| Error::Internal("change count overflow"))?,
        changes,
        source_changed: current.document.source_jsonc.expose() != draft.source,
        deleted_node_ids,
        address_changed_node_ids,
        identity_changed_node_ids,
        changed_listeners,
        affected_node_ids,
    })
}

/// One entry per listener with any change beneath `/nodes/{id}/listeners/{key}`;
/// the whole listener added or removed counts as such, anything else as changed.
fn listener_changes(changes: &[ConfigurationChange]) -> Vec<ListenerChange> {
    let mut seen: BTreeMap<(NodeId, String), ChangeKind> = BTreeMap::new();
    for change in changes {
        let Some(rest) = change.path.strip_prefix("/nodes/") else {
            continue;
        };
        let mut parts = rest.splitn(4, '/');
        let (Some(node), Some("listeners"), Some(key)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let Ok(node) = node.parse::<NodeId>() else {
            continue;
        };
        let key = key.replace("~1", "/").replace("~0", "~");
        let whole = parts.next().is_none();
        let kind = if whole {
            change.kind.clone()
        } else {
            ChangeKind::Changed
        };
        seen.entry((node, key))
            .and_modify(|existing| {
                if whole {
                    *existing = kind.clone();
                }
            })
            .or_insert(kind);
    }
    seen.into_iter()
        .map(|((node_id, key), kind)| ListenerChange { node_id, key, kind })
        .collect()
}

/// Nodes whose execution target may change. Only node-local display fields
/// leave peers untouched; every other change can alter what some peer dials,
/// routes or is permitted, so it affects every node.
fn affected_nodes(
    changes: &[ConfigurationChange],
    deleted: &[NodeId],
    view: &meshway_configuration::ConfigurationView,
) -> Vec<NodeId> {
    let mut local = BTreeSet::new();
    for change in changes {
        if change.path.starts_with("/bindings/") {
            // Binding pointers move with the values they belong to; the value
            // change is what is classified.
            if change.path != "/bindings/listeners" {
                return view.nodes.keys().copied().collect();
            }
            continue;
        }
        let Some(rest) = change.path.strip_prefix("/nodes/") else {
            return view.nodes.keys().copied().collect();
        };
        let mut parts = rest.splitn(3, '/');
        match (
            parts.next().and_then(|id| id.parse::<NodeId>().ok()),
            parts.next(),
        ) {
            (Some(node), Some("hostname" | "remark")) => {
                local.insert(node);
            }
            _ => return view.nodes.keys().copied().collect(),
        }
    }
    if !deleted.is_empty() {
        return view.nodes.keys().copied().collect();
    }
    local.into_iter().collect()
}
