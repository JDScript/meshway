use super::*;
use crate::execution::configuration::{Participant, check_inputs, validation_context};
use crate::{accounts::token_digest, store::Handle};
use meshway_configuration::{UserBinding, edit_source, parse_jsonc};
use meshway_protocol::{WireSecret, configuration::*};
use rusqlite::{Connection, TransactionBehavior};
use std::sync::Arc;

// Includes downloaded inputs waiting for the final accepted SQLite job. Its
// permit moves into that job so cancelling an HTTP waiter cannot free it early.
static PREPARATIONS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
enum ApplyStage {
    Complete(ConfigurationAppliedResponse),
    Download {
        request: ConfigurationRequest,
        resources: resources::Preparation,
        permit: tokio::sync::SemaphorePermit<'static>,
    },
}

#[derive(Clone)]
pub(crate) struct Configurations {
    handle: Handle,
    sealer: Arc<Sealer>,
    cache: Arc<SnapshotCache>,
    publisher: crate::state::service::CommitPublisher,
    downloads: Arc<crate::resources::downloads::Downloads>,
}
impl Configurations {
    pub(crate) fn new(
        handle: Handle,
        sealer: Arc<Sealer>,
        cache: Arc<SnapshotCache>,
        publisher: crate::state::service::CommitPublisher,
        downloads: Arc<crate::resources::downloads::Downloads>,
    ) -> Self {
        Self {
            handle,
            sealer,
            cache,
            publisher,
            downloads,
        }
    }
    pub(crate) fn synchronize_email(
        &self,
        tx: &Transaction<'_>,
        user: UserId,
        old_email: &str,
        new_email: &str,
        request: RequestId,
    ) -> Result<Vec<CommitEffects>, Error> {
        let mut effects = Vec::new();
        let networks = {
            let mut stmt = tx.prepare("SELECT id FROM networks WHERE deleted_at IS NULL")?;
            stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        for network in networks {
            let network = NetworkId::new(
                uuid::Uuid::from_slice(&network).map_err(|_| Error::decode("networks.id"))?,
            )
            .map_err(|_| Error::decode("networks.id"))?;
            let current = self.current(tx, network)?;
            if !current
                .bindings
                .users
                .get(old_email)
                .is_some_and(|binding| binding.user_id == user)
                && !current
                    .bindings
                    .node_users
                    .values()
                    .any(|binding| binding.user_id == user)
            {
                continue;
            }
            let mut input = NetworkConfigurationInput::from(&current.document.configuration);
            rename_bound_user_references(&mut input, &current, user, old_email, new_email);
            check_inputs(tx, &current, &input)?;
            let before =
                meshway_configuration::parse_jsonc(current.document.source_jsonc.expose())?;
            let mut after = before.clone();
            rename_bound_user_references(&mut after, &current, user, old_email, new_email);
            let source = meshway_configuration::rewrite_source(
                current.document.source_jsonc.expose(),
                &before,
                &after,
            )?;
            let mut expected = current.bindings.users.clone();
            if expected
                .get(old_email)
                .is_some_and(|binding| binding.user_id == user)
                && let Some(binding) = expected.remove(old_email)
            {
                expected.insert(new_email.into(), binding);
            }
            let verified = materials::prepare_materials(
                tx,
                network,
                &mut input,
                &current,
                &self.sealer,
                false,
                &BTreeSet::new(),
            )?;
            let mut validation = validation_context(tx, &current, &input, None, false)?;
            validation.expected_user_bindings = expected;
            validation.verified_listeners = verified;
            let context = TrustedContext {
                validation,
                rule_sets: crate::resources::bindings::for_input(tx, &current, &input)?.0,
            };
            let mut prepared = prepare(input, source, &current, &context)?;
            prepared.email_sync = Some(EmailSync {
                user,
                old_email: old_email.into(),
                new_email: new_email.into(),
            });
            effects.push(apply_in_transaction(
                tx,
                &Principal::Lifecycle(user),
                current.revision,
                &current,
                &prepared,
                CommitMetadata {
                    cause: ChangeCause::EmailSync,
                    request,
                },
                CommitRuntime {
                    sealer: &self.sealer,
                    participant: &Participant {
                        sealer: &self.sealer,
                        admission: None,
                    },
                },
            )?);
        }
        Ok(effects)
    }
    pub(crate) async fn read(
        &self,
        token: &str,
        network: NetworkId,
        revision: Option<Revision>,
    ) -> Result<Arc<Snapshot>, Error> {
        let digest = token_digest(token)?;
        let service = self.clone();
        self.handle
            .execute(move |db| {
                let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
                service.authorize(&tx, &digest, network)?;
                let revision = match revision {
                    Some(r) => r,
                    None => current_revision(&tx, network)?,
                };
                let snapshot = service
                    .cache
                    .load(&tx, network, revision, &service.sealer)?;
                tx.commit()?;
                Ok(snapshot)
            })
            .await?
    }
    pub(crate) async fn validate(
        &self,
        token: &str,
        network: NetworkId,
        request: ConfigurationRequest,
        id: RequestId,
    ) -> Result<ConfigurationPreviewResponse, Error> {
        let permit = PREPARATIONS
            .try_acquire()
            .map_err(|_| Error::DependencyUnavailable)?;
        let digest = token_digest(token)?;
        let (request, resources) = self
            .prepare_resources(digest, network, request, false)
            .await?;
        let service = self.clone();
        self.handle
            .execute(move |db| {
                let _permit = permit;
                let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
                service.authorize(&tx, &digest, network)?;
                let current = service.current(&tx, network)?;
                let input =
                    service.prepare_request(&tx, &current, &request, false, resources.as_ref())?;
                let result = preview::build(&current, input, resources.as_ref(), id)?;
                tx.commit()?;
                Ok(result)
            })
            .await?
    }
    pub(crate) async fn apply(
        &self,
        token: &str,
        network: NetworkId,
        request: ConfigurationRequest,
        id: RequestId,
    ) -> Result<ConfigurationAppliedResponse, Error> {
        let permit = PREPARATIONS
            .try_acquire()
            .map_err(|_| Error::DependencyUnavailable)?;
        let digest = token_digest(token)?;
        let service = self.clone();
        let stage = self
            .handle
            .execute(move |db| {
                let resources = {
                    let tx = db.transaction()?;
                    service.stage_resources(&tx, digest, network, &request, true)?
                };
                if let Some(resources) = resources.filter(|r| r.needs_download()) {
                    Ok(ApplyStage::Download {
                        request,
                        resources,
                        permit,
                    })
                } else {
                    // No network preparation: retain the original single accepted
                    // write-job cancellation semantics for ordinary configuration.
                    let _permit = permit;
                    service
                        .apply_request(db, digest, network, request, id, None)
                        .map(ApplyStage::Complete)
                }
            })
            .await??;
        let (request, mut resources, permit) = match stage {
            ApplyStage::Complete(result) => return Ok(result),
            ApplyStage::Download {
                request,
                resources,
                permit,
            } => (request, resources, permit),
        };
        resources.download(&self.downloads).await?;
        let service = self.clone();
        self.handle
            .execute(move |db| {
                let _permit = permit;
                service.apply_request(db, digest, network, request, id, Some(&resources))
            })
            .await?
    }
    fn apply_request(
        &self,
        db: &mut Connection,
        digest: [u8; 32],
        network: NetworkId,
        request: ConfigurationRequest,
        id: RequestId,
        resources: Option<&resources::Preparation>,
    ) -> Result<ConfigurationAppliedResponse, Error> {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let user = self.authorize(&tx, &digest, network)?;
        let encoded = Zeroizing::new(
            serde_json::to_vec(&request).map_err(|_| Error::encode("configuration request"))?,
        );
        let prior = tx.query_row(
            "SELECT base_revision,input,result FROM configuration_commits WHERE network_id=?1 AND user_id=?2 AND request_id=?3",
            params![network.as_uuid().as_bytes().as_slice(), user.as_uuid().as_bytes().as_slice(), request.request_id().as_uuid().as_bytes().as_slice()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, String>(2)?)),
        ).optional()?;
        if let Some((base, input, result)) = prior {
            let base = Revision::new(
                u64::try_from(base)
                    .map_err(|_| Error::decode("configuration_commits.base_revision"))?,
            )
            .map_err(|_| Error::decode("configuration_commits.base_revision"))?;
            let original = self.sealer.open(network, base, Purpose::Request, &input)?;
            if original.as_bytes() != encoded.as_slice() {
                return Err(Error::RequestConflict);
            }
            let mut result: ConfigurationAppliedResponse = serde_json::from_str(&result)
                .map_err(|_| Error::decode("configuration_commits.result"))?;
            result.request_id = id;
            tx.commit()?;
            return Ok(result);
        }
        let current = self.current(&tx, network)?;
        let input = self.prepare_request(&tx, &current, &request, true, resources)?;
        let mut prepared = prepare(input.input, input.source, &current, &input.context)?;
        if let ConfigurationRequest::Restore { restore_from, .. } = &request {
            prepared.rollback_from = Some(*restore_from);
        }
        let cause = if matches!(request, ConfigurationRequest::Restore { .. }) {
            ChangeCause::Restore
        } else {
            ChangeCause::Edit
        };
        let effects = apply_in_transaction(
            &tx,
            &Principal::Session(digest),
            request.expected_revision(),
            &current,
            &prepared,
            CommitMetadata {
                cause,
                request: request.request_id(),
            },
            CommitRuntime {
                sealer: &self.sealer,
                participant: &Participant {
                    sealer: &self.sealer,
                    admission: None,
                },
            },
        )?;
        if let Some(resources) = resources {
            resources.record(&tx, network, effects.revision)?;
        }
        let changes = effects.changes();
        let result = ConfigurationAppliedResponse {
            request_id: id,
            network_id: effects.network_id,
            revision: effects.revision,
            publication_ids: if effects.target_node_ids.is_empty() {
                Vec::new()
            } else {
                effects.publication_ids
            },
            target_count: u32::try_from(effects.target_node_ids.len())
                .map_err(|_| Error::Internal("configuration target count overflow"))?,
            deleted_node_ids: effects.deleted_node_ids.into_iter().collect(),
        };
        let input = self
            .sealer
            .seal(network, current.revision, Purpose::Request, &encoded)?;
        tx.execute(
            "INSERT INTO configuration_commits(network_id,user_id,request_id,base_revision,input,result) VALUES(?1,?2,?3,?4,?5,?6)",
            params![network.as_uuid().as_bytes().as_slice(), user.as_uuid().as_bytes().as_slice(),
                request.request_id().as_uuid().as_bytes().as_slice(), current.revision.get() as i64,
                input.as_bytes(), serde_json::to_string(&result).map_err(|_|Error::encode("configuration commit result"))?],
        )?;
        // One valid bulk deletion may exceed the replay window. The bounded
        // journal evicts old entries; it must not cap the configuration commit.
        tx.commit()?;
        for change in changes {
            self.publisher.publish_committed(change);
        }
        Ok(result)
    }
    pub(crate) async fn compare(
        &self,
        token: &str,
        network: NetworkId,
        request: CompareConfigurationRequest,
        id: RequestId,
    ) -> Result<ConfigurationComparison, Error> {
        let digest = token_digest(token)?;
        let service = self.clone();
        self.handle
            .execute(move |db| {
                let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
                service.authorize(&tx, &digest, network)?;
                let a = service
                    .cache
                    .load(&tx, network, request.from_revision, &service.sealer)?;
                let b = service
                    .cache
                    .load(&tx, network, request.to_revision, &service.sealer)?;
                let mut changes =
                    configuration_changes(&a.document.configuration, &b.document.configuration)?;
                let configuration_changed = !changes.is_empty();
                changes.extend(binding_changes(&a.bindings, &b.bindings));
                let result = ConfigurationComparison {
                    request_id: id,
                    from_revision: a.revision,
                    to_revision: b.revision,
                    source_changed: a.document.source_jsonc != b.document.source_jsonc,
                    configuration_changed,
                    bindings_changed: a.bindings != b.bindings,
                    changes,
                };
                tx.commit()?;
                Ok(result)
            })
            .await?
    }
    fn authorize(
        &self,
        tx: &Transaction<'_>,
        digest: &[u8; 32],
        network: NetworkId,
    ) -> Result<UserId, Error> {
        let user = current_session(tx, digest, chrono::Utc::now().timestamp_millis())?
            .user
            .id;
        authorize_manager(tx, network, user)?;
        Ok(user)
    }
    fn current(&self, tx: &Transaction<'_>, network: NetworkId) -> Result<Arc<Snapshot>, Error> {
        self.cache
            .load(tx, network, current_revision(tx, network)?, &self.sealer)
    }
    fn prepare_request<'a>(
        &self,
        tx: &Transaction<'_>,
        current: &'a Snapshot,
        request: &ConfigurationRequest,
        generate_materials: bool,
        resources: Option<&resources::Preparation>,
    ) -> Result<preview::RequestInput<'a>, Error> {
        if request.expected_revision() != current.revision {
            return Err(Error::RevisionConflict);
        }
        let (source, mut input, expected, expected_nodes) = match request {
            ConfigurationRequest::Document { source_jsonc, .. } => (
                source_jsonc.expose().to_owned(),
                parse_jsonc(source_jsonc.expose())?,
                current.bindings.users.clone(),
                current.bindings.node_users.clone(),
            ),
            ConfigurationRequest::Operations { ops, .. } => {
                let (source, input) = edit_source(current.document.source_jsonc.expose(), ops)?;
                (
                    source,
                    input,
                    current.bindings.users.clone(),
                    current.bindings.node_users.clone(),
                )
            }
            ConfigurationRequest::Restore { restore_from, .. } => {
                let snapshot =
                    self.cache
                        .load(tx, current.network_id, *restore_from, &self.sealer)?;
                let members = members(tx, current.network_id)?;
                let mut names = BTreeMap::new();
                let mut bindings = BTreeMap::new();
                for (name, binding) in &snapshot.bindings.users {
                    let current_name = members
                        .iter()
                        .find(|(_, candidate)| *candidate == binding)
                        .map(|(email, _)| email)
                        .ok_or(Error::DependencyUnavailable)?;
                    names.insert(name.clone(), current_name.clone());
                    bindings.insert(current_name.clone(), binding.clone());
                }
                let mut input = NetworkConfigurationInput::from(&snapshot.document.configuration);
                let mut node_names = BTreeMap::new();
                for (id, binding) in &snapshot.bindings.node_users {
                    let name = members
                        .iter()
                        .find(|(_, candidate)| *candidate == binding)
                        .map(|(email, _)| email.clone())
                        .ok_or(Error::DependencyUnavailable)?;
                    node_names.insert(*id, name);
                }
                meshway_configuration::rebind_policy_user_references(&mut input, &names);
                meshway_configuration::rebind_node_user_references(&mut input, &node_names);
                let before = parse_jsonc(snapshot.document.source_jsonc.expose())?;
                let mut after = before.clone();
                meshway_configuration::rebind_policy_user_references(&mut after, &names);
                meshway_configuration::rebind_node_user_references(&mut after, &node_names);
                let source = meshway_configuration::rewrite_source(
                    snapshot.document.source_jsonc.expose(),
                    &before,
                    &after,
                )?;
                (
                    source,
                    input,
                    bindings,
                    snapshot.bindings.node_users.clone(),
                )
            }
        };
        // Node inventory is transaction-backed. Remote artifacts still require
        // exact prepared content and the final binding adapter, never empty sets.
        check_inputs(tx, current, &input)?;
        if let Some(resources) = resources {
            resources.revalidate(tx, current, &input)?;
        }
        let members = members(tx, current.network_id)?;
        let mut recreated_listeners = BTreeSet::new();
        if let ConfigurationRequest::Operations { ops, .. } = request {
            let mut removed = BTreeSet::new();
            for op in ops {
                if let meshway_configuration::ConfigurationOperation::SetListener {
                    node_id,
                    key,
                    listener,
                } = op
                {
                    let resource = (*node_id, key.clone());
                    if listener.is_none() {
                        removed.insert(resource);
                    } else if removed.contains(&resource) {
                        recreated_listeners.insert(resource);
                    }
                }
            }
        }
        let verified_listeners = materials::prepare_materials(
            tx,
            current.network_id,
            &mut input,
            current,
            &self.sealer,
            generate_materials,
            &recreated_listeners,
        )?;
        let mut validation = validation_context(tx, current, &input, None, true)?;
        validation.members = members;
        validation.expected_user_bindings = expected;
        validation.expected_node_user_bindings = expected_nodes;
        validation.verified_listeners = verified_listeners;
        validation.recreated_listeners = recreated_listeners;
        let mut rule_sets = if let ConfigurationRequest::Restore {
            restore_from,
            resource_publication,
            ..
        } = request
        {
            let snapshot = self
                .cache
                .load(tx, current.network_id, *restore_from, &self.sealer)?;
            let bindings = match resource_publication {
                Some(publication) => crate::resources::bindings::at_publication(
                    tx,
                    current.network_id,
                    *restore_from,
                    *publication,
                )?,
                None => snapshot.bindings.rule_sets.clone(),
            };
            validation.rule_sets =
                crate::resources::bindings::contents(tx, current.network_id, &bindings)?;
            bindings
        } else {
            crate::resources::bindings::for_input(tx, current, &input)?.0
        };
        let mut new_content = false;
        if let Some(resources) = resources {
            validation.rule_sets.extend(resources.contents.clone());
            crate::resources::bindings::check_budget(&validation.rule_sets)?;
            if generate_materials {
                resources.bind(tx, current, &mut rule_sets)?;
            } else {
                new_content = resources.preview_bindings(tx, current, &mut rule_sets)?;
            }
        }
        let context = TrustedContext {
            validation,
            rule_sets,
        };
        Ok(preview::RequestInput {
            input,
            source,
            context,
            new_content,
        })
    }
    async fn prepare_resources(
        &self,
        digest: [u8; 32],
        network: NetworkId,
        request: ConfigurationRequest,
        retry: bool,
    ) -> Result<(ConfigurationRequest, Option<resources::Preparation>), Error> {
        let service = self.clone();
        let (request, mut resources) = self
            .handle
            .execute(move |db| {
                let tx = db.transaction()?;
                let preparation = service.stage_resources(&tx, digest, network, &request, retry)?;
                Ok::<_, Error>((request, preparation))
            })
            .await??;
        if let Some(resources) = &mut resources {
            resources.download(&self.downloads).await?;
        }
        Ok((request, resources))
    }
    fn stage_resources(
        &self,
        tx: &Transaction<'_>,
        digest: [u8; 32],
        network: NetworkId,
        request: &ConfigurationRequest,
        retry: bool,
    ) -> Result<Option<resources::Preparation>, Error> {
        let user = self.authorize(tx, &digest, network)?;
        if retry && tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM configuration_commits WHERE network_id=?1 AND user_id=?2 AND request_id=?3)",
                params![network.as_uuid().as_bytes().as_slice(),user.as_uuid().as_bytes().as_slice(),request.request_id().as_uuid().as_bytes().as_slice()],
                |row| row.get::<_,bool>(0),
            )? { return Ok(None); }
        let current = self.current(tx, network)?;
        if current.revision != request.expected_revision() {
            return Err(Error::RevisionConflict);
        }
        let input = match &request {
            ConfigurationRequest::Document { source_jsonc, .. } => {
                parse_jsonc(source_jsonc.expose())?
            }
            ConfigurationRequest::Operations { ops, .. } => {
                edit_source(current.document.source_jsonc.expose(), ops)?.1
            }
            ConfigurationRequest::Restore { .. } => return Ok(None),
        };
        check_inputs(tx, &current, &input)?;
        resources::Preparation::stage(tx, &current, &input).map(Some)
    }
}

pub(crate) fn current_revision(
    tx: &Transaction<'_>,
    network: NetworkId,
) -> Result<Revision, Error> {
    let r = tx
        .query_row(
            "SELECT current_revision FROM networks WHERE id=?1 AND deleted_at IS NULL",
            [network.as_uuid().as_bytes().as_slice()],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    Revision::new(u64::try_from(r).map_err(|_| Error::decode("networks.current_revision"))?)
        .map_err(|_| Error::decode("networks.current_revision"))
}

/// Readable names follow the stable subject for each binding purpose. An email
/// reused by another account must not rename the old account's policy references.
pub(crate) fn rename_bound_user_references(
    input: &mut NetworkConfigurationInput,
    snapshot: &Snapshot,
    user: UserId,
    old: &str,
    new: &str,
) {
    if snapshot
        .bindings
        .users
        .get(old)
        .is_some_and(|binding| binding.user_id == user)
    {
        meshway_configuration::rebind_policy_user_references(
            input,
            &BTreeMap::from([(old.to_owned(), new.to_owned())]),
        );
    }
    let names = snapshot
        .bindings
        .node_users
        .iter()
        .filter(|(_, binding)| binding.user_id == user)
        .map(|(id, _)| (*id, new.to_owned()))
        .collect();
    meshway_configuration::rebind_node_user_references(input, &names);
}
pub(crate) fn members(
    tx: &Transaction<'_>,
    network: NetworkId,
) -> Result<BTreeMap<String, UserBinding>, Error> {
    let mut stmt = tx.prepare(
        "SELECT u.email,u.id,m.id FROM memberships m JOIN users u ON u.id=m.user_id WHERE m.network_id=?1 AND m.revoked_at IS NULL AND u.disabled_at IS NULL",
    )?;
    let rows = stmt
        .query_map([network.as_uuid().as_bytes().as_slice()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(email, u, m)| {
            Ok((
                email,
                UserBinding {
                    user_id: UserId::new(
                        uuid::Uuid::from_slice(&u).map_err(|_| Error::decode("users.id"))?,
                    )
                    .map_err(|_| Error::decode("users.id"))?,
                    membership_id: meshway_model::MembershipId::new(
                        uuid::Uuid::from_slice(&m).map_err(|_| Error::decode("memberships.id"))?,
                    )
                    .map_err(|_| Error::decode("memberships.id"))?,
                },
            ))
        })
        .collect()
}

pub(crate) fn source_response(snapshot: &Snapshot, id: RequestId) -> ConfigurationSourceResponse {
    ConfigurationSourceResponse {
        request_id: id,
        network_id: snapshot.network_id,
        revision: snapshot.revision,
        source_jsonc: WireSecret::new(snapshot.document.source_jsonc.expose().to_owned()),
        configuration: snapshot.document.configuration.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        App, Settings,
        accounts::{Secret, TEST_KDF_LOCK},
    };

    fn request() -> RequestId {
        RequestId::new(random_uuid().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn failed_commit_rolls_back_version_activity_and_retry_identity() {
        let _serial = TEST_KDF_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(Settings::new(
            dir.path().canonicalize().unwrap().join("data"),
        ))
        .await
        .unwrap();
        let user = app
            .accounts()
            .register(
                "atomic@example.test".into(),
                Secret::new("valid password".into()),
                request(),
            )
            .await
            .unwrap();
        let network = app
            .networks()
            .create(user.token.expose(), "atomic".into(), request())
            .await
            .unwrap()
            .id;
        let service = app.configurations();
        let current = service
            .read(user.token.expose(), network, None)
            .await
            .unwrap();
        service.handle.execute(|db| db.execute_batch("CREATE TRIGGER fail_commit BEFORE INSERT ON configuration_commits BEGIN SELECT RAISE(ABORT, 'injected failure'); END;")).await.unwrap().unwrap();
        let edit_id = request();
        let edit = || ConfigurationRequest::Document {
            expected_revision: current.revision,
            request_id: edit_id,
            source_jsonc: WireSecret::new(format!(
                "// committed once\n{}",
                current.document.source_jsonc.expose()
            )),
        };
        assert!(matches!(
            service
                .apply(user.token.expose(), network, edit(), request())
                .await,
            Err(Error::Storage(_))
        ));
        let after = service
            .read(user.token.expose(), network, None)
            .await
            .unwrap();
        assert_eq!(after.revision, current.revision);
        assert_eq!(
            after.document.source_jsonc.expose(),
            current.document.source_jsonc.expose()
        );
        service
            .handle
            .execute(|db| {
                let versions: i64 = db
                    .query_row("SELECT count(*) FROM configuration_versions", [], |r| {
                        r.get(0)
                    })
                    .unwrap();
                let activities: i64 = db
                    .query_row(
                        "SELECT count(*) FROM activity_events WHERE action='configuration.apply'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(versions, 1);
                assert_eq!(activities, 0);
                db.execute_batch("DROP TRIGGER fail_commit").unwrap();
            })
            .await
            .unwrap();
        let applied = service
            .apply(user.token.expose(), network, edit(), request())
            .await
            .unwrap();
        let repeated = service
            .apply(user.token.expose(), network, edit(), request())
            .await
            .unwrap();
        assert_eq!(applied.revision.get(), 2);
        assert_eq!(repeated.revision, applied.revision);
        app.shutdown().await.unwrap();
    }
}
