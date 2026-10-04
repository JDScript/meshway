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
use std::{collections::BTreeSet, net::IpAddr};

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
        let retired = retired_addresses(tx, current)?;
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

fn retired_addresses(tx: &Transaction<'_>, current: &Snapshot) -> Result<BTreeSet<IpAddr>, Error> {
    let mut statement = tx.prepare(
        "SELECT address FROM address_retirements WHERE network_id=?1 AND (release_after IS NULL OR release_after>?2)",
    )?;
    statement
        .query_map(
            rusqlite::params![
                current.network_id.as_uuid().as_bytes().as_slice(),
                crate::store::execution::now()?
            ],
            |r| r.get::<_, String>(0),
        )?
        .map(|r| {
            r?.parse::<IpAddr>()
                .map_err(|_| Error::decode("address_retirements.address"))
        })
        .collect()
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
