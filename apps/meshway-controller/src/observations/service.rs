use super::TaskAdmission;
use crate::state::{
    StateError,
    service::{ReadIdentity, Runtime},
    stream::CommittedChange,
};
use chrono::{DateTime, Utc};
use meshway_model::{ObservationId, ObservationSubscriptionId, SessionId};
use meshway_protocol::observations::*;
use rusqlite::Transaction;
use std::sync::{Arc, Mutex};

/// B computes the authorized full path candidates; A checks current session and
/// source User identity ownership. A network role cannot manufacture a path.
pub trait ObservationProvider: Send + Sync + 'static {
    fn synchronize(
        &self,
        _: &Transaction<'_>,
        _: &[SubscriptionLease],
        _: DateTime<Utc>,
    ) -> Result<(), StateError> {
        Ok(())
    }
    fn scope(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<crate::state::VisibilityScope, StateError>;
    fn report(
        &self,
        tx: &Transaction<'_>,
        authorization: &str,
        now: DateTime<Utc>,
    ) -> Result<crate::state::ReportAdmission, StateError>;
    /// May synthesize and persist the diagnostic execution demand within this
    /// transaction. C commits it together with task acceptance before publishing.
    fn prepare_observation(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<TaskAdmission, StateError> {
        self.observation(tx, identity, request, now)
    }
    /// Read-only reauthorization for already accepted tasks and subscribers.
    fn observation(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<TaskAdmission, StateError>;
    fn observation_session(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<SessionId, StateError>;
}
#[derive(PartialEq, Eq)]
pub struct SubscriptionLease {
    pub id: ObservationSubscriptionId,
    pub expires_at: DateTime<Utc>,
}

/// SQLite rollback also restores the task metadata. Shared immutable window
/// indexes avoid copying all retained deduplication evidence on each tick.
struct RuntimeTransaction<'a> {
    runtime: std::sync::MutexGuard<'a, Runtime>,
    previous: Option<super::Observations>,
}
impl<'a> RuntimeTransaction<'a> {
    fn new(runtime: std::sync::MutexGuard<'a, Runtime>) -> Self {
        let previous = Some(runtime.observations.clone());
        Self { runtime, previous }
    }
    fn committed(&mut self) {
        let Some(previous) = self.previous.take() else {
            return;
        };
        let mut changed = std::collections::BTreeSet::new();
        for (id, stored) in &self.runtime.observations.tasks {
            if previous.tasks.get(id).is_none_or(|old| {
                old.task.configuration != stored.task.configuration
                    || old.task.phase != stored.task.phase
                    || old.task.expires_at != stored.task.expires_at
                    || old.subscribers.len() != stored.subscribers.len()
                    || old.subscribers.iter().any(|(id, old)| {
                        stored.subscribers.get(id).is_none_or(|s| {
                            s.expires_at != old.expires_at || s.terminal != old.terminal
                        })
                    })
            }) {
                changed.insert(stored.network);
            }
        }
        for (id, stored) in previous.tasks {
            if !self.runtime.observations.tasks.contains_key(&id) {
                changed.insert(stored.network);
            }
        }
        for network_id in changed {
            self.runtime
                .hub
                .publish_committed(CommittedChange::Progress { network_id });
        }
    }
}
impl std::ops::Deref for RuntimeTransaction<'_> {
    type Target = Runtime;
    fn deref(&self) -> &Runtime {
        &self.runtime
    }
}
impl std::ops::DerefMut for RuntimeTransaction<'_> {
    fn deref_mut(&mut self) -> &mut Runtime {
        &mut self.runtime
    }
}
impl Drop for RuntimeTransaction<'_> {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            self.runtime.observations = previous;
        }
    }
}
pub struct ObservationService<P: ObservationProvider> {
    handle: crate::store::Handle,
    provider: Arc<P>,
    runtime: Arc<Mutex<Runtime>>,
}
impl<P: ObservationProvider> Clone for ObservationService<P> {
    fn clone(&self) -> Self {
        Self {
            handle: self.handle.clone(),
            provider: self.provider.clone(),
            runtime: self.runtime.clone(),
        }
    }
}
impl<P: ObservationProvider> ObservationService<P> {
    /// App-owned worker; expiry does not depend on an HTTP request or SSE poll.
    pub(crate) fn start(&self) -> tokio::sync::watch::Sender<bool> {
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let service = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(250));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _=stopped.changed()=>break,
                    _=interval.tick()=>{ if service.maintain().await.is_err() && *stopped.borrow() { break; } }
                }
            }
        });
        stop
    }

    async fn maintain(&self) -> Result<(), StateError> {
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let mut runtime =
                    RuntimeTransaction::new(runtime.lock().map_err(|_| StateError::Unavailable)?);
                runtime.observations.expire(now);
                reauthorize_all(&*provider, &tx, &mut runtime, now, None);
                provider.synchronize(&tx, &runtime.observations.leases(), now)?;
                reauthorize_all(&*provider, &tx, &mut runtime, now, None);
                tx.commit()?;
                runtime.committed();
                Ok(())
            })
            .await?
    }
    pub(crate) fn new(
        handle: crate::store::Handle,
        provider: P,
        runtime: Arc<Mutex<Runtime>>,
    ) -> Self {
        Self {
            handle,
            provider: Arc::new(provider),
            runtime,
        }
    }
    pub async fn create_observation(
        &self,
        identity: ReadIdentity,
        request: CreateObservation,
    ) -> Result<ObservationTask, StateError> {
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let mut runtime =
                    RuntimeTransaction::new(runtime.lock().map_err(|_| StateError::Unavailable)?);
                runtime.observations.expire(now);
                if let Some(stored) = runtime
                    .observations
                    .tasks
                    .values()
                    .find(|s| s.subscribers.contains_key(&request.subscription_id))
                {
                    let scope = provider.scope(&tx, &identity, now)?;
                    let session = provider.observation_session(&tx, &identity, now)?;
                    let subscriber = &stored.subscribers[&request.subscription_id];
                    if stored.network != scope.network_id
                        || subscriber.session != session
                        || subscriber.identity.fingerprint() != identity.fingerprint()
                        || stored.task.source_node_id != request.source_node_id
                        || stored.task.target_node_id != request.target_node_id
                        || stored.task.service != request.service
                        || stored.task.selection != request.selection
                        || stored.task.metrics != request.metrics
                        || stored.task.interval_ms != request.interval_ms
                    {
                        return Err(StateError::Conflict);
                    }
                    if subscriber.authorization_digest != scope.observation_authorization_digest
                        || !scope.full.contains(&request.source_node_id)
                    {
                        return Err(StateError::Forbidden);
                    }
                    let id = stored.task.id;
                    reauthorize_all(&*provider, &tx, &mut runtime, now, Some(scope.network_id));
                    let stored = runtime
                        .observations
                        .tasks
                        .get(&id)
                        .ok_or(StateError::NotFound)?;
                    let task = super::subscriber_view(stored, request.subscription_id)?;
                    tx.commit()?;
                    runtime.committed();
                    return Ok(task);
                }
                let admission = provider.prepare_observation(&tx, &identity, &request, now)?;
                reauthorize_all(
                    &*provider,
                    &tx,
                    &mut runtime,
                    now,
                    Some(admission.report.network_id),
                );
                let task = runtime
                    .observations
                    .create(&admission, identity, request, now)?;
                provider.synchronize(&tx, &runtime.observations.leases(), now)?;
                tx.commit()?;
                runtime.committed();
                runtime.hub.publish_committed(CommittedChange::Observation {
                    network_id: admission.report.network_id,
                    observation_id: task.id,
                });
                runtime.hub.publish_committed(CommittedChange::Progress {
                    network_id: admission.report.network_id,
                });
                Ok(task)
            })
            .await?
    }
    pub async fn observations(
        &self,
        identity: ReadIdentity,
        selected: Option<ObservationSubscriptionId>,
        operation: ObservationOperation,
    ) -> Result<ObservationList, StateError> {
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let scope = provider.scope(&tx, &identity, now)?;
                let session = provider.observation_session(&tx, &identity, now)?;
                let mut runtime =
                    RuntimeTransaction::new(runtime.lock().map_err(|_| StateError::Unavailable)?);
                runtime.observations.expire(now);
                reauthorize_all(&*provider, &tx, &mut runtime, now, Some(scope.network_id));
                let candidates: Vec<_> = runtime
                    .observations
                    .tasks
                    .values()
                    .filter(|stored| {
                        stored.network == scope.network_id
                            && scope.visible.contains(&stored.task.source_node_id)
                            && scope.visible.contains(&stored.task.target_node_id)
                            && stored.task.path.as_ref().is_none_or(|path| {
                                path.iter().all(|node| scope.visible.contains(node))
                            })
                    })
                    .flat_map(|stored| {
                        stored
                            .subscribers
                            .iter()
                            .filter(|(id, subscriber)| {
                                subscriber.session == session
                                    && subscriber.identity.fingerprint() == identity.fingerprint()
                                    && subscriber.authorization_digest
                                        == scope.observation_authorization_digest
                                    && selected.is_none_or(|selected| selected == **id)
                            })
                            .filter_map(|(id, _)| super::subscriber_view(stored, *id).ok())
                    })
                    .collect();
                if selected.is_some()
                    && candidates.is_empty()
                    && operation != ObservationOperation::Cancel
                {
                    return Err(StateError::NotFound);
                }
                let mut items = vec![];
                for task in candidates {
                    let subscription = task.subscription_id.ok_or(StateError::Unavailable)?;
                    if operation == ObservationOperation::Cancel {
                        runtime.observations.cancel(
                            session,
                            scope.network_id,
                            task.id,
                            subscription,
                            now,
                        )?;
                        continue;
                    }
                    if !super::active(task.phase) && operation == ObservationOperation::Read {
                        items.push(task);
                        continue;
                    }
                    let request = CreateObservation {
                        subscription_id: subscription,
                        source_node_id: task.source_node_id,
                        target_node_id: task.target_node_id,
                        service: task.service.clone(),
                        selection: task.selection.clone(),
                        metrics: task.metrics.clone(),
                        interval_ms: task.interval_ms,
                    };
                    let admission = provider.observation(&tx, &identity, &request, now)?;
                    let task = match operation {
                        ObservationOperation::Read => {
                            runtime
                                .observations
                                .get(&admission, task.id, subscription, now)?
                        }
                        ObservationOperation::Renew => {
                            runtime
                                .observations
                                .renew(&admission, task.id, subscription, now)?
                        }
                        ObservationOperation::Cancel => {
                            runtime.observations.cancel(
                                session,
                                scope.network_id,
                                task.id,
                                subscription,
                                now,
                            )?;
                            continue;
                        }
                    };
                    items.push(task);
                }
                reauthorize_all(&*provider, &tx, &mut runtime, now, Some(scope.network_id));
                provider.synchronize(&tx, &runtime.observations.leases(), now)?;
                reauthorize_all(&*provider, &tx, &mut runtime, now, Some(scope.network_id));
                tx.commit()?;
                runtime.committed();
                let items = items
                    .into_iter()
                    .map(|item| {
                        item.subscription_id
                            .and_then(|id| {
                                runtime
                                    .observations
                                    .tasks
                                    .get(&item.id)
                                    .and_then(|s| super::subscriber_view(s, id).ok())
                            })
                            .unwrap_or(item)
                    })
                    .collect();
                Ok(ObservationList { items })
            })
            .await?
    }
    pub async fn node_tasks(&self, authorization: String) -> Result<ObservationList, StateError> {
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let admission = provider.report(&tx, &authorization, now)?;
                let mut runtime =
                    RuntimeTransaction::new(runtime.lock().map_err(|_| StateError::Unavailable)?);
                reauthorize(&*provider, &tx, &mut runtime, &admission, None, now);
                provider.synchronize(&tx, &runtime.observations.leases(), now)?;
                tx.commit()?;
                runtime.committed();
                Ok(ObservationList {
                    items: runtime.observations.node_tasks(&admission, now),
                })
            })
            .await?
    }
    pub async fn observation_result(
        &self,
        authorization: String,
        id: ObservationId,
        result: ObservationResult,
    ) -> Result<ObservationTask, StateError> {
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let admission = provider.report(&tx, &authorization, now)?;
                let mut runtime =
                    RuntimeTransaction::new(runtime.lock().map_err(|_| StateError::Unavailable)?);
                reauthorize(&*provider, &tx, &mut runtime, &admission, Some(id), now);
                let result_digest = crate::resources::content_digest(
                    &serde_json::to_vec(&result).map_err(|_| StateError::Invalid)?,
                );
                let is_sample = matches!(&result.result, ObservationUpdate::Sample { .. });
                let duplicate = runtime
                    .observations
                    .tasks
                    .get(&id)
                    .and_then(|task| match &result.result {
                        ObservationUpdate::Sample { sample, .. } => {
                            task.seen.get(&sample.measurement_id)
                        }
                        ObservationUpdate::Phase { .. } => None,
                    })
                    .is_some_and(|seen| seen.digest == result_digest);
                let original = runtime
                    .observations
                    .tasks
                    .get(&id)
                    .ok_or(StateError::NotFound)?
                    .clone();
                let mut staged = super::Observations {
                    tasks: std::collections::BTreeMap::from([(id, original)]),
                };
                let task = staged.result(&admission, id, result, now)?;
                if let Some(probe) = task
                    .latest
                    .as_ref()
                    .filter(|sample| is_sample && !sample.partial)
                    .and_then(|sample| sample.probe.clone())
                {
                    let occurred_at = probe.completed_at.ok_or(StateError::Invalid)?;
                    let event = meshway_model::observations::ObservationEvent {
                        event_id: probe.measurement_id,
                        configuration: task.configuration.clone(),
                        occurred_at,
                        payload: meshway_model::observations::ObservationPayload::ProbeCompleted {
                            probe,
                        },
                    };
                    super::history::accept(
                        &tx,
                        &admission,
                        HistoryReport {
                            instance_id: admission.instance_id,
                            events: vec![event],
                        },
                        now,
                    )?;
                }
                let mut all_leases = runtime.observations.leases();
                if !super::active(task.phase) {
                    let removed: std::collections::BTreeSet<_> =
                        staged.tasks[&id].subscribers.keys().copied().collect();
                    all_leases.retain(|s| !removed.contains(&s.id));
                }
                provider.synchronize(&tx, &all_leases, now)?;
                tx.commit()?;
                runtime
                    .observations
                    .tasks
                    .insert(id, staged.tasks.remove(&id).ok_or(StateError::Unavailable)?);
                runtime.committed();
                if !duplicate {
                    let change = if is_sample {
                        CommittedChange::ObservationSample {
                            network_id: admission.network_id,
                            observation: Box::new(task.clone()),
                        }
                    } else {
                        CommittedChange::Observation {
                            network_id: admission.network_id,
                            observation_id: id,
                        }
                    };
                    runtime.hub.publish_committed(change);
                }
                Ok(task)
            })
            .await?
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ObservationOperation {
    Read,
    Renew,
    Cancel,
}

fn reauthorize<P: ObservationProvider>(
    provider: &P,
    tx: &Transaction<'_>,
    runtime: &mut crate::state::service::Runtime,
    admission: &crate::state::ReportAdmission,
    _selected: Option<ObservationId>,
    now: DateTime<Utc>,
) {
    reauthorize_all(provider, tx, runtime, now, Some(admission.network_id));
}

fn reauthorize_all<P: ObservationProvider>(
    provider: &P,
    tx: &Transaction<'_>,
    runtime: &mut Runtime,
    now: DateTime<Utc>,
    network: Option<meshway_model::NetworkId>,
) {
    let checks: Vec<_> = runtime
        .observations
        .tasks
        .values()
        .filter(|stored| {
            super::active(stored.task.phase)
                && network.is_none_or(|network| network == stored.network)
        })
        .flat_map(|stored| {
            stored
                .subscribers
                .iter()
                .filter(|(_, subscriber)| subscriber.terminal.is_none())
                .map(|(subscription, subscriber)| {
                    (
                        stored.task.id,
                        *subscription,
                        subscriber.identity.clone(),
                        subscriber.authorization_digest,
                        CreateObservation {
                            subscription_id: *subscription,
                            source_node_id: stored.task.source_node_id,
                            target_node_id: stored.task.target_node_id,
                            service: stored.task.service.clone(),
                            selection: stored.task.selection.clone(),
                            metrics: stored.task.metrics.clone(),
                            interval_ms: stored.task.interval_ms,
                        },
                    )
                })
        })
        .collect();
    for (task, subscription, identity, authorization, request) in checks {
        let current = provider.observation(tx, &identity, &request, now);
        let valid =
            current.as_ref().is_ok_and(|admission| {
                admission.authorization_digest == authorization
                    && runtime.observations.tasks.get(&task).is_some_and(|stored| {
                        admission.report.instance_id == stored.task.instance_id
                    })
            });
        if valid
            && let (Ok(admission), Some(stored)) =
                (current, runtime.observations.tasks.get_mut(&task))
            && stored.task.configuration != admission.configuration
        {
            stored.task.configuration = admission.configuration;
            stored.paths = admission.paths;
            stored.task.phase = ObservationPhase::Preparing;
            stored.task.path = None;
            stored.task.path_segment = None;
            stored.task.latest = None;
        }
        if !valid && let Some(stored) = runtime.observations.tasks.get_mut(&task) {
            stored.subscribers.remove(&subscription);
        }
    }
    runtime.observations.expire(now);
}
