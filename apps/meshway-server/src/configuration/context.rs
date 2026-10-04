//! Trusted inputs a configuration normalization reads from the transaction:
//! members, node inventory and the previous version's bindings.
use super::{Snapshot, commit::Draft};
use crate::error::Error;
use meshway_configuration::{NetworkConfigurationInput, UserBinding, ValidationContext};
use meshway_model::{NetworkId, NodeId};
use meshway_transport_spec::Listener;
use rusqlite::Transaction;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn inventory_ids(
    tx: &Transaction<'_>,
    network: NetworkId,
) -> Result<BTreeSet<NodeId>, Error> {
    let mut statement =
        tx.prepare("SELECT id FROM nodes WHERE network_id=?1 AND credentials_revoked_at IS NULL")?;
    statement
        .query_map([network.as_uuid().as_bytes().as_slice()], |r| {
            r.get::<_, Vec<u8>>(0)
        })?
        .map(|row| crate::store::execution::id(row?))
        .collect()
}

pub(crate) fn members(
    tx: &Transaction<'_>,
    network: NetworkId,
) -> Result<BTreeMap<String, UserBinding>, Error> {
    let mut statement = tx.prepare(
        "SELECT u.email,u.id,m.id FROM memberships m JOIN users u ON u.id=m.user_id WHERE m.network_id=?1 AND m.revoked_at IS NULL AND u.disabled_at IS NULL",
    )?;
    statement
        .query_map([network.as_uuid().as_bytes().as_slice()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })?
        .map(|row| {
            let (email, user, member) = row?;
            Ok((
                email,
                UserBinding {
                    user_id: crate::store::execution::id(user)?,
                    membership_id: crate::store::execution::id(member)?,
                },
            ))
        })
        .collect()
}

/// Every node an input names must exist in the inventory with live credentials.
pub(crate) fn check_inputs(
    tx: &Transaction<'_>,
    current: &Snapshot,
    input: &NetworkConfigurationInput,
) -> Result<(), Error> {
    let nodes = inventory_ids(tx, current.network_id)?;
    if input.nodes.keys().any(|id| !nodes.contains(id)) {
        return Err(Error::DependencyUnavailable);
    }
    Ok(())
}

/// The context a draft is normalized against. Verified listeners are the
/// declarations the material step attested in this transaction.
pub(crate) fn for_draft<'a>(
    tx: &Transaction<'_>,
    current: &'a Snapshot,
    draft: &Draft,
    verified_listeners: BTreeMap<(NodeId, String), Listener>,
) -> Result<ValidationContext<'a>, Error> {
    Ok(ValidationContext {
        members: members(tx, current.network_id)?,
        nodes: inventory_ids(tx, current.network_id)?,
        previous: Some(&current.document.configuration),
        expected_user_bindings: draft.expected_user_bindings.clone(),
        previous_user_bindings: current.bindings.users.clone(),
        expected_node_user_bindings: draft.expected_node_user_bindings.clone(),
        previous_node_user_bindings: current.bindings.node_users.clone(),
        approved_identities: draft.approved_identities.clone(),
        verified_listeners,
        recreated_listeners: draft.recreated_listeners.clone(),
        rule_sets: draft.rule_set_contents.clone(),
        previous_rule_sets: crate::resources::bindings::current_contents(tx, current)?,
    })
}

/// The context under which the current version itself normalizes: its own
/// bindings and declarations, with the given rule-set contents.
pub(crate) fn for_current<'a>(
    tx: &Transaction<'_>,
    current: &'a Snapshot,
    rule_sets: BTreeMap<String, meshway_model::resources::RuleSetData>,
) -> Result<ValidationContext<'a>, Error> {
    Ok(ValidationContext {
        members: members(tx, current.network_id)?,
        nodes: inventory_ids(tx, current.network_id)?,
        previous: Some(&current.document.configuration),
        expected_user_bindings: current.bindings.users.clone(),
        previous_user_bindings: current.bindings.users.clone(),
        expected_node_user_bindings: current.bindings.node_users.clone(),
        previous_node_user_bindings: current.bindings.node_users.clone(),
        approved_identities: BTreeMap::new(),
        verified_listeners: current
            .document
            .configuration
            .nodes
            .iter()
            .flat_map(|(node, settings)| {
                settings.listeners.iter().map(move |(key, listener)| {
                    ((*node, key.clone()), listener.declaration().clone())
                })
            })
            .collect(),
        recreated_listeners: BTreeSet::new(),
        rule_sets,
        previous_rule_sets: crate::resources::bindings::current_contents(tx, current)?,
    })
}
