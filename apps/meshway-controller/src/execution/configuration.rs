//! Configuration owner participant: final identity checks and durable effects.
use crate::{
    configuration::{self, CommitParticipant, Error, PreparedConfiguration, Principal, Snapshot},
    enrollment::NodeAdmission,
    secrets::Sealer,
    store::{
        self,
        execution::{self as node_store, user_binding},
    },
};
use meshway_configuration::{
    NetworkConfigurationInput, NodeIdentity, UserBinding, ValidationContext,
};
use meshway_model::{NetworkId, NodeId, PublicationId, Revision};
use rusqlite::{Transaction, params};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct Participant<'a> {
    pub(crate) sealer: &'a Sealer,
    pub(crate) admission: Option<&'a NodeAdmission>,
}
pub(crate) fn check_inputs(
    tx: &Transaction<'_>,
    current: &Snapshot,
    input: &NetworkConfigurationInput,
) -> Result<(), Error> {
    let nodes = inventory_ids(tx, current.network_id)?;
    if input.nodes.keys().any(|id| !nodes.contains(id)) {
        return Err(Error::DependencyUnavailable);
    }
    // Remote artifacts are supplied by the resources owner, not reconstructed
    // from URL declarations or represented as a fabricated empty dataset.
    if input
        .rule_sets
        .values()
        .any(|r| matches!(r, meshway_configuration::RuleSet::Remote { .. }))
    {
        return Err(Error::DependencyUnavailable);
    }
    Ok(())
}
impl CommitParticipant for Participant<'_> {
    fn revalidate(
        &self,
        tx: &Transaction<'_>,
        principal: &Principal,
        current: &Snapshot,
        prepared: &PreparedConfiguration,
    ) -> Result<(), Error> {
        let now = node_store::now().map_err(map_error)?;
        match principal {
            Principal::Session(digest) => {
                let user = store::authorization::current_session(tx, digest, now)
                    .map_err(|_| Error::InvalidSession)?
                    .user
                    .id;
                configuration::authorize_manager(tx, current.network_id, user)?;
            }
            Principal::Lifecycle(actor) => {
                if let Some(sync) = &prepared.email_sync {
                    if sync.user != *actor
                        || !current
                            .bindings
                            .users
                            .get(&sync.old_email)
                            .is_some_and(|b| b.user_id == *actor)
                    {
                        return Err(Error::PermissionDenied);
                    }
                    let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND email=?2 AND disabled_at IS NULL)",params![actor.as_uuid().as_bytes().as_slice(),sync.new_email],|r|r.get(0))?;
                    if !valid {
                        return Err(Error::PermissionDenied);
                    }
                    let mut expected =
                        NetworkConfigurationInput::from(&current.document.configuration);
                    meshway_configuration::rename_user_references(
                        &mut expected,
                        &sync.old_email,
                        &sync.new_email,
                    );
                    if expected != NetworkConfigurationInput::from(&prepared.document.configuration)
                    {
                        return Err(Error::PermissionDenied);
                    }
                } else {
                    let admission = self.admission.ok_or(Error::PermissionDenied)?;
                    if admission.actor != *actor || admission.network != current.network_id {
                        return Err(Error::PermissionDenied);
                    }
                    store::enrollment::verify_admission(tx, self.sealer, admission)
                        .map_err(map_error)?;
                }
            }
        }
        let target = &prepared.document.configuration;
        let inventory = inventory_ids(tx, current.network_id)?;
        for (id, node) in &target.nodes {
            if !inventory.contains(id) {
                return Err(Error::DependencyUnavailable);
            }
            let previous = current.document.configuration.nodes.get(id);
            if previous.is_none() && self.admission.is_none_or(|a| a.node != *id) {
                return Err(Error::PermissionDenied);
            }
            if previous.map(|n| &n.identity) != Some(&node.identity) {
                let admitted = self
                    .admission
                    .is_some_and(|a| a.node == *id && a.identity == node.identity);
                let administrative_tag = matches!(principal, Principal::Session(_))
                    && previous.is_some()
                    && matches!(&node.identity,NodeIdentity::Tag{tags} if !tags.is_empty());
                let email_sync = prepared.email_sync.is_some()
                    && previous.is_some_and(|n| matches!(&n.identity, NodeIdentity::User { .. }))
                    && user_binding(target, &prepared.bindings, *id)
                        == user_binding(&current.document.configuration, &current.bindings, *id);
                if !admitted && !administrative_tag && !email_sync {
                    return Err(Error::PermissionDenied);
                }
            }
            node_store::identity_eligible(tx, target, &prepared.bindings, current.network_id, *id)
                .map_err(map_error)?;
        }
        if let Some(a) = self.admission {
            // Admission can change one node, never carry unrelated edits under a
            // valid login/key proof. Source formatting is handled by Config.
            let mut expected = current.document.configuration.clone();
            let admitted = target.nodes.get(&a.node).ok_or(Error::PermissionDenied)?;
            expected.nodes.insert(a.node, admitted.clone());
            if expected != *target {
                return Err(Error::PermissionDenied);
            }
        }
        Ok(())
    }
    fn apply(
        &self,
        tx: &Transaction<'_>,
        current: &Snapshot,
        prepared: &PreparedConfiguration,
        _revision: Revision,
    ) -> Result<(Vec<PublicationId>, BTreeSet<NodeId>), Error> {
        let publication = node_store::apply_configuration_effects(
            tx,
            self.sealer,
            current.network_id,
            &current.document.configuration,
            &prepared.document.configuration,
            node_store::now().map_err(map_error)?,
        )
        .map_err(map_error)?;
        Ok((
            vec![publication],
            prepared
                .document
                .configuration
                .nodes
                .keys()
                .copied()
                .collect(),
        ))
    }
}
pub(crate) fn map_error(error: crate::execution::NodeError) -> Error {
    match error {
        crate::execution::NodeError::Unauthorized => Error::InvalidSession,
        crate::execution::NodeError::Forbidden => Error::PermissionDenied,
        crate::execution::NodeError::Conflict => Error::Conflict,
        crate::execution::NodeError::NotFound => Error::NotFound,
        crate::execution::NodeError::Storage => Error::Storage,
        _ => Error::DependencyUnavailable,
    }
}
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
        .map(|row| node_store::id(row?).map_err(map_error))
        .collect()
}
pub(crate) fn members(
    tx: &Transaction<'_>,
    network: NetworkId,
) -> Result<BTreeMap<String, UserBinding>, Error> {
    let mut statement=tx.prepare("SELECT u.email,u.id,m.id FROM memberships m JOIN users u ON u.id=m.user_id WHERE m.network_id=?1 AND m.revoked_at IS NULL AND u.disabled_at IS NULL")?;
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
                    user_id: node_store::id(user).map_err(map_error)?,
                    membership_id: node_store::id(member).map_err(map_error)?,
                },
            ))
        })
        .collect()
}
pub(crate) fn validation_context<'a>(
    tx: &Transaction<'_>,
    current: &'a Snapshot,
    input: &NetworkConfigurationInput,
    admission: Option<&NodeAdmission>,
    manager: bool,
) -> Result<ValidationContext<'a>, Error> {
    let mut approved = BTreeMap::new();
    for (id, node) in &input.nodes {
        let changed = current
            .document
            .configuration
            .nodes
            .get(id)
            .map(|n| &n.identity)
            != Some(&node.identity);
        if changed
            && (admission.is_some_and(|a| a.node == *id && a.identity == node.identity)
                || (manager
                    && current.document.configuration.nodes.contains_key(id)
                    && matches!(&node.identity,NodeIdentity::Tag{tags} if !tags.is_empty())))
        {
            approved.insert(*id, node.identity.clone());
        }
    }
    Ok(ValidationContext {
        members: members(tx, current.network_id)?,
        nodes: inventory_ids(tx, current.network_id)?,
        previous: Some(&current.document.configuration),
        expected_user_bindings: current.bindings.users.clone(),
        previous_user_bindings: current.bindings.users.clone(),
        approved_identities: approved,
        ..Default::default()
    })
}
