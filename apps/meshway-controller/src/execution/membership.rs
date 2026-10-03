//! Member removal participates in the one authoritative configuration commit.
use crate::{configuration::*, secrets::Sealer, store::authorization::current_session};
use meshway_configuration::{NetworkConfigurationInput, parse_jsonc, rewrite_source};
use meshway_model::{
    MembershipId, MembershipRole, NetworkId, NodeId, PublicationId, RequestId, Revision, Version,
};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::BTreeSet;

pub(crate) struct MembershipRemoval {
    pub digest: [u8; 32],
    pub network: NetworkId,
    pub membership: MembershipId,
    pub version: Version,
    pub request: RequestId,
}
struct RemovalParticipant<'a> {
    removal: &'a MembershipRemoval,
    sealer: &'a Sealer,
}

pub(crate) fn remove_nodes(
    tx: &Transaction<'_>,
    removal: &MembershipRemoval,
    sealer: &Sealer,
    cache: &SnapshotCache,
) -> Result<Revision, Error> {
    let (current_revision, _, _) =
        crate::store::execution::current_configuration(tx, sealer, removal.network)
            .map_err(super::configuration::map_error)?;
    let current = cache.load(tx, removal.network, current_revision, sealer)?;
    let mut input = NetworkConfigurationInput::from(&current.document.configuration);
    input.nodes.retain(|id, _| {
        crate::store::execution::user_binding(
            &current.document.configuration,
            &current.bindings,
            *id,
        )
        .is_none_or(|binding| binding.membership_id != removal.membership)
    });
    let before = parse_jsonc(current.document.source_jsonc.expose())?;
    let mut after = before.clone();
    after.nodes.retain(|id, _| input.nodes.contains_key(id));
    let source = rewrite_source(current.document.source_jsonc.expose(), &before, &after)?;
    let verified = materials::prepare_materials(
        tx,
        removal.network,
        &mut input,
        &current,
        sealer,
        false,
        &BTreeSet::new(),
    )?;
    let mut validation =
        super::configuration::validation_context(tx, &current, &input, None, false)?;
    validation.verified_listeners = verified;
    let prepared = prepare(
        input,
        source,
        &current,
        &TrustedContext {
            validation,
            rule_sets: current.bindings.rule_sets.clone(),
        },
    )?;
    let actor = current_session(tx, &removal.digest, chrono::Utc::now().timestamp_millis())
        .map_err(|_| Error::InvalidSession)?
        .user
        .id;
    let effects = apply_in_transaction(
        tx,
        &Principal::Lifecycle(actor),
        current.revision,
        &current,
        &prepared,
        CommitMetadata {
            cause: ChangeCause::MemberRemoved,
            request: removal.request,
        },
        CommitRuntime {
            sealer,
            participant: &RemovalParticipant { removal, sealer },
        },
    )?;
    Ok(effects.revision)
}

impl CommitParticipant for RemovalParticipant<'_> {
    fn revalidate(
        &self,
        tx: &Transaction<'_>,
        principal: &Principal,
        current: &Snapshot,
        prepared: &PreparedConfiguration,
    ) -> Result<(), Error> {
        let actor = current_session(
            tx,
            &self.removal.digest,
            chrono::Utc::now().timestamp_millis(),
        )
        .map_err(|_| Error::InvalidSession)?
        .user
        .id;
        if !matches!(principal, Principal::Lifecycle(id) if *id == actor)
            || current.network_id != self.removal.network
        {
            return Err(Error::PermissionDenied);
        }
        let (caller_id, caller_role): (Vec<u8>, String) = tx.query_row("SELECT id,role FROM memberships WHERE network_id=?1 AND user_id=?2 AND revoked_at IS NULL",
            params![current.network_id.as_uuid().as_bytes().as_slice(), actor.as_uuid().as_bytes().as_slice()], |r| Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(Error::PermissionDenied)?;
        let (target_role, version): (String, i64) = tx.query_row("SELECT role,version FROM memberships WHERE network_id=?1 AND id=?2 AND revoked_at IS NULL",
            params![current.network_id.as_uuid().as_bytes().as_slice(), self.removal.membership.as_uuid().as_bytes().as_slice()], |r| Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(Error::PermissionDenied)?;
        if version != self.removal.version.get() as i64 {
            return Err(Error::Conflict);
        }
        let caller_id =
            crate::store::execution::id(caller_id).map_err(super::configuration::map_error)?;
        let owners: i64 = tx.query_row("SELECT count(*) FROM memberships WHERE network_id=?1 AND role='owner' AND revoked_at IS NULL", [current.network_id.as_uuid().as_bytes().as_slice()], |r|r.get(0))?;
        if !crate::store::authorization::member_actions(
            (caller_id, role(&caller_role)?),
            (self.removal.membership, role(&target_role)?),
            owners,
        )
        .remove
        {
            return Err(Error::PermissionDenied);
        }
        let mut expected = current.document.configuration.clone();
        expected.nodes.retain(|id, _| {
            crate::store::execution::user_binding(
                &current.document.configuration,
                &current.bindings,
                *id,
            )
            .is_none_or(|binding| binding.membership_id != self.removal.membership)
        });
        if expected != prepared.document.configuration {
            return Err(Error::PermissionDenied);
        }
        Ok(())
    }
    fn apply(
        &self,
        tx: &Transaction<'_>,
        current: &Snapshot,
        prepared: &PreparedConfiguration,
        revision: Revision,
    ) -> Result<(Vec<PublicationId>, BTreeSet<NodeId>), Error> {
        super::configuration::Participant {
            sealer: self.sealer,
            admission: None,
        }
        .apply(tx, current, prepared, revision)
    }
}
fn role(value: &str) -> Result<MembershipRole, Error> {
    match value {
        "owner" => Ok(MembershipRole::Owner),
        "admin" => Ok(MembershipRole::Admin),
        "member" => Ok(MembershipRole::Member),
        _ => Err(Error::Storage),
    }
}
