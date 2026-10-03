use super::*;
use crate::{accounts::token_digest, store::Handle};
use meshway_configuration::{ConfigurationView, RuleSet, UserBinding, edit_source, parse_jsonc};
use meshway_protocol::{WireSecret, configuration::*};
use rusqlite::{Connection, TransactionBehavior};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct Configurations {
    handle: Handle,
    sealer: Arc<Sealer>,
    cache: Arc<SnapshotCache>,
}
impl Configurations {
    pub(crate) fn new(handle: Handle, sealer: Arc<Sealer>, cache: Arc<SnapshotCache>) -> Self {
        Self {
            handle,
            sealer,
            cache,
        }
    }
    pub(crate) fn synchronize_email(
        &self,
        tx: &Transaction<'_>,
        user: UserId,
        old_email: &str,
        new_email: &str,
        request: RequestId,
    ) -> Result<(), Error> {
        let networks = {
            let mut stmt = tx.prepare("SELECT id FROM networks WHERE deleted_at IS NULL")?;
            stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        for network in networks {
            let network =
                NetworkId::new(uuid::Uuid::from_slice(&network).map_err(|_| Error::Storage)?)
                    .map_err(|_| Error::Storage)?;
            let current = self.current(tx, network)?;
            if !current
                .bindings
                .users
                .get(old_email)
                .is_some_and(|binding| binding.user_id == user)
            {
                continue;
            }
            let mut input = NetworkConfigurationInput::from(&current.document.configuration);
            meshway_configuration::rename_user_references(&mut input, old_email, new_email);
            UnattachedDependencies::check(tx, &current, &input)?;
            let before =
                meshway_configuration::parse_jsonc(current.document.source_jsonc.expose())?;
            let mut after = before.clone();
            meshway_configuration::rename_user_references(&mut after, old_email, new_email);
            let source = meshway_configuration::rewrite_source(
                current.document.source_jsonc.expose(),
                &before,
                &after,
            )?;
            let mut expected = current.bindings.users.clone();
            if let Some(binding) = expected.remove(old_email) {
                expected.insert(new_email.into(), binding);
            }
            let context = TrustedContext {
                validation: ValidationContext {
                    members: members(tx, network)?,
                    previous: Some(&current.document.configuration),
                    expected_user_bindings: expected,
                    previous_user_bindings: current.bindings.users.clone(),
                    ..Default::default()
                },
                rule_sets: current.bindings.rule_sets.clone(),
            };
            let mut prepared = prepare(input, source, &current, &context)?;
            prepared.email_sync = Some(EmailSync {
                user,
                old_email: old_email.into(),
                new_email: new_email.into(),
            });
            apply_in_transaction(
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
                    participant: &UnattachedDependencies,
                },
            )?;
        }
        Ok(())
    }
    pub(crate) async fn read(
        &self,
        token: &str,
        network: NetworkId,
        revision: Option<Revision>,
    ) -> Result<Arc<Snapshot>, Error> {
        let digest = token_digest(token).map_err(|_| Error::InvalidSession)?;
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
            .await
            .map_err(|_| Error::Storage)?
    }
    pub(crate) async fn validate(
        &self,
        token: &str,
        network: NetworkId,
        request: ConfigurationRequest,
        id: RequestId,
    ) -> Result<ConfigurationPreviewResponse, Error> {
        let digest = token_digest(token).map_err(|_| Error::InvalidSession)?;
        let service = self.clone();
        self.handle
            .execute(move |db| {
                let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
                service.authorize(&tx, &digest, network)?;
                let current = service.current(&tx, network)?;
                let prepared = service.prepare_request(&tx, &current, &request, false)?;
                let result = ConfigurationPreviewResponse {
                    unavailable_dependencies: prepared.unavailable_dependencies,
                    request_id: id,
                    base_revision: current.revision,
                    preview: ConfigurationView::from(&prepared.document.configuration),
                    change_count: u32::try_from(prepared.changes.len())
                        .map_err(|_| Error::Storage)?,
                    changes: prepared.changes,
                    source_changed: prepared.source_changed,
                    deleted_node_ids: prepared.deleted_node_ids.into_iter().collect(),
                    address_changed_node_ids: prepared
                        .address_changed_node_ids
                        .into_iter()
                        .collect(),
                };
                tx.commit()?;
                Ok(result)
            })
            .await
            .map_err(|_| Error::Storage)?
    }
    pub(crate) async fn apply(
        &self,
        token: &str,
        network: NetworkId,
        request: ConfigurationRequest,
        id: RequestId,
    ) -> Result<ConfigurationAppliedResponse, Error> {
        let digest = token_digest(token).map_err(|_| Error::InvalidSession)?;
        let service = self.clone();
        self.handle
            .execute(move |db| service.apply_request(db, digest, network, request, id))
            .await
            .map_err(|_| Error::Storage)?
    }
    fn apply_request(
        &self,
        db: &mut Connection,
        digest: [u8; 32],
        network: NetworkId,
        request: ConfigurationRequest,
        id: RequestId,
    ) -> Result<ConfigurationAppliedResponse, Error> {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let user = self.authorize(&tx, &digest, network)?;
        let encoded = Zeroizing::new(serde_json::to_vec(&request).map_err(|_| Error::Storage)?);
        let prior = tx.query_row(
            "SELECT base_revision,input,result FROM configuration_commits WHERE network_id=?1 AND user_id=?2 AND request_id=?3",
            params![network.as_uuid().as_bytes().as_slice(), user.as_uuid().as_bytes().as_slice(), request.request_id().as_uuid().as_bytes().as_slice()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, String>(2)?)),
        ).optional()?;
        if let Some((base, input, result)) = prior {
            let base = Revision::new(u64::try_from(base).map_err(|_| Error::Storage)?)
                .map_err(|_| Error::Storage)?;
            let original = self
                .sealer
                .open(network, base, Purpose::Request, &input)
                .map_err(|_| Error::Storage)?;
            if original.as_bytes() != encoded.as_slice() {
                return Err(Error::Conflict);
            }
            let mut result: ConfigurationAppliedResponse =
                serde_json::from_str(&result).map_err(|_| Error::Storage)?;
            result.request_id = id;
            tx.commit()?;
            return Ok(result);
        }
        let current = self.current(&tx, network)?;
        let prepared = self.prepare_request(&tx, &current, &request, true)?;
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
                participant: &UnattachedDependencies,
            },
        )?;
        let result = ConfigurationAppliedResponse {
            request_id: id,
            network_id: effects.network_id,
            revision: effects.revision,
            publication_ids: effects.publication_ids,
            target_count: u32::try_from(effects.target_node_ids.len())
                .map_err(|_| Error::Storage)?,
            deleted_node_ids: effects.deleted_node_ids.into_iter().collect(),
        };
        let input = self
            .sealer
            .seal(network, current.revision, Purpose::Request, &encoded)
            .map_err(|_| Error::Storage)?;
        tx.execute(
            "INSERT INTO configuration_commits(network_id,user_id,request_id,base_revision,input,result) VALUES(?1,?2,?3,?4,?5,?6)",
            params![network.as_uuid().as_bytes().as_slice(), user.as_uuid().as_bytes().as_slice(),
                request.request_id().as_uuid().as_bytes().as_slice(), current.revision.get() as i64,
                input.as_bytes(), serde_json::to_string(&result).map_err(|_|Error::Storage)?],
        )?;
        tx.commit()?;
        Ok(result)
    }
    pub(crate) async fn compare(
        &self,
        token: &str,
        network: NetworkId,
        request: CompareConfigurationRequest,
        id: RequestId,
    ) -> Result<ConfigurationComparison, Error> {
        let digest = token_digest(token).map_err(|_| Error::InvalidSession)?;
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
            .await
            .map_err(|_| Error::Storage)?
    }
    fn authorize(
        &self,
        tx: &Transaction<'_>,
        digest: &[u8; 32],
        network: NetworkId,
    ) -> Result<UserId, Error> {
        let user = current_session(tx, digest, chrono::Utc::now().timestamp_millis())
            .map_err(|_| Error::InvalidSession)?
            .user
            .id;
        authorize_manager(tx, network, user)?;
        Ok(user)
    }
    fn current(&self, tx: &Transaction<'_>, network: NetworkId) -> Result<Arc<Snapshot>, Error> {
        self.cache
            .load(tx, network, current_revision(tx, network)?, &self.sealer)
    }
    fn prepare_request(
        &self,
        tx: &Transaction<'_>,
        current: &Snapshot,
        request: &ConfigurationRequest,
        generate_materials: bool,
    ) -> Result<PreparedConfiguration, Error> {
        if request.expected_revision() != current.revision {
            return Err(Error::Conflict);
        }
        let (source, mut input, expected) = match request {
            ConfigurationRequest::Document { source_jsonc, .. } => (
                source_jsonc.expose().to_owned(),
                parse_jsonc(source_jsonc.expose())?,
                current.bindings.users.clone(),
            ),
            ConfigurationRequest::Operations { ops, .. } => {
                let (source, input) = edit_source(current.document.source_jsonc.expose(), ops)?;
                (source, input, current.bindings.users.clone())
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
                meshway_configuration::rebind_user_references(&mut input, &names);
                let before = parse_jsonc(snapshot.document.source_jsonc.expose())?;
                let mut after = before.clone();
                meshway_configuration::rebind_user_references(&mut after, &names);
                let source = meshway_configuration::rewrite_source(
                    snapshot.document.source_jsonc.expose(),
                    &before,
                    &after,
                )?;
                (source, input, bindings)
            }
        };
        // There is no production Node/resource adapter in this work package's
        // baseline. Never construct an empty inventory for an existing target.
        UnattachedDependencies::check(tx, current, &input)?;
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
        let context = TrustedContext {
            validation: ValidationContext {
                members,
                previous: Some(&current.document.configuration),
                expected_user_bindings: expected,
                previous_user_bindings: current.bindings.users.clone(),
                verified_listeners,
                recreated_listeners,
                ..Default::default()
            },
            rule_sets: BTreeMap::new(),
        };
        let mut prepared = prepare(input, source, current, &context)?;
        if let ConfigurationRequest::Restore { restore_from, .. } = request {
            prepared.rollback_from = Some(*restore_from);
        }
        Ok(prepared)
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
    Revision::new(u64::try_from(r).map_err(|_| Error::Storage)?).map_err(|_| Error::Storage)
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
                    user_id: UserId::new(uuid::Uuid::from_slice(&u).map_err(|_| Error::Storage)?)
                        .map_err(|_| Error::Storage)?,
                    membership_id: meshway_model::MembershipId::new(
                        uuid::Uuid::from_slice(&m).map_err(|_| Error::Storage)?,
                    )
                    .map_err(|_| Error::Storage)?,
                },
            ))
        })
        .collect()
}

struct UnattachedDependencies;
impl UnattachedDependencies {
    fn check(
        tx: &Transaction<'_>,
        current: &Snapshot,
        input: &NetworkConfigurationInput,
    ) -> Result<(), Error> {
        if !current.document.configuration.nodes.is_empty()
            || !input.nodes.is_empty()
            || input
                .rule_sets
                .values()
                .any(|s| matches!(s, RuleSet::Remote { .. }))
        {
            return Err(Error::DependencyUnavailable);
        }
        // Once B installs its inventory schema the explicit integration is
        // required even for apparently empty snapshots; do not guess its state.
        let installed = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='nodes')",
            [],
            |r| r.get::<_, bool>(0),
        )?;
        if installed {
            return Err(Error::DependencyUnavailable);
        }
        Ok(())
    }
}
impl CommitParticipant for UnattachedDependencies {
    fn revalidate(
        &self,
        tx: &Transaction<'_>,
        principal: &Principal,
        current: &Snapshot,
        prepared: &PreparedConfiguration,
    ) -> Result<(), Error> {
        match principal {
            Principal::Session(_) => {}
            Principal::Lifecycle(user) => {
                let EmailSync {
                    user: proof_user,
                    old_email,
                    new_email,
                } = prepared
                    .email_sync
                    .as_ref()
                    .ok_or(Error::DependencyUnavailable)?;
                if proof_user != user {
                    return Err(Error::PermissionDenied);
                }
                if !current
                    .bindings
                    .users
                    .get(old_email)
                    .is_some_and(|b| b.user_id == *user)
                {
                    return Err(Error::PermissionDenied);
                }
                let valid = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND email=?2 AND disabled_at IS NULL)",
                    params![user.as_uuid().as_bytes().as_slice(), new_email],
                    |row| row.get::<_, bool>(0),
                )?;
                if !valid {
                    return Err(Error::PermissionDenied);
                }
                let mut expected = NetworkConfigurationInput::from(&current.document.configuration);
                meshway_configuration::rename_user_references(&mut expected, old_email, new_email);
                // Re-normalization may sort reference sets, but no independent
                // business edit is authorized by this system principal.
                let context = ValidationContext {
                    members: members(tx, current.network_id)?,
                    previous: Some(&current.document.configuration),
                    expected_user_bindings: prepared.bindings.users.clone(),
                    previous_user_bindings: current.bindings.users.clone(),
                    ..Default::default()
                };
                if normalize(expected, &context)?.0 != prepared.document.configuration {
                    return Err(Error::PermissionDenied);
                }
            }
        }
        Self::check(
            tx,
            current,
            &NetworkConfigurationInput::from(&prepared.document.configuration),
        )
    }
    fn apply(
        &self,
        _: &Transaction<'_>,
        _: &Snapshot,
        prepared: &PreparedConfiguration,
        _: Revision,
    ) -> Result<(Vec<PublicationId>, BTreeSet<NodeId>), Error> {
        if !prepared.document.configuration.nodes.is_empty() {
            return Err(Error::DependencyUnavailable);
        }
        Ok((Vec::new(), BTreeSet::new()))
    }
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
            Err(Error::Storage)
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
