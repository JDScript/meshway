//! A verified enrollment adds one node with its initial settings and fresh
//! addresses, or re-admits an existing node under its new identity.
use super::{Draft, Snapshot};
use crate::configuration::ChangeCause;
use crate::enrollment::NodeAdmission;
use crate::error::Error;
use meshway_configuration::{
    Input, NetworkConfigurationInput, NodeAddressesInput, NodeIdentity, NodeSettingsInput,
    UserBinding, allocate_addresses,
};
use rusqlite::Transaction;
use std::collections::BTreeSet;

pub(super) fn author(
    tx: &Transaction<'_>,
    current: &Snapshot,
    admission: &NodeAdmission,
) -> Result<Draft, Error> {
    let before = meshway_configuration::parse_jsonc(current.document.source_jsonc.expose())
        .map_err(|_| Error::decode("configuration_versions.source_jsonc"))?;
    let mut input = NetworkConfigurationInput::from(&current.document.configuration);
    let existing = input.nodes.contains_key(&admission.node);
    if let Some(node) = input.nodes.get_mut(&admission.node) {
        node.identity = admission.identity.clone();
    } else {
        let retired =
            crate::store::configuration::addresses::retired_addresses(tx, current.network_id)?;
        let hostname = unique_hostname(
            admission.initial.hostname.as_deref().unwrap_or("node"),
            input.nodes.values().map(|n| n.hostname.as_str()),
        )?;
        input.nodes.insert(
            admission.node,
            NodeSettingsInput {
                hostname,
                remark: String::new(),
                identity: admission.identity.clone(),
                enabled: true,
                addresses: NodeAddressesInput::default(),
                listeners: admission.initial.listeners.clone(),
                dialers: Input::Omitted,
                exit_enabled: false,
            },
        );
        allocate_addresses(
            &mut input,
            &current.document.configuration,
            &retired,
            &BTreeSet::new(),
        )?;
    }
    let mut after = before.clone();
    after
        .nodes
        .insert(admission.node, input.nodes[&admission.node].clone());
    let source = meshway_configuration::rewrite_source(
        current.document.source_jsonc.expose(),
        &before,
        &after,
    )
    .map_err(|_| Error::encode("configuration source"))?;
    let mut draft = Draft::new(input, source, current);
    draft
        .approved_identities
        .insert(admission.node, admission.identity.clone());
    match (&admission.identity, admission.membership) {
        (NodeIdentity::User { .. }, Some(membership)) => {
            draft.expected_node_user_bindings.insert(
                admission.node,
                UserBinding {
                    user_id: admission.actor,
                    membership_id: membership,
                },
            );
        }
        (NodeIdentity::Tag { .. }, None) => {
            draft.expected_node_user_bindings.remove(&admission.node);
        }
        _ => return Err(Error::PermissionDenied),
    }
    draft.retain_rule_sets(tx, current)?;
    draft.cause = if existing {
        ChangeCause::IdentityTransition
    } else {
        ChangeCause::Enrollment
    };
    Ok(draft)
}

/// The suggested hostname, or the first free numbered variant of it.
fn unique_hostname<'a>(
    base: &str,
    existing: impl Iterator<Item = &'a str>,
) -> Result<String, Error> {
    let existing: BTreeSet<_> = existing.collect();
    if !existing.contains(base) {
        return Ok(base.to_owned());
    }
    for suffix in 2..=10_001 {
        let suffix = format!("-{suffix}");
        let prefix = &base[..base.len().min(63 - suffix.len())];
        let candidate = format!("{}{suffix}", prefix.trim_end_matches('-'));
        if !existing.contains(candidate.as_str()) {
            return Ok(candidate);
        }
    }
    Err(Error::Busy)
}
