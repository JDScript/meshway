//! A Node leaving deletes exactly that Node, the same permanent deletion a
//! manager's `deleteNode` performs.
use super::{Draft, Snapshot};
use crate::configuration::ChangeCause;
use crate::error::Error;
use crate::execution::NodePrincipal;
use crate::store;
use meshway_configuration::{NetworkConfigurationInput, parse_jsonc, rewrite_source};
use meshway_model::UserId;
use rusqlite::Transaction;

/// The version is recorded under the user accountable for the Node: its
/// bound user, or the user who admitted its Tag identity. The schema keeps
/// a user author for every version; the notice names the Node itself.
pub(super) fn authorize(
    tx: &Transaction<'_>,
    current: &Snapshot,
    principal: &NodePrincipal,
) -> Result<UserId, Error> {
    let node = principal.node_id();
    if principal.network_id() != current.network_id
        || !current.document.configuration.nodes.contains_key(&node)
    {
        return Err(Error::NodeIdentityRejected);
    }
    if let Some(binding) =
        store::execution::user_binding(&current.document.configuration, &current.bindings, node)
    {
        return Ok(binding.user_id);
    }
    store::enrollment::admitting_user(tx, current.network_id, node)?.ok_or(Error::Internal(
        "Node admission without an accountable user",
    ))
}

pub(super) fn author(
    tx: &Transaction<'_>,
    current: &Snapshot,
    principal: &NodePrincipal,
) -> Result<Draft, Error> {
    let node = principal.node_id();
    let mut input = NetworkConfigurationInput::from(&current.document.configuration);
    input.nodes.remove(&node);
    let before = parse_jsonc(current.document.source_jsonc.expose())?;
    let mut after = before.clone();
    after.nodes.remove(&node);
    let source = rewrite_source(current.document.source_jsonc.expose(), &before, &after)?;
    let mut draft = Draft::new(input, source, current);
    draft.retain_rule_sets(tx, current)?;
    draft.cause = ChangeCause::NodeDeleted;
    Ok(draft)
}
