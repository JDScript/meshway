//! Readable-name maintenance after an account changes its e-mail. Only the
//! bindings of the user's active membership are renamed: an e-mail reused by
//! another account, or a revoked membership of the same user, keeps its name
//! and never learns the new address.
use super::{Committer, Draft, Intent, Snapshot};
use crate::configuration::{ChangeCause, CommitEffects};
use crate::error::Error;
use meshway_configuration::{NetworkConfigurationInput, UserBinding, rewrite_source};
use meshway_model::{MembershipId, RequestId, UserId};
use rusqlite::Transaction;
use std::collections::BTreeMap;

impl Committer {
    /// One version per network in which the membership's name is bound.
    pub(crate) fn synchronize_email(
        &self,
        tx: &Transaction<'_>,
        user: UserId,
        old_email: &str,
        new_email: &str,
        request: RequestId,
    ) -> Result<Vec<CommitEffects>, Error> {
        let memberships = crate::store::configuration::memberships::for_user(tx, user)?;
        let mut effects = Vec::new();
        for (network, membership) in memberships {
            let binding = UserBinding {
                user_id: user,
                membership_id: membership,
            };
            let current = self.current(tx, network)?;
            if !bound(&current, &binding, old_email) {
                continue;
            }
            effects.push(self.commit(
                tx,
                network,
                Intent::EmailSync {
                    user,
                    membership,
                    old_email,
                    new_email,
                },
                request,
            )?);
        }
        Ok(effects)
    }
}

fn bound(current: &Snapshot, binding: &UserBinding, old_email: &str) -> bool {
    current.bindings.users.get(old_email) == Some(binding)
        || current.bindings.node_users.values().any(|b| b == binding)
}

/// The membership is still active and the account already carries the new
/// address; the user is the actor of their own maintenance.
pub(super) fn authorize(
    tx: &Transaction<'_>,
    current: &Snapshot,
    user: UserId,
    membership: MembershipId,
    new_email: &str,
) -> Result<UserId, Error> {
    let (active, valid) = crate::store::configuration::memberships::email_sync_status(
        tx,
        current.network_id,
        user,
        membership,
        new_email,
    )?;
    if !active || !valid {
        return Err(Error::PermissionDenied);
    }
    Ok(user)
}

pub(super) fn author(
    tx: &Transaction<'_>,
    current: &Snapshot,
    binding: &UserBinding,
    old_email: &str,
    new_email: &str,
) -> Result<Draft, Error> {
    if !bound(current, binding, old_email) {
        return Err(Error::PermissionDenied);
    }
    let mut input = NetworkConfigurationInput::from(&current.document.configuration);
    rename_bound_user_references(&mut input, current, binding, old_email, new_email);
    let before = meshway_configuration::parse_jsonc(current.document.source_jsonc.expose())?;
    let mut after = before.clone();
    rename_bound_user_references(&mut after, current, binding, old_email, new_email);
    let source = rewrite_source(current.document.source_jsonc.expose(), &before, &after)?;
    let mut draft = Draft::new(input, source, current);
    if draft.expected_user_bindings.get(old_email) == Some(binding)
        && let Some(moved) = draft.expected_user_bindings.remove(old_email)
    {
        draft.expected_user_bindings.insert(new_email.into(), moved);
    }
    draft.retain_rule_sets(tx, current)?;
    draft.cause = ChangeCause::EmailSync;
    Ok(draft)
}

/// Renames policy references bound to this exact membership and the identity
/// of the User nodes it resolves.
pub(crate) fn rename_bound_user_references(
    input: &mut NetworkConfigurationInput,
    snapshot: &Snapshot,
    binding: &UserBinding,
    old: &str,
    new: &str,
) {
    if snapshot.bindings.users.get(old) == Some(binding) {
        meshway_configuration::rebind_policy_user_references(
            input,
            &BTreeMap::from([(old.to_owned(), new.to_owned())]),
        );
    }
    let names = snapshot
        .bindings
        .node_users
        .iter()
        .filter(|(_, bound)| *bound == binding)
        .map(|(id, _)| (*id, new.to_owned()))
        .collect();
    meshway_configuration::rebind_node_user_references(input, &names);
}
