use crate::state::{
    StateError,
    service::{ReadIdentity, Runtime},
    stream::CommittedChange,
};
use chrono::{DateTime, Utc};
use meshway_configuration::RuleSet;
use meshway_model::{NetworkId, Revision};
use meshway_protocol::resources::{
    ArtifactSummary, ResourceProblem, RuleSetData, RuleSetList, RuleSetView,
};
use rusqlite::{Transaction, params};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub struct RuleSetScope {
    pub network: NetworkId,
    pub revision: Revision,
    pub can_refresh: bool,
    pub definitions: BTreeMap<String, RuleSet>,
    pub views: BTreeMap<String, RuleSetView>,
}
pub trait ResourceProvider: Send + Sync + 'static {
    fn rule_sets(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<RuleSetScope, StateError>;
    /// Rechecks source definition/revision and atomically binds the immutable
    /// artifact to an execution refresh. Never changes a historical artifact.
    fn bind_rule_set(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        revision: Revision,
        key: &str,
        artifact: &ArtifactSummary,
        now: DateTime<Utc>,
    ) -> Result<(), StateError>;
}
static DOWNLOADS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
pub struct ResourceService<P: ResourceProvider> {
    handle: crate::store::Handle,
    provider: Arc<P>,
    runtime: Arc<Mutex<Runtime>>,
    connector: Arc<dyn super::download::Connector>,
}
impl<P: ResourceProvider> Clone for ResourceService<P> {
    fn clone(&self) -> Self {
        Self {
            handle: self.handle.clone(),
            provider: self.provider.clone(),
            runtime: self.runtime.clone(),
            connector: self.connector.clone(),
        }
    }
}
impl<P: ResourceProvider> ResourceService<P> {
    pub(crate) fn new(
        handle: crate::store::Handle,
        provider: P,
        runtime: Arc<Mutex<Runtime>>,
    ) -> Self {
        Self {
            handle,
            provider: Arc::new(provider),
            runtime,
            connector: Arc::new(super::download::Https),
        }
    }
    #[cfg(test)]
    pub(crate) fn with_connector(mut self, connector: Arc<dyn super::download::Connector>) -> Self {
        self.connector = connector;
        self
    }
    pub async fn rule_sets(
        &self,
        identity: ReadIdentity,
        selected: Option<String>,
    ) -> Result<RuleSetList, StateError> {
        let provider = self.provider.clone();
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let scope = provider.rule_sets(&tx, &identity, Utc::now())?;
                if selected
                    .as_ref()
                    .is_some_and(|key| !scope.definitions.contains_key(key))
                {
                    return Err(StateError::NotFound);
                }
                Ok(RuleSetList {
                    items: scope
                        .definitions
                        .keys()
                        .filter(|key| selected.as_ref().is_none_or(|selected| selected == *key))
                        .map(|key| {
                            let mut view =
                                scope
                                    .views
                                    .get(key)
                                    .cloned()
                                    .unwrap_or_else(|| RuleSetView {
                                        key: key.clone(),
                                        configuration_revision: scope.revision,
                                        definition: None,
                                        artifact: None,
                                        publication: None,
                                        problem: None,
                                        checked_at: None,
                                        last_success_at: None,
                                    });
                            view.definition =
                                scope.can_refresh.then(|| scope.definitions[key].clone());
                            view
                        })
                        .collect(),
                })
            })
            .await?
    }
    pub async fn refresh_rule_set(
        &self,
        identity: ReadIdentity,
        key: String,
    ) -> Result<RuleSetView, StateError> {
        let permit = DOWNLOADS.try_acquire().map_err(|_| StateError::Budget)?;
        let provider = self.provider.clone();
        let check_identity = identity.clone();
        let check_key = key.clone();
        let (network, revision, definition, previous_artifact) = self
            .handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let scope = provider.rule_sets(&tx, &check_identity, Utc::now())?;
                if !scope.can_refresh {
                    return Err(StateError::Forbidden);
                }
                let definition = scope
                    .definitions
                    .get(&check_key)
                    .cloned()
                    .ok_or(StateError::NotFound)?;
                let artifact = scope
                    .views
                    .get(&check_key)
                    .and_then(|view| view.artifact.as_ref())
                    .map(|artifact| artifact.id);
                Ok::<_, StateError>((scope.network, scope.revision, definition, artifact))
            })
            .await??;
        let converted = match &definition {
            RuleSet::Inline {
                ip_cidr,
                domain_suffix,
            } => super::normalize(RuleSetData {
                ip_cidr: ip_cidr.clone(),
                domain_suffix: domain_suffix.clone(),
            }),
            RuleSet::Remote { url, format, .. } => {
                match super::download::download(url, self.connector.as_ref()).await {
                    Ok(bytes) => super::convert(&bytes, format),
                    Err(problem) => Err(problem),
                }
            }
        };
        drop(permit);
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let scope = provider.rule_sets(&tx, &identity, now)?;
                if !scope.can_refresh {
                    return Err(StateError::Forbidden);
                }
                let current_artifact = scope.views.get(&key)
                    .and_then(|view| view.artifact.as_ref())
                    .map(|artifact| artifact.id);
                if scope.network != network
                    || scope.revision != revision
                    || scope.definitions.get(&key) != Some(&definition)
                    || current_artifact != previous_artifact
                {
                    return Err(StateError::Conflict);
                }
                let mut artifact = scope.views.get(&key).and_then(|view| view.artifact.clone());
                let problem = match converted {
                    Err(problem) => Some(problem),
                    Ok(data) => {
                        tx.execute_batch("SAVEPOINT resource_binding")?;
                        let candidate = super::store_artifact(&tx, network, "rule-set", &data, now)?;
                        match provider.bind_rule_set(&tx, network, revision, &key, &candidate, now) {
                            Ok(()) => {
                                tx.execute_batch("RELEASE resource_binding")?;
                                artifact = Some(candidate);
                                None
                            }
                            Err(StateError::Invalid) => {
                                tx.execute_batch("ROLLBACK TO resource_binding; RELEASE resource_binding")?;
                                Some(ResourceProblem::ValidationFailed)
                            }
                            Err(error) => return Err(error),
                        }
                    }
                };
                let stored_problem = problem
                    .map(|value| serde_json::to_string(&value))
                    .transpose()
                    .map_err(|_| StateError::Storage)?;
                tx.execute(
                    "INSERT INTO rule_set_checks(network_id,revision,rule_set_key,problem,checked_at,last_success_at)
                     VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(network_id,revision,rule_set_key)
                     DO UPDATE SET problem=excluded.problem,checked_at=excluded.checked_at,
                        last_success_at=COALESCE(excluded.last_success_at,rule_set_checks.last_success_at)",
                    params![
                        network.as_uuid().as_bytes().as_slice(),
                        revision.get() as i64,
                        key,
                        stored_problem,
                        now.timestamp_millis(),
                        problem.is_none().then_some(now.timestamp_millis()),
                    ],
                )?;
                let result = provider.rule_sets(&tx, &identity, now)?
                    .views.remove(&key).ok_or(StateError::Storage)?;
                if result.artifact != artifact || result.problem != problem {
                    return Err(StateError::Storage);
                }
                // Once admitted, this store closure owns COMMIT and publication
                // even if the HTTP waiter is cancelled during the download tail.
                crate::state::service::CommitPublisher::new(runtime).commit(
                    tx,
                    vec![
                        CommittedChange::Resource { network_id: network },
                        CommittedChange::Progress { network_id: network },
                    ],
                )?;
                Ok(result)
            })
            .await?
    }
}
