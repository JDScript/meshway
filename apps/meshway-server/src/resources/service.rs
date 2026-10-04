use crate::effect::Effect;
use crate::error::Error;
use crate::state::service::ReadIdentity;
use chrono::{DateTime, Utc};
use meshway_configuration::RuleSet;
use meshway_model::{NetworkId, Revision};
use meshway_protocol::resources::{ArtifactSummary, ResourceProblem, RuleSetList, RuleSetView};
use rusqlite::{Transaction, TransactionBehavior, params};
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct RuleSetScope {
    pub network: NetworkId,
    pub revision: Revision,
    pub can_refresh: bool,
    pub definitions: BTreeMap<String, RuleSet>,
    pub views: BTreeMap<String, RuleSetView>,
    /// Durable attempt CAS, independent of timestamp precision and artifact equality.
    pub check_versions: BTreeMap<String, i64>,
}
pub trait ResourceProvider: Send + Sync + 'static {
    /// Internal scheduler authority is network existence, never a user token.
    fn system_rule_sets(
        &self,
        _tx: &Transaction<'_>,
        _network: NetworkId,
        _now: DateTime<Utc>,
    ) -> Result<RuleSetScope, Error> {
        Err(Error::PermissionDenied)
    }
    fn rule_sets(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<RuleSetScope, Error>;
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
    ) -> Result<(), Error>;
}
#[derive(Clone)]
pub(crate) struct SystemTask {
    pub network: NetworkId,
    pub revision: Revision,
    pub key: String,
    pub definition: RuleSet,
    pub artifact: Option<meshway_model::ArtifactId>,
    pub checked_at: Option<DateTime<Utc>>,
    pub check_version: i64,
}
static REFRESHES: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);
#[derive(Clone)]
enum Authority {
    User(ReadIdentity),
    System(NetworkId),
}
impl Authority {
    fn scope<P: ResourceProvider>(
        &self,
        provider: &P,
        tx: &Transaction<'_>,
        now: DateTime<Utc>,
    ) -> Result<RuleSetScope, Error> {
        match self {
            Self::User(identity) => provider.rule_sets(tx, identity, now),
            Self::System(network) => provider.system_rule_sets(tx, *network, now),
        }
    }
}
pub struct ResourceService<P: ResourceProvider> {
    handle: crate::store::Handle,
    provider: Arc<P>,
    downloads: Arc<super::downloads::Downloads>,
}
impl<P: ResourceProvider> Clone for ResourceService<P> {
    fn clone(&self) -> Self {
        Self {
            handle: self.handle.clone(),
            provider: self.provider.clone(),
            downloads: self.downloads.clone(),
        }
    }
}
impl<P: ResourceProvider> ResourceService<P> {
    pub(crate) fn new(
        handle: crate::store::Handle,
        provider: P,
        downloads: Arc<super::downloads::Downloads>,
    ) -> Self {
        Self {
            handle,
            provider: Arc::new(provider),
            downloads,
        }
    }
    #[cfg(test)]
    pub(crate) fn with_connector(mut self, connector: Arc<dyn super::download::Connector>) -> Self {
        self.downloads = super::downloads::Downloads::with_connector(connector);
        self
    }
    pub async fn rule_sets(
        &self,
        identity: ReadIdentity,
        selected: Option<String>,
    ) -> Result<RuleSetList, Error> {
        let provider = self.provider.clone();
        self.handle
            .read(move |tx| {
                let scope = provider.rule_sets(tx, &identity, Utc::now())?;
                if selected
                    .as_ref()
                    .is_some_and(|key| !scope.definitions.contains_key(key))
                {
                    return Err(Error::NotFound);
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
            .await
    }
    pub async fn refresh_rule_set(
        &self,
        identity: ReadIdentity,
        key: String,
    ) -> Result<RuleSetView, Error> {
        self.refresh(Authority::User(identity), key, None).await
    }
    pub(crate) async fn refresh_system(&self, task: SystemTask) -> Result<RuleSetView, Error> {
        self.refresh(
            Authority::System(task.network),
            task.key.clone(),
            Some(task),
        )
        .await
    }
    async fn refresh(
        &self,
        identity: Authority,
        key: String,
        expected: Option<SystemTask>,
    ) -> Result<RuleSetView, Error> {
        let permit = REFRESHES.try_acquire().map_err(|_| Error::StateLimit)?;
        let provider = self.provider.clone();
        let check_identity = identity.clone();
        let check_key = key.clone();
        let (network, revision, definition, previous_artifact, previous_check_version) = self
            .handle
            .read(move |tx| {
                let scope = check_identity.scope(provider.as_ref(), tx, Utc::now())?;
                if !scope.can_refresh {
                    return Err(Error::PermissionDenied);
                }
                let definition = scope
                    .definitions
                    .get(&check_key)
                    .cloned()
                    .ok_or(Error::NotFound)?;
                let artifact = scope
                    .views
                    .get(&check_key)
                    .and_then(|view| view.artifact.as_ref())
                    .map(|artifact| artifact.id);
                let check_version = *scope.check_versions.get(&check_key).unwrap_or(&0);
                if expected.is_some_and(|task| {
                    task.revision != scope.revision
                        || task.definition != definition
                        || task.artifact != artifact
                        || task.check_version != check_version
                }) {
                    return Err(Error::StateConflict);
                }
                Ok::<_, Error>((
                    scope.network,
                    scope.revision,
                    definition,
                    artifact,
                    check_version,
                ))
            })
            .await?;
        let converted = match self.downloads.fetch(&definition).await {
            Ok(data) => Ok(data),
            Err(super::downloads::FetchError::Resource(problem)) => Err(problem),
            Err(super::downloads::FetchError::Cancelled) => return Err(Error::CLOSED),
        };
        let provider = self.provider.clone();
        self.handle
            .transact(TransactionBehavior::Deferred, move |tx| {
                let _permit = permit;
                let now = Utc::now();
                let scope = identity.scope(provider.as_ref(), tx, now)?;
                if !scope.can_refresh {
                    return Err(Error::PermissionDenied);
                }
                let current_artifact = scope.views.get(&key)
                    .and_then(|view| view.artifact.as_ref())
                    .map(|artifact| artifact.id);
                if scope.network != network
                    || scope.revision != revision
                    || scope.definitions.get(&key) != Some(&definition)
                    || current_artifact != previous_artifact
                    || *scope.check_versions.get(&key).unwrap_or(&0) != previous_check_version
                {
                    return Err(Error::StateConflict);
                }
                let mut artifact = scope.views.get(&key).and_then(|view| view.artifact.clone());
                let problem = match converted {
                    Err(problem) => Some(problem),
                    Ok(data) => {
                        tx.execute_batch("SAVEPOINT resource_binding")?;
                        let candidate = super::store_artifact(tx, network, "rule-set", &data, now)?;
                        match provider.bind_rule_set(tx, network, revision, &key, &candidate, now) {
                            Ok(()) => {
                                tx.execute_batch("RELEASE resource_binding")?;
                                artifact = Some(candidate);
                                None
                            }
                            Err(Error::InvalidRequest | Error::InvalidConfiguration(_)) => {
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
                    .map_err(|_| Error::encode("rule set check problem"))?;
                tx.execute(
                    "INSERT INTO rule_set_checks(network_id,revision,rule_set_key,problem,checked_at,last_success_at,failures)
                     VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(network_id,revision,rule_set_key)
                     DO UPDATE SET problem=excluded.problem,checked_at=excluded.checked_at,
                        last_success_at=COALESCE(excluded.last_success_at,rule_set_checks.last_success_at),
                        failures=CASE WHEN excluded.problem IS NULL THEN 0 ELSE min(rule_set_checks.failures+1,16) END,
                        check_version=rule_set_checks.check_version+1",
                    params![
                        network.as_uuid().as_bytes().as_slice(),
                        revision.get() as i64,
                        key,
                        stored_problem,
                        now.timestamp_millis(),
                        problem.is_none().then_some(now.timestamp_millis()),
                        i64::from(problem.is_some()),
                    ],
                )?;
                let result = identity.scope(provider.as_ref(), tx, now)?
                    .views.remove(&key).ok_or(Error::Internal("bound rule set view missing"))?;
                if result.artifact != artifact || result.problem != problem {
                    return Err(Error::Internal("bound rule set view mismatch"));
                }
                // Once admitted, the executor owns COMMIT and publication
                // even if the HTTP waiter is cancelled during the download tail.
                Ok((
                    result,
                    vec![
                        Effect::Resource { network_id: network },
                        Effect::Progress { network_id: network },
                    ],
                ))
            })
            .await
    }
}
