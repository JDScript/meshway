use crate::effect::Effect;
use crate::error::Error;
use crate::state::service::ReadIdentity;
use crate::store;
use chrono::{DateTime, Utc};
use meshway_configuration::RuleSet;
use meshway_model::{NetworkId, RequestId, Revision, UserId};
use meshway_protocol::resources::{ArtifactSummary, ResourceProblem, RuleSetList, RuleSetView};
use rusqlite::{Transaction, TransactionBehavior};
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct RuleSetScope {
    pub network: NetworkId,
    pub revision: Revision,
    /// The session user behind a user view; system views have none.
    pub user: Option<UserId>,
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
    /// Returns the effects of the binding (netmap wake-ups).
    fn bind_rule_set(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        revision: Revision,
        key: &str,
        artifact: &ArtifactSummary,
        now: DateTime<Utc>,
    ) -> Result<Vec<Effect>, Error>;
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
enum Refresh {
    Replayed(RuleSetView),
    Perform(
        (
            NetworkId,
            Revision,
            RuleSet,
            Option<meshway_model::ArtifactId>,
            i64,
        ),
    ),
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
    /// A manual refresh is idempotent per request ID: the same user repeating
    /// it for the same key gets the current view without another download;
    /// the same ID for another key is a request ID conflict.
    pub async fn refresh_rule_set(
        &self,
        identity: ReadIdentity,
        key: String,
        request: RequestId,
    ) -> Result<RuleSetView, Error> {
        self.refresh(Authority::User(identity), key, None, Some(request))
            .await
    }
    pub(crate) async fn refresh_system(&self, task: SystemTask) -> Result<RuleSetView, Error> {
        self.refresh(
            Authority::System(task.network),
            task.key.clone(),
            Some(task),
            None,
        )
        .await
    }
    async fn refresh(
        &self,
        identity: Authority,
        key: String,
        expected: Option<SystemTask>,
        request: Option<RequestId>,
    ) -> Result<RuleSetView, Error> {
        let permit = REFRESHES.try_acquire().map_err(|_| Error::StateLimit)?;
        let provider = self.provider.clone();
        let check_identity = identity.clone();
        let check_key = key.clone();
        let (network, revision, definition, previous_artifact, previous_check_version) = match self
            .handle
            .read(move |tx| {
                let now = Utc::now();
                let mut scope = check_identity.scope(provider.as_ref(), tx, now)?;
                if !scope.can_refresh {
                    return Err(Error::PermissionDenied);
                }
                if let (Some(request), Some(user)) = (request, scope.user) {
                    match store::resources::checks::refresh_request(
                        tx,
                        scope.network,
                        user,
                        request,
                        now,
                    )? {
                        Some(earlier) if earlier == check_key => {
                            let view = scope.views.remove(&check_key).ok_or(Error::NotFound)?;
                            return Ok(Refresh::Replayed(view));
                        }
                        Some(_) => return Err(Error::RequestConflict),
                        None => {}
                    }
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
                Ok::<_, Error>(Refresh::Perform((
                    scope.network,
                    scope.revision,
                    definition,
                    artifact,
                    check_version,
                )))
            })
            .await?
        {
            Refresh::Replayed(view) => return Ok(view),
            Refresh::Perform(checked) => checked,
        };
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
                let current_artifact = scope
                    .views
                    .get(&key)
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
                if let (Some(request), Some(user)) = (request, scope.user) {
                    // A concurrent duplicate finished first: its result is ours.
                    match store::resources::checks::refresh_request(
                        tx, network, user, request, now,
                    )? {
                        Some(earlier) if earlier == key => {
                            let view = scope.views.get(&key).cloned().ok_or(Error::NotFound)?;
                            return Ok((view, Vec::new()));
                        }
                        Some(_) => return Err(Error::RequestConflict),
                        None => {
                            store::resources::checks::record_refresh_request(
                                tx, network, user, request, &key, now,
                            )?;
                        }
                    }
                }
                let mut artifact = scope.views.get(&key).and_then(|view| view.artifact.clone());
                let mut bound = Vec::new();
                let problem = match converted {
                    Err(problem) => Some(problem),
                    Ok(data) => {
                        store::resources::bindings::begin(tx)?;
                        let candidate = super::store_artifact(tx, network, "rule-set", &data, now)?;
                        match provider.bind_rule_set(tx, network, revision, &key, &candidate, now) {
                            Ok(effects) => {
                                store::resources::bindings::release(tx)?;
                                artifact = Some(candidate);
                                bound = effects;
                                None
                            }
                            Err(Error::InvalidRequest | Error::InvalidConfiguration(_)) => {
                                store::resources::bindings::rollback(tx)?;
                                Some(ResourceProblem::ValidationFailed)
                            }
                            Err(error) => return Err(error),
                        }
                    }
                };
                store::resources::checks::record(tx, network, revision, &key, problem, now)?;
                let result = identity
                    .scope(provider.as_ref(), tx, now)?
                    .views
                    .remove(&key)
                    .ok_or(Error::Internal("bound rule set view missing"))?;
                if result.artifact != artifact || result.problem != problem {
                    return Err(Error::Internal("bound rule set view mismatch"));
                }
                // Once admitted, the executor owns COMMIT and publication
                // even if the HTTP waiter is cancelled during the download tail.
                let mut effects = vec![
                    Effect::Resource {
                        network_id: network,
                    },
                    Effect::Progress {
                        network_id: network,
                    },
                ];
                effects.extend(bound);
                Ok((result, effects))
            })
            .await
    }
}
