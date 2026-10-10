//! A manager's edit: a complete JSONC document, typed operations on the
//! current source, or a restore of an earlier version under current members.
use super::{Committer, Draft, Snapshot};
use crate::configuration::{ChangeCause, context, resources};
use crate::error::Error;
use meshway_configuration::{
    ConfigurationOperation, NetworkConfigurationInput, NodeIdentity, edit_source, parse_jsonc,
};
use meshway_protocol::configuration::ConfigurationRequest;
use rusqlite::Transaction;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn author(
    committer: &Committer,
    tx: &Transaction<'_>,
    current: &Snapshot,
    request: &ConfigurationRequest,
    resources: Option<&resources::Preparation>,
    commit: bool,
) -> Result<Draft, Error> {
    let mut draft = match request {
        ConfigurationRequest::Document { source_jsonc, .. } => Draft::new(
            parse_jsonc(source_jsonc.expose())?,
            source_jsonc.expose().to_owned(),
            current,
        ),
        ConfigurationRequest::Operations { ops, .. } => {
            let (source, input) = edit_source(current.document.source_jsonc.expose(), ops)?;
            let mut draft = Draft::new(input, source, current);
            draft.recreated_listeners = recreated_listeners(ops);
            draft
        }
        ConfigurationRequest::Restore { restore_from, .. } => {
            restore(committer, tx, current, *restore_from)?
        }
    };
    draft.cause = if matches!(request, ConfigurationRequest::Restore { .. }) {
        ChangeCause::Restore
    } else {
        ChangeCause::Edit
    };
    context::check_inputs(tx, current, &draft.input)?;
    if let Some(resources) = resources {
        resources.revalidate(tx, current, &draft.input)?;
    }
    // A manager may convert an existing node to a non-empty Tag identity in
    // place; other identity changes come from enrollment only.
    for (id, node) in &draft.input.nodes {
        let changed = current
            .document
            .configuration
            .nodes
            .get(id)
            .is_some_and(|previous| previous.identity != node.identity);
        if changed && matches!(&node.identity, NodeIdentity::Tag { tags } if !tags.is_empty()) {
            draft.approved_identities.insert(*id, node.identity.clone());
        }
    }
    if let ConfigurationRequest::Restore {
        restore_from,
        rule_set_bindings,
        ..
    } = request
    {
        let snapshot =
            committer
                .cache
                .load(tx, current.network_id, *restore_from, &committer.sealer)?;
        let bindings = match rule_set_bindings {
            Some(meshway_protocol::configuration::RuleSetBindingChoice::Refreshed) => {
                crate::resources::bindings::at_revision(
                    tx,
                    current.network_id,
                    *restore_from,
                    &snapshot.bindings.rule_sets,
                )?
            }
            Some(meshway_protocol::configuration::RuleSetBindingChoice::Initial) | None => {
                snapshot.bindings.rule_sets.clone()
            }
        };
        draft.rule_set_contents =
            crate::resources::bindings::contents(tx, current.network_id, &bindings)?;
        draft.rule_sets = bindings;
    } else {
        draft.retain_rule_sets(tx, current)?;
    }
    if let Some(resources) = resources {
        draft.rule_set_contents.extend(resources.contents.clone());
        crate::resources::bindings::check_budget(&draft.rule_set_contents)?;
        if commit {
            resources.bind(tx, current, &mut draft.rule_sets)?;
        } else {
            draft.new_content = resources.preview_bindings(tx, current, &mut draft.rule_sets)?;
        }
    }
    Ok(draft)
}

/// Resources deleted and then set again in one batch are new resources:
/// their materials are not inherited.
fn recreated_listeners(
    ops: &[ConfigurationOperation],
) -> BTreeSet<(meshway_model::NodeId, String)> {
    let mut removed = BTreeSet::new();
    let mut recreated = BTreeSet::new();
    for op in ops {
        if let ConfigurationOperation::SetListener {
            node_id,
            key,
            listener,
        } = op
        {
            let resource = (*node_id, key.clone());
            if listener.is_none() {
                removed.insert(resource);
            } else if removed.contains(&resource) {
                recreated.insert(resource);
            }
        }
    }
    recreated
}

/// An earlier version's content under the current members: every stable
/// binding of that version must resolve to an active member now, whose
/// current e-mail replaces the readable name recorded then.
fn restore(
    committer: &Committer,
    tx: &Transaction<'_>,
    current: &Snapshot,
    restore_from: meshway_model::Revision,
) -> Result<Draft, Error> {
    let snapshot = committer
        .cache
        .load(tx, current.network_id, restore_from, &committer.sealer)?;
    let members = context::members(tx, current.network_id)?;
    let name_of = |binding| {
        members
            .iter()
            .find(|(_, candidate)| *candidate == binding)
            .map(|(email, _)| email.clone())
            .ok_or(Error::DependencyUnavailable)
    };
    let mut names = BTreeMap::new();
    let mut expected_users = BTreeMap::new();
    for (name, binding) in &snapshot.bindings.users {
        let current_name = name_of(binding)?;
        names.insert(name.clone(), current_name.clone());
        expected_users.insert(current_name, binding.clone());
    }
    let mut node_names = BTreeMap::new();
    for (id, binding) in &snapshot.bindings.node_users {
        node_names.insert(*id, name_of(binding)?);
    }
    let mut input = NetworkConfigurationInput::from(&snapshot.document.configuration);
    meshway_configuration::rebind_policy_user_references(&mut input, &names);
    meshway_configuration::rebind_node_user_references(&mut input, &node_names);
    let before = parse_jsonc(snapshot.document.source_jsonc.expose())?;
    let mut after = before.clone();
    meshway_configuration::rebind_policy_user_references(&mut after, &names);
    meshway_configuration::rebind_node_user_references(&mut after, &node_names);
    let source = meshway_configuration::rewrite_source(
        snapshot.document.source_jsonc.expose(),
        &before,
        &after,
    )?;
    let mut draft = Draft::new(input, source, current);
    draft.expected_user_bindings = expected_users;
    draft.expected_node_user_bindings = snapshot.bindings.node_users.clone();
    draft.rollback_from = Some(restore_from);
    Ok(draft)
}
