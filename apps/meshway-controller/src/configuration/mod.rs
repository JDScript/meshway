//! One complete version/CAS boundary for all administrative and lifecycle edits.
//! Preparation is in-memory work, not a two-phase publication protocol.
pub(crate) mod catalog;
pub(crate) mod materials;
mod service;
mod snapshot;
pub(crate) use service::Configurations;
pub(crate) use service::source_response;
pub(crate) use snapshot::{Snapshot, SnapshotCache};

use crate::{
    accounts::random_uuid,
    networks::configuration::ConfigurationBindings,
    secrets::{Purpose, Sealer},
    store::authorization::current_session,
};
use meshway_configuration::{
    ConfigurationChange, ConfigurationDocument, ConfigurationError, NetworkConfigurationInput,
    ValidationContext, canonical_json, configuration_changes, normalize,
};
use meshway_model::{ArtifactId, NetworkId, NodeId, PublicationId, RequestId, Revision, UserId};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::{BTreeMap, BTreeSet};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("invalid or expired session")]
    InvalidSession,
    #[error("resource not found")]
    NotFound,
    #[error("permission denied")]
    PermissionDenied,
    #[error("configuration revision changed")]
    Conflict,
    #[error("configuration dependencies are unavailable")]
    DependencyUnavailable,
    #[error("configuration input is invalid")]
    Invalid(#[from] ConfigurationError),
    #[error("configuration storage failed")]
    Storage,
    #[error("invalid or expired pagination")]
    InvalidPage,
}
impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

pub(crate) struct TrustedContext<'a> {
    pub(crate) validation: ValidationContext<'a>,
    pub(crate) rule_sets: BTreeMap<String, ArtifactId>,
}

pub(crate) struct PreparedConfiguration {
    pub(crate) network_id: NetworkId,
    pub(crate) base_revision: Revision,
    pub(crate) document: ConfigurationDocument,
    pub(crate) bindings: ConfigurationBindings,
    pub(crate) changes: Vec<ConfigurationChange>,
    pub(crate) deleted_node_ids: BTreeSet<NodeId>,
    pub(crate) address_changed_node_ids: BTreeSet<NodeId>,
    pub(crate) source_changed: bool,
    pub(crate) rollback_from: Option<Revision>,
    pub(crate) email_sync: Option<EmailSync>,
}
pub(crate) struct EmailSync {
    pub(crate) user: UserId,
    pub(crate) old_email: String,
    pub(crate) new_email: String,
}

/// Actor proof is rechecked by the final transaction. System callers supply an
/// independently authorized lifecycle operation through CommitParticipant.
pub(crate) enum Principal {
    Session([u8; 32]),
    Lifecycle(UserId),
}

pub(crate) use meshway_protocol::history::ConfigurationChangeCause as ChangeCause;
pub(crate) struct CommitMetadata {
    pub(crate) cause: ChangeCause,
    pub(crate) request: RequestId,
}
pub(crate) struct CommitRuntime<'a> {
    pub(crate) sealer: &'a Sealer,
    pub(crate) participant: &'a dyn CommitParticipant,
}

pub(crate) struct CommitEffects {
    pub(crate) network_id: NetworkId,
    pub(crate) revision: Revision,
    pub(crate) publication_ids: Vec<PublicationId>,
    pub(crate) target_node_ids: BTreeSet<NodeId>,
    pub(crate) deleted_node_ids: BTreeSet<NodeId>,
}

/// B/C integration is mandatory for any Node/resource dependency. Both methods
/// execute within the caller's one SQL transaction; errors roll everything back.
pub(crate) trait CommitParticipant: Send + Sync {
    fn revalidate(
        &self,
        tx: &Transaction<'_>,
        principal: &Principal,
        current: &Snapshot,
        prepared: &PreparedConfiguration,
    ) -> Result<(), Error>;
    fn apply(
        &self,
        tx: &Transaction<'_>,
        current: &Snapshot,
        prepared: &PreparedConfiguration,
        revision: Revision,
    ) -> Result<(Vec<PublicationId>, BTreeSet<NodeId>), Error>;
}

pub(crate) fn prepare(
    input: NetworkConfigurationInput,
    source: String,
    current: &Snapshot,
    context: &TrustedContext<'_>,
) -> Result<PreparedConfiguration, Error> {
    if context.validation.previous != Some(&current.document.configuration) {
        return Err(Error::DependencyUnavailable);
    }
    let (configuration, user_bindings) = normalize(input, &context.validation)?;
    let changes = configuration_changes(&current.document.configuration, &configuration)?;
    let old = &current.document.configuration;
    let deleted_node_ids = old
        .nodes
        .keys()
        .filter(|id| !configuration.nodes.contains_key(id))
        .copied()
        .collect();
    let address_changed_node_ids = old
        .nodes
        .iter()
        .filter(|(id, n)| {
            configuration.nodes.get(id).map(|new| &new.addresses) != Some(&n.addresses)
        })
        .map(|(id, _)| *id)
        .collect();
    let next = Revision::new(
        current
            .revision
            .get()
            .checked_add(1)
            .ok_or(Error::Storage)?,
    )
    .map_err(|_| Error::Storage)?;
    let listeners = configuration
        .nodes
        .iter()
        .map(|(id, node)| {
            (
                *id,
                node.listeners
                    .keys()
                    .map(|key| {
                        (
                            key.clone(),
                            current
                                .bindings
                                .listeners
                                .get(id)
                                .and_then(|m| m.get(key))
                                .copied()
                                .unwrap_or(next),
                        )
                    })
                    .collect(),
            )
        })
        .collect();
    let source_changed = current.document.source_jsonc.expose() != source;
    let mut document = current.document.clone();
    document.set_source(source);
    document.configuration = configuration;
    document.user_bindings = user_bindings.clone();
    Ok(PreparedConfiguration {
        network_id: current.network_id,
        base_revision: current.revision,
        document,
        bindings: ConfigurationBindings {
            users: user_bindings,
            listeners,
            rule_sets: context.rule_sets.clone(),
        },
        changes,
        deleted_node_ids,
        address_changed_node_ids,
        source_changed,
        rollback_from: None,
        email_sync: None,
    })
}

/// Common atomic write path. No cache or external event is published until the
/// caller commits. It must deliver CommitEffects through the committed outbox.
pub(crate) fn apply_in_transaction(
    tx: &Transaction<'_>,
    principal: &Principal,
    expected_revision: Revision,
    current: &Snapshot,
    prepared: &PreparedConfiguration,
    metadata: CommitMetadata,
    runtime: CommitRuntime<'_>,
) -> Result<CommitEffects, Error> {
    let CommitMetadata { cause, request } = metadata;
    let CommitRuntime {
        sealer,
        participant,
    } = runtime;
    let now = chrono::Utc::now().timestamp_millis();
    let actor = match principal {
        Principal::Session(digest) => {
            let user = current_session(tx, digest, now)
                .map_err(|_| Error::InvalidSession)?
                .user
                .id;
            authorize_manager(tx, current.network_id, user)?;
            user
        }
        Principal::Lifecycle(user) => *user,
    };
    let actual = tx
        .query_row(
            "SELECT current_revision FROM networks WHERE id=?1 AND deleted_at IS NULL",
            [current.network_id.as_uuid().as_bytes().as_slice()],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    if actual != expected_revision.get() as i64
        || current.revision != expected_revision
        || prepared.base_revision != expected_revision
        || prepared.network_id != current.network_id
    {
        return Err(Error::Conflict);
    }
    participant.revalidate(tx, principal, current, prepared)?;
    let changed = !prepared.changes.is_empty()
        || prepared.source_changed
        || prepared.bindings != current.bindings
        || prepared.rollback_from.is_some();
    if !changed {
        return Ok(CommitEffects {
            network_id: current.network_id,
            revision: current.revision,
            publication_ids: Vec::new(),
            target_node_ids: BTreeSet::new(),
            deleted_node_ids: BTreeSet::new(),
        });
    }
    let revision = Revision::new(
        expected_revision
            .get()
            .checked_add(1)
            .ok_or(Error::Storage)?,
    )
    .map_err(|_| Error::Storage)?;
    let canonical = canonical_json(&prepared.document.configuration)?;
    let bindings =
        Zeroizing::new(serde_json::to_vec(&prepared.bindings).map_err(|_| Error::Storage)?);
    let source = sealer
        .seal(
            current.network_id,
            revision,
            Purpose::Source,
            prepared.document.source_jsonc.expose().as_bytes(),
        )
        .map_err(|_| Error::Storage)?;
    let configuration = sealer
        .seal(
            current.network_id,
            revision,
            Purpose::Configuration,
            canonical.expose().as_bytes(),
        )
        .map_err(|_| Error::Storage)?;
    let bindings = sealer
        .seal(current.network_id, revision, Purpose::Bindings, &bindings)
        .map_err(|_| Error::Storage)?;
    tx.execute("INSERT INTO configuration_versions(network_id,revision,source_jsonc,configuration,bindings,created_by,created_at,cause,request_id,rollback_from) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![current.network_id.as_uuid().as_bytes().as_slice(),revision.get() as i64,source.as_bytes(),configuration.as_bytes(),bindings.as_bytes(),actor.as_uuid().as_bytes().as_slice(),now,cause.as_str(),request.as_uuid().as_bytes().as_slice(),prepared.rollback_from.map(|r|r.get() as i64)])?;
    tx.execute(
        "UPDATE networks SET current_revision=?1 WHERE id=?2 AND current_revision=?3",
        params![
            revision.get() as i64,
            current.network_id.as_uuid().as_bytes().as_slice(),
            expected_revision.get() as i64
        ],
    )?;
    let (publication_ids, target_node_ids) = participant.apply(tx, current, prepared, revision)?;
    tx.execute("INSERT INTO activity_events(id,scope,network_id,at,actor_user_id,action,request_id,before_revision,after_revision) VALUES(?1,'network',?2,?3,?4,'configuration.apply',?5,?6,?7)",params![random_uuid().map_err(|_|Error::Storage)?.as_bytes().as_slice(),current.network_id.as_uuid().as_bytes().as_slice(),now,actor.as_uuid().as_bytes().as_slice(),request.as_uuid().as_bytes().as_slice(),expected_revision.get() as i64,revision.get() as i64])?;
    Ok(CommitEffects {
        network_id: current.network_id,
        revision,
        publication_ids,
        target_node_ids,
        deleted_node_ids: prepared.deleted_node_ids.clone(),
    })
}

pub(crate) fn authorize_manager(
    tx: &Transaction<'_>,
    network: NetworkId,
    user: UserId,
) -> Result<(), Error> {
    let role=tx.query_row("SELECT m.role FROM memberships m JOIN networks n ON n.id=m.network_id WHERE m.network_id=?1 AND m.user_id=?2 AND m.revoked_at IS NULL AND n.deleted_at IS NULL",params![network.as_uuid().as_bytes().as_slice(),user.as_uuid().as_bytes().as_slice()],|r|r.get::<_,String>(0)).optional()?.ok_or(Error::NotFound)?;
    if !matches!(role.as_str(), "owner" | "admin") {
        return Err(Error::PermissionDenied);
    }
    Ok(())
}
