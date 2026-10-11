//! The one transaction entry for every configuration version. Edits, node
//! admission, member removal and e-mail maintenance are intents; they differ
//! in who is authorized and how the draft is authored, and share everything
//! after that: materials, normalization, scope check, version, publication.
mod edit;
mod email;
mod enrollment;
mod leave;
mod removal;

use super::{
    ChangeCause, CommitEffects, PreparedConfiguration, Snapshot, SnapshotCache, context, materials,
    prepare, resources,
};
use crate::error::Error;
use crate::{
    enrollment::NodeAdmission,
    secrets::{Purpose, Sealer},
    store::{self, authorization::current_session},
};
use meshway_configuration::{NetworkConfigurationInput, NodeIdentity, UserBinding, canonical_json};
use meshway_model::{
    ArtifactId, MembershipId, NetworkId, NodeId, RequestId, Revision, UserId, Version,
};
use meshway_protocol::configuration::{ConfigurationPreviewResponse, ConfigurationRequest};
use rusqlite::Transaction;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use zeroize::Zeroizing;

/// What a caller wants the next version to be and under which authority.
pub(crate) enum Intent<'a> {
    /// A manager's JSONC document, typed operations or restore.
    Edit {
        session: [u8; 32],
        request: &'a ConfigurationRequest,
        resources: Option<&'a resources::Preparation>,
    },
    /// A verified enrollment adds the node or transitions its identity.
    Enrollment { admission: &'a NodeAdmission },
    /// Removing a member deletes that member's User devices.
    MemberRemoval {
        session: [u8; 32],
        membership: MembershipId,
        version: Version,
    },
    /// An authenticated Node deletes itself.
    NodeLeave {
        principal: &'a crate::execution::NodePrincipal,
    },
    /// Renames the readable references of one active membership.
    EmailSync {
        user: UserId,
        membership: MembershipId,
        old_email: &'a str,
        new_email: &'a str,
    },
}

/// The authored target of an intent, before materials and normalization.
pub(crate) struct Draft {
    pub(crate) input: NetworkConfigurationInput,
    pub(crate) source: String,
    pub(crate) expected_user_bindings: BTreeMap<String, UserBinding>,
    pub(crate) expected_node_user_bindings: BTreeMap<NodeId, UserBinding>,
    pub(crate) approved_identities: BTreeMap<NodeId, NodeIdentity>,
    pub(crate) recreated_listeners: BTreeSet<(NodeId, String)>,
    pub(crate) rule_sets: BTreeMap<String, ArtifactId>,
    pub(crate) rule_set_contents: BTreeMap<String, meshway_model::resources::RuleSetData>,
    /// A preview found downloaded content that is not stored yet.
    pub(crate) new_content: bool,
    pub(crate) rollback_from: Option<Revision>,
    pub(crate) cause: ChangeCause,
}

impl Draft {
    fn new(input: NetworkConfigurationInput, source: String, current: &Snapshot) -> Self {
        Self {
            input,
            source,
            expected_user_bindings: current.bindings.users.clone(),
            expected_node_user_bindings: current.bindings.node_users.clone(),
            approved_identities: BTreeMap::new(),
            recreated_listeners: BTreeSet::new(),
            rule_sets: BTreeMap::new(),
            rule_set_contents: BTreeMap::new(),
            new_content: false,
            rollback_from: None,
            cause: ChangeCause::Edit,
        }
    }
    /// Rule-set bindings retained from the current version for this input.
    fn retain_rule_sets(&mut self, tx: &Transaction<'_>, current: &Snapshot) -> Result<(), Error> {
        let (bindings, contents) = crate::resources::bindings::for_input(tx, current, &self.input)?;
        self.rule_sets = bindings;
        self.rule_set_contents = contents;
        Ok(())
    }
}

pub(crate) struct EvaluationDraft {
    pub(crate) prepared: PreparedConfiguration,
    pub(crate) rule_set_contents: BTreeMap<String, meshway_model::resources::RuleSetData>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Commit,
    Preview,
}

/// Owner of configuration versions. Shared by every service that commits one.
#[derive(Clone)]
pub(crate) struct Committer {
    pub(crate) sealer: Arc<Sealer>,
    pub(crate) cache: Arc<SnapshotCache>,
}

impl Committer {
    pub(crate) fn new(sealer: Arc<Sealer>, cache: Arc<SnapshotCache>) -> Self {
        Self { sealer, cache }
    }
    pub(crate) fn current(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
    ) -> Result<Arc<Snapshot>, Error> {
        self.cache.load(
            tx,
            network,
            store::configuration::versions::current_revision(tx, network)?,
            &self.sealer,
        )
    }

    /// Commits the intent as the next version of `network` inside `tx`. No
    /// cache or event is published until the caller commits; the returned
    /// effects go through the committed outbox.
    pub(crate) fn commit(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        intent: Intent<'_>,
        request: RequestId,
    ) -> Result<CommitEffects, Error> {
        let now = chrono::Utc::now().timestamp_millis();
        let current = self.current(tx, network)?;
        let actor = self.authorize(tx, &current, &intent, now)?;
        let draft = self.author(tx, &current, &intent, Mode::Commit)?;
        let prepared = self.prepare(tx, &current, &intent, draft, Mode::Commit)?;
        scope(tx, &current, &intent, &prepared)?;
        let event_actor = match &intent {
            Intent::NodeLeave { principal } => meshway_protocol::events::EventActor::Node {
                node_id: principal.node_id(),
            },
            _ => meshway_protocol::events::EventActor::User { user_id: actor },
        };
        self.persist(tx, &current, prepared, actor, event_actor, request, now)
    }

    /// Runs the same authoring, material inheritance and normalization as a
    /// commit, without generating materials or writing anything.
    pub(crate) fn preview(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        intent: Intent<'_>,
        id: RequestId,
    ) -> Result<ConfigurationPreviewResponse, Error> {
        let now = chrono::Utc::now().timestamp_millis();
        let current = self.current(tx, network)?;
        self.authorize(tx, &current, &intent, now)?;
        let draft = self.author(tx, &current, &intent, Mode::Preview)?;
        let resources = match &intent {
            Intent::Edit { resources, .. } => *resources,
            _ => None,
        };
        let (draft, context) = self.materials(tx, &current, &intent, draft, Mode::Preview)?;
        super::preview::build(&current, draft, context, resources, id)
    }

    /// The configuration a draft would become, with the rule-set content it
    /// would bind, for evaluation only: nothing is generated or persisted.
    /// Missing listener materials are reported as preparation requirements.
    pub(crate) fn draft(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        intent: Intent<'_>,
    ) -> Result<(Arc<Snapshot>, EvaluationDraft), Error> {
        let now = chrono::Utc::now().timestamp_millis();
        let current = self.current(tx, network)?;
        self.authorize(tx, &current, &intent, now)?;
        let draft = self.author(tx, &current, &intent, Mode::Preview)?;
        let (draft, context) = self.materials(tx, &current, &intent, draft, Mode::Preview)?;
        let Draft {
            input,
            source,
            rule_sets,
            rule_set_contents,
            ..
        } = draft;
        let prepared = prepare(input, source, &current, &context, rule_sets)?;
        Ok((
            current,
            EvaluationDraft {
                prepared,
                rule_set_contents,
            },
        ))
    }

    fn authorize(
        &self,
        tx: &Transaction<'_>,
        current: &Snapshot,
        intent: &Intent<'_>,
        now: i64,
    ) -> Result<UserId, Error> {
        match intent {
            Intent::Edit {
                session, request, ..
            } => {
                let user = current_session(tx, session, now)?.user.id;
                store::authorization::authorize_manager(tx, current.network_id, user)?;
                if request.expected_revision() != current.revision {
                    return Err(Error::RevisionConflict);
                }
                Ok(user)
            }
            Intent::Enrollment { admission } => {
                if admission.network != current.network_id {
                    return Err(Error::PermissionDenied);
                }
                store::enrollment::verify_admission(tx, &self.sealer, admission)?;
                Ok(admission.actor)
            }
            Intent::MemberRemoval {
                session,
                membership,
                version,
            } => removal::authorize(tx, current, session, *membership, *version, now),
            Intent::NodeLeave { principal } => leave::authorize(tx, current, principal),
            Intent::EmailSync {
                user,
                membership,
                new_email,
                ..
            } => email::authorize(tx, current, *user, *membership, new_email),
        }
    }

    fn author(
        &self,
        tx: &Transaction<'_>,
        current: &Snapshot,
        intent: &Intent<'_>,
        mode: Mode,
    ) -> Result<Draft, Error> {
        let draft = match intent {
            Intent::Edit {
                request, resources, ..
            } => edit::author(self, tx, current, request, *resources, mode == Mode::Commit)?,
            Intent::Enrollment { admission } => enrollment::author(tx, current, admission)?,
            Intent::MemberRemoval { membership, .. } => removal::author(tx, current, *membership)?,
            Intent::NodeLeave { principal } => leave::author(tx, current, principal)?,
            Intent::EmailSync {
                user,
                membership,
                old_email,
                new_email,
            } => email::author(
                tx,
                current,
                &UserBinding {
                    user_id: *user,
                    membership_id: *membership,
                },
                old_email,
                new_email,
            )?,
        };
        context::check_inputs(tx, current, &draft.input)?;
        Ok(draft)
    }

    /// Inherits and verifies listener materials, generating missing ones only
    /// for a committing intent that may create credentials.
    fn materials<'a>(
        &self,
        tx: &Transaction<'_>,
        current: &'a Snapshot,
        intent: &Intent<'_>,
        mut draft: Draft,
        mode: Mode,
    ) -> Result<(Draft, meshway_configuration::ValidationContext<'a>), Error> {
        let generate = mode == Mode::Commit
            && matches!(intent, Intent::Edit { .. } | Intent::Enrollment { .. });
        let verified = materials::prepare_materials(
            tx,
            current.network_id,
            &mut draft.input,
            current,
            &self.sealer,
            generate,
            &draft.recreated_listeners,
        )?;
        let context = context::for_draft(tx, current, &draft, verified)?;
        Ok((draft, context))
    }

    fn prepare(
        &self,
        tx: &Transaction<'_>,
        current: &Snapshot,
        intent: &Intent<'_>,
        draft: Draft,
        mode: Mode,
    ) -> Result<PreparedConfiguration, Error> {
        let (draft, context) = self.materials(tx, current, intent, draft, mode)?;
        let Draft {
            input,
            source,
            rule_sets,
            rollback_from,
            cause,
            ..
        } = draft;
        let mut prepared = prepare(input, source, current, &context, rule_sets)?;
        prepared.rollback_from = rollback_from;
        prepared.cause = cause;
        Ok(prepared)
    }

    fn persist(
        &self,
        tx: &Transaction<'_>,
        current: &Snapshot,
        prepared: PreparedConfiguration,
        actor: UserId,
        event_actor: meshway_protocol::events::EventActor,
        request: RequestId,
        now: i64,
    ) -> Result<CommitEffects, Error> {
        let network = current.network_id;
        let current_revision_matches = store::configuration::versions::current_revision_matches(
            tx,
            network,
            current.revision,
        )?;
        if !current_revision_matches
            || prepared.base_revision != current.revision
            || prepared.network_id != network
        {
            return Err(Error::RevisionConflict);
        }
        let changed = !prepared.changes.is_empty()
            || prepared.source_changed
            || prepared.bindings != current.bindings;
        if !changed {
            return Ok(CommitEffects {
                notice: None,
                network_id: network,
                revision: current.revision,
                target_node_ids: BTreeSet::new(),
                deleted_node_ids: BTreeSet::new(),
                netmap: Default::default(),
            });
        }
        let revision = next_revision(current.revision)?;
        let canonical = canonical_json(&prepared.document.configuration)?;
        let bindings = Zeroizing::new(
            serde_json::to_vec(&prepared.bindings)
                .map_err(|_| Error::encode("configuration bindings"))?,
        );
        let source = self.sealer.seal(
            network,
            revision,
            Purpose::Source,
            prepared.document.source_jsonc.expose().as_bytes(),
        )?;
        let configuration = self.sealer.seal(
            network,
            revision,
            Purpose::Configuration,
            canonical.expose().as_bytes(),
        )?;
        let bindings = self
            .sealer
            .seal(network, revision, Purpose::Bindings, &bindings)?;
        let version_commit = store::configuration::versions::VersionCommit {
            network,
            previous_revision: current.revision,
            revision,
            actor,
            request,
            now,
        };
        store::configuration::versions::insert_version(
            tx,
            &version_commit,
            &prepared,
            [&source, &configuration, &bindings],
        )?;
        crate::resources::bindings::inherit_checks(tx, current, &prepared, revision)?;
        store::execution::apply_configuration_effects(
            tx,
            &self.sealer,
            network,
            &current.document.configuration,
            &current.bindings,
            &prepared.document.configuration,
            store::execution::now()?,
        )?;
        // Netmaps are part of the version: the same transaction stores every
        // changed Node's next serial, or rejects the version.
        let netmap = store::netmap::recompile(
            tx,
            &self.sealer,
            network,
            now,
            store::netmap::Trigger::Configuration,
        )?;
        // The version's fixed participants: every active Node's netmap
        // current after this apply, including offline Nodes.
        let execution_targets =
            store::execution::netmap_targets(tx, network, &prepared.document.configuration)?;
        let target_node_ids: BTreeSet<NodeId> = execution_targets.keys().copied().collect();
        store::configuration::versions::record_execution_targets(
            tx,
            &version_commit,
            &execution_targets,
        )?;
        Ok(CommitEffects {
            notice: Some(meshway_protocol::events::ConfigurationNotice {
                network_id: network,
                previous_revision: current.revision,
                revision,
                changed_at: chrono::DateTime::from_timestamp_millis(now)
                    .ok_or(Error::Internal("commit timestamp"))?,
                cause: prepared.cause,
                actor: event_actor,
                request_id: Some(request),
                changed_pointers: prepared
                    .changes
                    .iter()
                    .map(|change| change.path.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                changes_complete: true,
                source_changed: prepared.source_changed,
            }),
            network_id: network,
            revision,
            target_node_ids,
            deleted_node_ids: prepared.deleted_node_ids,
            netmap,
        })
    }
}

pub(super) fn next_revision(current: Revision) -> Result<Revision, Error> {
    Revision::new(
        current
            .get()
            .checked_add(1)
            .ok_or(Error::Internal("configuration revision overflow"))?,
    )
    .map_err(|_| Error::Internal("configuration revision overflow"))
}

/// The prepared document may only differ from the current one in the ways
/// the intent allows. Authoring already produced the target; this guards the
/// boundary against any drift between the two.
fn scope(
    tx: &Transaction<'_>,
    current: &Snapshot,
    intent: &Intent<'_>,
    prepared: &PreparedConfiguration,
) -> Result<(), Error> {
    let before = &current.document.configuration;
    let after = &prepared.document.configuration;
    let inventory = context::inventory_ids(tx, current.network_id)?;
    for id in after.nodes.keys() {
        if !inventory.contains(id) {
            return Err(Error::DependencyUnavailable);
        }
    }
    match intent {
        Intent::Edit { .. } => {
            for (id, node) in &after.nodes {
                let Some(previous) = before.nodes.get(id) else {
                    // Managers edit nodes; only enrollment adds one.
                    return Err(Error::PermissionDenied);
                };
                let identity_changed = previous.identity != node.identity
                    || current.bindings.node_users.get(id) != prepared.bindings.node_users.get(id);
                if identity_changed
                    && !matches!(&node.identity, NodeIdentity::Tag { tags } if !tags.is_empty())
                {
                    return Err(Error::PermissionDenied);
                }
            }
        }
        Intent::Enrollment { admission } => {
            let admitted = after
                .nodes
                .get(&admission.node)
                .ok_or(Error::PermissionDenied)?;
            if admitted.identity != admission.identity {
                return Err(Error::PermissionDenied);
            }
            let mut expected = before.clone();
            expected.nodes.insert(admission.node, admitted.clone());
            if expected != *after {
                return Err(Error::PermissionDenied);
            }
        }
        Intent::MemberRemoval { membership, .. } => {
            let mut expected = before.clone();
            expected.nodes.retain(|id, _| {
                store::execution::user_binding(before, &current.bindings, *id)
                    .is_none_or(|binding| binding.membership_id != *membership)
            });
            if expected != *after {
                return Err(Error::PermissionDenied);
            }
        }
        Intent::NodeLeave { principal } => {
            let mut expected = before.clone();
            expected.nodes.remove(&principal.node_id());
            if expected != *after {
                return Err(Error::PermissionDenied);
            }
        }
        Intent::EmailSync {
            user,
            membership,
            old_email,
            new_email,
        } => {
            let binding = UserBinding {
                user_id: *user,
                membership_id: *membership,
            };
            if before.nodes.len() != after.nodes.len() {
                return Err(Error::PermissionDenied);
            }
            for (id, node) in &after.nodes {
                let previous = before.nodes.get(id).ok_or(Error::PermissionDenied)?;
                if std::mem::discriminant(&previous.identity)
                    != std::mem::discriminant(&node.identity)
                    || store::execution::user_binding(before, &current.bindings, *id)
                        != store::execution::user_binding(after, &prepared.bindings, *id)
                {
                    return Err(Error::PermissionDenied);
                }
            }
            let mut expected = current.bindings.users.clone();
            if expected.get(*old_email) == Some(&binding)
                && let Some(moved) = expected.remove(*old_email)
            {
                expected.insert((*new_email).to_owned(), moved);
            }
            if prepared.bindings.users != expected
                || prepared.bindings.node_users != current.bindings.node_users
            {
                return Err(Error::PermissionDenied);
            }
        }
    }
    for id in after.nodes.keys() {
        store::execution::identity_eligible(tx, after, &prepared.bindings, current.network_id, *id)
            .map_err(|error| match error {
                // In a configuration commit an ineligible identity is a
                // dependency of the prepared document, not a caller credential.
                Error::InvalidCredentials => Error::DependencyUnavailable,
                other => other,
            })?;
    }
    Ok(())
}
