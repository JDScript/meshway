//! Removing a member permanently deletes that membership's User devices.
use super::{Draft, Snapshot};
use crate::configuration::ChangeCause;
use crate::error::Error;
use crate::store::{self, authorization::current_session};
use meshway_configuration::{NetworkConfigurationInput, parse_jsonc, rewrite_source};
use meshway_model::{MembershipId, UserId, Version};
use rusqlite::Transaction;

/// The caller may remove this membership at this version; returns the actor.
pub(super) fn authorize(
    tx: &Transaction<'_>,
    current: &Snapshot,
    session: &[u8; 32],
    membership: MembershipId,
    version: Version,
    now: i64,
) -> Result<UserId, Error> {
    let actor = current_session(tx, session, now)?.user.id;
    let participants = store::configuration::memberships::removal_participants(
        tx,
        current.network_id,
        actor,
        membership,
    )?;
    if !participants.version_matches(version) {
        return Err(Error::VersionConflict);
    }
    let owners = store::configuration::memberships::owner_count(tx, current.network_id)?;
    let (caller, target_role) = participants.roles()?;
    let allowed =
        store::authorization::member_actions(caller, (membership, target_role), owners).remove;
    if !allowed {
        return Err(Error::PermissionDenied);
    }
    Ok(actor)
}

pub(super) fn author(
    tx: &Transaction<'_>,
    current: &Snapshot,
    membership: MembershipId,
) -> Result<Draft, Error> {
    let mut input = NetworkConfigurationInput::from(&current.document.configuration);
    input.nodes.retain(|id, _| {
        store::execution::user_binding(&current.document.configuration, &current.bindings, *id)
            .is_none_or(|binding| binding.membership_id != membership)
    });
    let before = parse_jsonc(current.document.source_jsonc.expose())?;
    let mut after = before.clone();
    after.nodes.retain(|id, _| input.nodes.contains_key(id));
    let source = rewrite_source(current.document.source_jsonc.expose(), &before, &after)?;
    let mut draft = Draft::new(input, source, current);
    draft.retain_rule_sets(tx, current)?;
    draft.cause = ChangeCause::MemberRemoved;
    Ok(draft)
}
