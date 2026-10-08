use super::{Committer, Intent, binding_changes, context, resources};
use crate::configuration::{Snapshot, SnapshotCache};
use crate::effect::Effect;
use crate::error::Error;
use crate::secrets::{Purpose, Sealer};
use crate::store::authorization::{authorize_manager, current_session};
use crate::store::configuration::{commits, versions};
use crate::{accounts::token_digest, store::Handle};
use meshway_configuration::{configuration_changes, edit_source, parse_jsonc};
use meshway_model::{NetworkId, RequestId, Revision, UserId};
use meshway_policy::compile::CompileError;
use meshway_protocol::{WireSecret, configuration::*};
use rusqlite::Transaction;
use rusqlite::TransactionBehavior;
use std::sync::Arc;
use zeroize::Zeroizing;

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

/// Management use cases over the configuration owner: read, validate, apply
/// (idempotent per request), compare.
#[derive(Clone)]
pub(crate) struct Configurations {
    handle: Handle,
    committer: Committer,
    downloads: Arc<crate::resources::downloads::Downloads>,
}
impl Configurations {
    pub(crate) fn new(
        handle: Handle,
        committer: Committer,
        downloads: Arc<crate::resources::downloads::Downloads>,
    ) -> Self {
        Self {
            handle,
            committer,
            downloads,
        }
    }
    pub(crate) fn committer(&self) -> Committer {
        self.committer.clone()
    }
    fn sealer(&self) -> &Sealer {
        &self.committer.sealer
    }
    fn cache(&self) -> &SnapshotCache {
        &self.committer.cache
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
            .read(move |tx| {
                service.authorize(tx, &digest, network)?;
                let revision = match revision {
                    Some(r) => r,
                    None => versions::current_revision(tx, network)?,
                };
                let snapshot = service
                    .cache()
                    .load(tx, network, revision, service.sealer())?;
                Ok(snapshot)
            })
            .await
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
            .read(move |tx| {
                let _permit = permit;
                service.committer.preview(
                    tx,
                    network,
                    Intent::Edit {
                        session: digest,
                        request: &request,
                        resources: resources.as_ref(),
                    },
                    id,
                )
            })
            .await
    }
    /// Plans one source/target over the current version or a draft. A draft
    /// goes through the same preparation as validate, including remote rule-set
    /// downloads, and is evaluated as a change to the current version.
    pub(crate) async fn evaluate(
        &self,
        token: &str,
        network: NetworkId,
        request: EvaluateConfigurationRequest,
        id: RequestId,
    ) -> Result<EvaluationResponse, Error> {
        let digest = token_digest(token)?;
        let destination = match &request.target {
            EvaluationTarget::Node { node_id } => {
                meshway_policy::compile::PlanDestination::Node(*node_id)
            }
            EvaluationTarget::Address { address } => {
                meshway_policy::compile::PlanDestination::Address(
                    address.parse().map_err(|_| Error::InvalidRequest)?,
                )
            }
            EvaluationTarget::Domain { name } => {
                if name.len() > meshway_configuration::MAX_DNS_NAME_INPUT_BYTES {
                    return Err(Error::InvalidRequest);
                }
                meshway_policy::compile::PlanDestination::Domain(name.clone())
            }
            EvaluationTarget::Internet => meshway_policy::compile::PlanDestination::Internet,
        };
        if request
            .profile
            .as_ref()
            .is_some_and(|p| p.is_empty() || p.len() > 128)
        {
            return Err(Error::InvalidRequest);
        }
        let draft = match request.configuration {
            EvaluationConfiguration::Current => None,
            EvaluationConfiguration::Document {
                expected_revision,
                source_jsonc,
            } => Some(ConfigurationRequest::Document {
                expected_revision,
                request_id: id,
                source_jsonc,
            }),
            EvaluationConfiguration::Operations {
                expected_revision,
                ops,
            } => Some(ConfigurationRequest::Operations {
                expected_revision,
                request_id: id,
                ops,
            }),
        };
        let permit = PREPARATIONS
            .try_acquire()
            .map_err(|_| Error::DependencyUnavailable)?;
        let prepared = match draft {
            Some(draft) => Some(
                self.prepare_resources(digest, network, draft, false)
                    .await?,
            ),
            None => None,
        };
        let service = self.clone();
        let (source, profile) = (request.source, request.profile);
        self.handle
            .read(move |tx| {
                let _permit = permit;
                service.authorize(tx, &digest, network)?;
                let (revision, cfg, bindings, rule_sets, is_draft) = match prepared {
                    None => {
                        let current = service.committer.current(tx, network)?;
                        let selected = crate::resources::bindings::at_revision(
                            tx,
                            network,
                            current.revision,
                            &current.bindings.rule_sets,
                        )?;
                        let rule_sets = crate::resources::bindings::compilation_contents(
                            tx, network, &selected,
                        )?;
                        (
                            current.revision,
                            current.document.configuration.clone(),
                            current.bindings.clone(),
                            rule_sets,
                            false,
                        )
                    }
                    Some((draft, resources)) => {
                        let (current, evaluation) = service.committer.draft(
                            tx,
                            network,
                            Intent::Edit {
                                session: digest,
                                request: &draft,
                                resources: resources.as_ref(),
                            },
                        )?;
                        let rule_sets = evaluation
                            .rule_set_contents
                            .into_iter()
                            .filter_map(|(key, data)| {
                                let artifact = *evaluation.prepared.bindings.rule_sets.get(&key)?;
                                Some((
                                    key,
                                    meshway_configuration::execution::RuleSetContent {
                                        network_id: network,
                                        artifact,
                                        ip_cidr: data.ip_cidr,
                                        domain_suffix: data.domain_suffix,
                                        domain: data.domain,
                                    },
                                ))
                            })
                            .collect();
                        (
                            current.revision,
                            evaluation.prepared.document.configuration,
                            evaluation.prepared.bindings,
                            rule_sets,
                            true,
                        )
                    }
                };
                let context =
                    crate::store::execution::compilation_context(tx, network, &cfg, &bindings)?;
                let exit_inputs = crate::state::exit::inputs(
                    tx,
                    network,
                    revision,
                    chrono::Utc::now().timestamp_millis(),
                )?;
                let evaluation = meshway_policy::compile::plan_with_exit(
                    meshway_policy::compile::PlanRequest {
                        topology: meshway_policy::compile::Topology {
                            configuration: &cfg,
                            subjects: &context.subjects,
                            inventory: &context.inventory,
                        },
                        rule_sets: &rule_sets,
                        selected_profiles: &context.selected_profiles,
                        source,
                        destination,
                        profile: profile.as_deref(),
                    },
                    &exit_inputs,
                    chrono::Utc::now().timestamp_millis() as u64,
                )
                .map_err(|error| match error {
                    CompileError::InvalidInput => Error::InvalidRequest,
                    CompileError::Budget => Error::StateLimit,
                    error @ CompileError::EdgeTransportUnavailable { .. } => Error::from(error),
                    _ => Error::DependencyUnavailable,
                })?;
                Ok(EvaluationResponse {
                    request_id: id,
                    revision,
                    draft: is_draft,
                    evaluation,
                })
            })
            .await
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
            .transact(TransactionBehavior::Immediate, move |tx| {
                let resources = service.stage_resources(tx, digest, network, &request, true)?;
                if let Some(resources) = resources.filter(|r| r.needs_download()) {
                    Ok((
                        ApplyStage::Download {
                            request,
                            resources,
                            permit,
                        },
                        Vec::new(),
                    ))
                } else {
                    // No network preparation: retain the original single accepted
                    // write-job cancellation semantics for ordinary configuration.
                    let _permit = permit;
                    service
                        .apply_request(tx, digest, network, request, id, None)
                        .map(|(result, effects)| (ApplyStage::Complete(result), effects))
                }
            })
            .await?;
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
            .transact(TransactionBehavior::Immediate, move |tx| {
                let _permit = permit;
                service.apply_request(tx, digest, network, request, id, Some(&resources))
            })
            .await
    }
    fn apply_request(
        &self,
        tx: &Transaction<'_>,
        digest: [u8; 32],
        network: NetworkId,
        request: ConfigurationRequest,
        id: RequestId,
        resources: Option<&resources::Preparation>,
    ) -> Result<(ConfigurationAppliedResponse, Vec<Effect>), Error> {
        let user = self.authorize(tx, &digest, network)?;
        let encoded = Zeroizing::new(
            serde_json::to_vec(&request).map_err(|_| Error::encode("configuration request"))?,
        );
        let prior = commits::find(tx, self.sealer(), network, user, request.request_id())?;
        if let Some(prior) = prior {
            if prior.original.as_bytes() != encoded.as_slice() {
                return Err(Error::RequestConflict);
            }
            let mut result = prior.result()?;
            result.request_id = id;
            return Ok((result, Vec::new()));
        }
        let base = request.expected_revision();
        let effects = self.committer.commit(
            tx,
            network,
            Intent::Edit {
                session: digest,
                request: &request,
                resources,
            },
            request.request_id(),
        )?;
        if let Some(resources) = resources {
            resources.record(tx, network, effects.revision)?;
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
            .sealer()
            .seal(network, base, Purpose::Request, &encoded)?;
        commits::insert(
            tx,
            network,
            user,
            request.request_id(),
            base,
            &input,
            &result,
        )?;
        // One valid bulk deletion may exceed the replay window. The bounded
        // journal evicts old entries; it must not cap the configuration commit.
        Ok((result, changes))
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
            .read(move |tx| {
                service.authorize(tx, &digest, network)?;
                let a =
                    service
                        .cache()
                        .load(tx, network, request.from_revision, service.sealer())?;
                let b = service
                    .cache()
                    .load(tx, network, request.to_revision, service.sealer())?;
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
                Ok(result)
            })
            .await
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
            .read(move |tx| {
                let preparation = service.stage_resources(tx, digest, network, &request, retry)?;
                Ok::<_, Error>((request, preparation))
            })
            .await?;
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
        if retry && commits::exists(tx, network, user, request.request_id())? {
            return Ok(None);
        }
        let current = self.committer.current(tx, network)?;
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
        context::check_inputs(tx, &current, &input)?;
        resources::Preparation::stage(tx, &current, &input).map(Some)
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
    use crate::accounts::random_uuid;
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
        service
            .handle
            .execute(|db| {
                let tx = db.transaction()?;
                commits::testing::install_failure(&tx)?;
                tx.commit()?;
                Ok::<_, Error>(())
            })
            .await
            .unwrap()
            .unwrap();
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
                let tx = db.transaction().unwrap();
                let (versions, activities) = commits::testing::counts(&tx).unwrap();
                assert_eq!(versions, 1);
                assert_eq!(activities, 0);
                commits::testing::remove_failure(&tx).unwrap();
                tx.commit().unwrap();
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
