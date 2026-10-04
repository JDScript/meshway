//! Removing a member permanently deletes that membership's User devices.
use super::{Draft, Snapshot};
use crate::configuration::ChangeCause;
use crate::error::Error;
use crate::store::{self, authorization::current_session};
use meshway_configuration::{NetworkConfigurationInput, parse_jsonc, rewrite_source};
use meshway_model::{MembershipId, MembershipRole, UserId, Version};
use rusqlite::{OptionalExtension, Transaction, params};

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
    let network = current.network_id.as_uuid().as_bytes().as_slice();
    let (caller_id, caller_role): (Vec<u8>, String) = tx
        .query_row(
            "SELECT id,role FROM memberships WHERE network_id=?1 AND user_id=?2 AND revoked_at IS NULL",
            params![network, actor.as_uuid().as_bytes().as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(Error::PermissionDenied)?;
    let (target_role, actual): (String, i64) = tx
        .query_row(
            "SELECT role,version FROM memberships WHERE network_id=?1 AND id=?2 AND revoked_at IS NULL",
            params![network, membership.as_uuid().as_bytes().as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(Error::PermissionDenied)?;
    if actual != version.get() as i64 {
        return Err(Error::VersionConflict);
    }
    let owners: i64 = tx.query_row(
        "SELECT count(*) FROM memberships WHERE network_id=?1 AND role='owner' AND revoked_at IS NULL",
        [network],
        |r| r.get(0),
    )?;
    let allowed = store::authorization::member_actions(
        (store::execution::id(caller_id)?, role(&caller_role)?),
        (membership, role(&target_role)?),
        owners,
    )
    .remove;
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

fn role(value: &str) -> Result<MembershipRole, Error> {
    match value {
        "owner" => Ok(MembershipRole::Owner),
        "admin" => Ok(MembershipRole::Admin),
        "member" => Ok(MembershipRole::Member),
        _ => Err(Error::decode("memberships.role")),
    }
}
