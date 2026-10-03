use crate::state::{
    StateError,
    service::{ReadIdentity, StateProvider, StateService},
    stream::CommittedChange,
};
use chrono::{DateTime, Utc};
use meshway_configuration::RuleSet;
use meshway_model::{NetworkId, Revision};
use meshway_protocol::resources::{ArtifactSummary, RuleSetData, RuleSetList, RuleSetView};
use rusqlite::Transaction;
use std::collections::BTreeMap;

pub struct RuleSetScope {
    pub network: NetworkId,
    pub revision: Revision,
    pub can_refresh: bool,
    pub definitions: BTreeMap<String, RuleSet>,
    pub views: BTreeMap<String, RuleSetView>,
}
pub trait ResourceProvider: StateProvider {
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
impl<P: ResourceProvider> StateService<P> {
    pub async fn rule_sets(
        &self,
        identity: ReadIdentity,
        selected: Option<String>,
    ) -> Result<RuleSetList, StateError> {
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
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
                let runtime = runtime.lock().map_err(|_| StateError::Unavailable)?;
                Ok(RuleSetList {
                    items: scope
                        .definitions
                        .keys()
                        .filter(|key| selected.as_ref().is_none_or(|selected| selected == *key))
                        .map(|key| {
                            let mut view = runtime
                                .resource_results
                                .get(&(scope.network, key.clone()))
                                .filter(|result| result.configuration_revision == scope.revision)
                                .or_else(|| scope.views.get(key))
                                .cloned()
                                .unwrap_or_else(|| RuleSetView {
                                    key: key.clone(),
                                    configuration_revision: scope.revision,
                                    definition: None,
                                    artifact: None,
                                    problem: None,
                                    checked_at: None,
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
        let (network, revision, definition) = self
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
                Ok::<_, StateError>((scope.network, scope.revision, definition))
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
            RuleSet::Remote { url, format, .. } => match super::download(url).await {
                Ok(bytes) => super::convert(&bytes, format),
                Err(problem) => Err(problem),
            },
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
                if scope.network != network
                    || scope.revision != revision
                    || scope.definitions.get(&key) != Some(&definition)
                {
                    return Err(StateError::Conflict);
                }
                let data = match converted {
                    Ok(data) => data,
                    Err(problem) => {
                        let result = RuleSetView {
                            key: key.clone(),
                            configuration_revision: revision,
                            definition: Some(definition.clone()),
                            artifact: scope.views.get(&key).and_then(|view| view.artifact.clone()),
                            problem: Some(problem),
                            checked_at: Some(now),
                        };
                        let mut runtime = runtime.lock().map_err(|_| StateError::Unavailable)?;
                        if runtime.resource_results.len() >= 4096 {
                            runtime.resource_results.clear();
                        }
                        runtime
                            .resource_results
                            .insert((network, key), result.clone());
                        return Ok(result);
                    }
                };
                let source = match &definition {
                    RuleSet::Inline { .. } => "inline",
                    RuleSet::Remote { url, .. } => url,
                };
                let artifact = super::store_artifact(&tx, network, source, &data, now)?;
                provider.bind_rule_set(&tx, network, revision, &key, &artifact, now)?;
                tx.commit()?;
                let mut runtime = runtime.lock().map_err(|_| StateError::Unavailable)?;
                runtime.hub.publish_committed(CommittedChange::Resource {
                    network_id: network,
                });
                let result = RuleSetView {
                    key: key.clone(),
                    configuration_revision: revision,
                    definition: Some(definition),
                    artifact: Some(artifact),
                    problem: None,
                    checked_at: Some(now),
                };
                if runtime.resource_results.len() >= 4096 {
                    runtime.resource_results.clear();
                }
                runtime
                    .resource_results
                    .insert((network, key), result.clone());
                Ok(result)
            })
            .await?
    }
}
