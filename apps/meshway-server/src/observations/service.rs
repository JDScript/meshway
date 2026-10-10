use super::TaskAdmission;
use crate::effect::Effect;
use crate::error::Error;
use crate::state::service::{ReadIdentity, Runtime};
use chrono::{DateTime, Utc};
use meshway_model::{ObservationId, ObservationSubscriptionId, SessionId};
use meshway_protocol::observations::*;
use meshway_protocol::{InstanceRequest, node_observations::*};
use rusqlite::Transaction;
use std::sync::{Arc, Mutex};

/// Owned by App, not by a Router clone. Aborting this async producer cannot
/// cancel a SQL closure already accepted by the independently owned executor.
pub(crate) struct ObservationMaintenance {
    stop: tokio::sync::watch::Sender<bool>,
    worker: tokio::task::JoinHandle<()>,
}
impl ObservationMaintenance {
    pub(crate) async fn shutdown(mut self) -> Result<(), tokio::task::JoinError> {
        let _ = self.stop.send(true);
        (&mut self.worker).await
    }
}
impl Drop for ObservationMaintenance {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        self.worker.abort();
    }
}

#[derive(Default)]
pub(crate) enum MaintenanceOwner {
    #[default]
    Idle,
    Running(ObservationMaintenance),
    Closed,
}

/// B computes the authorized full path candidates; A checks current session and
/// source User identity ownership. A network role cannot manufacture a path.
pub trait ObservationProvider: Send + Sync + 'static {
    /// True only when all authorization changes publish committed wakeups;
    /// subscriber deadlines remain an independent maintenance trigger.
    fn event_driven_maintenance(&self) -> bool {
        false
    }
    fn node_observation_identity(
        &self,
        _: &Transaction<'_>,
        _: &str,
        _: &InstanceRequest,
        _: DateTime<Utc>,
    ) -> Result<NodeObservationIdentity, Error> {
        Err(Error::PermissionDenied)
    }
    fn prepare_node_observation(
        &self,
        _: &Transaction<'_>,
        _: &str,
        _: &CreateNodeObservation,
        _: DateTime<Utc>,
    ) -> Result<NodeObservationAdmission, Error> {
        Err(Error::PermissionDenied)
    }
    /// `stored` is the accepted subscriber's owner and immutable selection.
    fn node_observation(
        &self,
        _: &Transaction<'_>,
        _: ObservationSubscriptionId,
        _: &StoredSubscription,
        _: DateTime<Utc>,
    ) -> Result<NodeObservationAdmission, Error> {
        Err(Error::PermissionDenied)
    }
    /// Return whether synchronization may have changed task authorization
    /// bindings, requiring a second check before exposing their current view.
    fn synchronize(
        &self,
        _: &Transaction<'_>,
        _: &[SubscriptionLease],
        _: DateTime<Utc>,
    ) -> Result<bool, Error> {
        // Unknown providers conservatively require a second authorization pass.
        Ok(true)
    }
    fn scope(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<crate::state::VisibilityScope, Error>;
    fn report(
        &self,
        tx: &Transaction<'_>,
        authorization: &str,
        now: DateTime<Utc>,
    ) -> Result<crate::state::ReportAdmission, Error>;
    /// May synthesize and persist the diagnostic execution demand within this
    /// transaction. The observation service commits it together with task acceptance before publishing.
    fn prepare_observation(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<TaskAdmission, Error> {
        self.observation(tx, identity, request, now)
    }
    /// Read-only reauthorization for already accepted tasks and subscribers.
    fn observation(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        request: &CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<TaskAdmission, Error>;
    fn observation_session(
        &self,
        tx: &Transaction<'_>,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<SessionId, Error>;
}
/// An accepted subscriber as held in RAM, for reauthorization.
pub struct StoredSubscription {
    pub(crate) observation: ObservationId,
    pub(crate) principal: super::SubscriberPrincipal,
    pub(crate) request: CreateObservation,
}
pub struct NodeObservationIdentity {
    pub(crate) principal: super::SubscriberPrincipal,
    pub(crate) expires_at: DateTime<Utc>,
}
pub struct NodeObservationAdmission {
    pub(crate) admission: TaskAdmission,
    pub(crate) request: CreateObservation,
    pub(crate) target: NodeObservationTargetView,
}
#[derive(PartialEq, Eq)]
pub struct SubscriptionLease {
    pub id: ObservationSubscriptionId,
    pub expires_at: DateTime<Utc>,
}

/// SQLite rollback restores only observations actually changed by this call.
/// Keep this guard alive through SQL COMMIT: dropping it at the end of a
/// `Handle::transact` operation would roll RAM back before the executor commits,
/// while calling `committed` there would discard undo before COMMIT can fail.
struct RuntimeTransaction<'a> {
    runtime: std::sync::MutexGuard<'a, Runtime>,
}
impl<'a> RuntimeTransaction<'a> {
    fn new(mut runtime: std::sync::MutexGuard<'a, Runtime>) -> Self {
        runtime.observations.begin_changes();
        Self { runtime }
    }
    fn committed(&mut self) {
        let changed = self.runtime.observations.commit_changes();
        for network_id in changed {
            self.runtime
                .hub
                .publish_committed(Effect::Progress { network_id });
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
        self.runtime.observations.rollback_changes();
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
    pub(crate) fn start(&self) -> ObservationMaintenance {
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let service = self.clone();
        let worker = tokio::spawn(async move {
            let mut activity = match service.runtime.lock() {
                Ok(runtime) => runtime.hub.wakeups.activity(),
                Err(_) => return,
            };
            let coalesce = std::time::Duration::from_millis(250);
            loop {
                let outcome = tokio::select! {
                    biased;
                    _=stopped.changed()=>break,
                    result=service.maintain(activity.clone())=>result,
                };
                let Ok((deadline, observed)) = outcome else {
                    tokio::select! {biased; _=stopped.changed()=>break, _=tokio::time::sleep(coalesce)=>{}}
                    continue;
                };
                activity = observed;
                let deadline = deadline.map(|deadline| {
                    tokio::time::Instant::now()
                        + (deadline - Utc::now()).to_std().unwrap_or_default()
                });
                let fallback = (!service.provider.event_driven_maintenance())
                    .then(|| tokio::time::Instant::now() + coalesce);
                let due = deadline.into_iter().chain(fallback).min();
                let changed = tokio::select! {
                    biased;
                    _=stopped.changed()=>break,
                    _=wait_deadline(due)=>false,
                    _=activity.changed()=>true,
                };
                // Coalesce committed traffic without periodic reauthorization.
                // A subscription/retention deadline is never postponed by it.
                if !changed {
                    continue;
                }
                let due = due.map_or(tokio::time::Instant::now() + coalesce, |due| {
                    due.min(tokio::time::Instant::now() + coalesce)
                });
                tokio::select! {
                    biased;
                    _=stopped.changed()=>break,
                    _=tokio::time::sleep_until(due)=>{}
                }
            }
        });
        ObservationMaintenance { stop, worker }
    }

    async fn maintain(
        &self,
        mut activity: tokio::sync::watch::Receiver<u64>,
    ) -> Result<(Option<DateTime<Utc>>, tokio::sync::watch::Receiver<u64>), Error> {
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let mut runtime =
                    RuntimeTransaction::new(runtime.lock().map_err(|_| Error::RUNTIME)?);
                runtime.observations.expire(now);
                reauthorize_all(&*provider, &tx, &mut runtime, now, None);
                if provider.synchronize(&tx, &runtime.observations.leases(), now)? {
                    reauthorize_all(&*provider, &tx, &mut runtime, now, None);
                }
                tx.commit()?;
                runtime.committed();
                // Mark on the serialized owner, after our own effects. A later
                // accepted transaction cannot be swallowed by the async waiter.
                activity.borrow_and_update();
                Ok((runtime.observations.next_deadline(), activity))
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
    ) -> Result<ObservationTask, Error> {
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let mut runtime =
                    RuntimeTransaction::new(runtime.lock().map_err(|_| Error::RUNTIME)?);
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
                        || subscriber.principal.user_session() != Some(session)
                        || !subscriber
                            .principal
                            .user_identity()
                            .is_some_and(|owner| owner.fingerprint() == identity.fingerprint())
                        || stored.task.source_node_id != request.source_node_id
                        || stored.task.target_node_id != request.target_node_id
                        || stored.task.service != request.service
                        || stored.task.selection != request.selection
                        || stored.task.metrics != request.metrics
                        || stored.task.interval_ms != request.interval_ms
                    {
                        return Err(Error::StateConflict);
                    }
                    if subscriber.authorization_digest != scope.observation_authorization_digest
                        || !scope.full.contains(&request.source_node_id)
                    {
                        return Err(Error::PermissionDenied);
                    }
                    let id = stored.task.id;
                    reauthorize_all(&*provider, &tx, &mut runtime, now, Some(scope.network_id));
                    let stored = runtime.observations.tasks.get(&id).ok_or(Error::NotFound)?;
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
                runtime.hub.publish_committed(Effect::Observation {
                    network_id: admission.report.network_id,
                    observation_id: task.id,
                });
                runtime.hub.publish_committed(Effect::Progress {
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
    ) -> Result<ObservationList, Error> {
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let scope = provider.scope(&tx, &identity, now)?;
                let session = provider.observation_session(&tx, &identity, now)?;
                let mut runtime =
                    RuntimeTransaction::new(runtime.lock().map_err(|_| Error::RUNTIME)?);
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
                                subscriber.principal.user_session() == Some(session)
                                    && subscriber.principal.user_identity().is_some_and(|owner| {
                                        owner.fingerprint() == identity.fingerprint()
                                    })
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
                    return Err(Error::NotFound);
                }
                let mut items = vec![];
                for task in candidates {
                    let subscription = task
                        .subscription_id
                        .ok_or(Error::Internal("observation subscription missing"))?;
                    let renew = match operation {
                        ObservationOperation::Read => false,
                        ObservationOperation::Renew => true,
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
                    if !super::active(task.phase) && !renew {
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
                    let task = if renew {
                        runtime
                            .observations
                            .renew(&admission, task.id, subscription, now)?
                    } else {
                        runtime
                            .observations
                            .get(&admission, task.id, subscription, now)?
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
    pub async fn create_node_observation(
        &self,
        authorization: String,
        request: CreateNodeObservation,
    ) -> Result<NodeObservationResponse, Error> {
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
        let queued_at = std::time::Instant::now();
        self.handle
            .execute(move |connection| {
                let started_at = std::time::Instant::now();
                let tx = connection.transaction()?;
                let now = Utc::now();
                let mut runtime =
                    RuntimeTransaction::new(runtime.lock().map_err(|_| Error::RUNTIME)?);
                runtime.observations.expire(now);
                let prepared =
                    provider.prepare_node_observation(&tx, &authorization, &request, now)?;
                reauthorize_all(
                    &*provider,
                    &tx,
                    &mut runtime,
                    now,
                    Some(prepared.admission.report.network_id),
                );
                let task = runtime.observations.create_admitted(
                    &prepared.admission,
                    prepared.request,
                    now,
                )?;
                provider.synchronize(&tx, &runtime.observations.leases(), now)?;
                tx.commit()?;
                runtime.committed();
                runtime.hub.publish_committed(Effect::Observation {
                    network_id: prepared.admission.report.network_id,
                    observation_id: task.id,
                });
                tracing::info!(subscription = ?task.subscription_id, observation = %task.id,
                    netmap_serial = task.netmap.serial.get(),
                    queue_us = started_at.duration_since(queued_at).as_micros() as u64,
                    processing_us = started_at.elapsed().as_micros() as u64,
                    "Node observation subscription admitted");
                Ok(NodeObservationResponse {
                    task,
                    target: prepared.target,
                })
            })
            .await?
    }

    pub async fn node_observation(
        &self,
        authorization: String,
        instance: InstanceRequest,
        subscription: ObservationSubscriptionId,
        expires_at_ms: Option<u64>,
    ) -> Result<NodeObservationResponse, Error> {
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let owner =
                    provider.node_observation_identity(&tx, &authorization, &instance, now)?;
                let mut runtime =
                    RuntimeTransaction::new(runtime.lock().map_err(|_| Error::RUNTIME)?);
                let stored = stored_request(&runtime, subscription).ok_or(Error::NotFound)?;
                let current = provider.node_observation(&tx, subscription, &stored, now)?;
                if !owner.principal.same_owner(&current.admission.principal) {
                    return Err(Error::PermissionDenied);
                }
                runtime.observations.expire(now);
                reauthorize_all(
                    &*provider,
                    &tx,
                    &mut runtime,
                    now,
                    Some(current.admission.report.network_id),
                );
                let id = current.admission.observation_id.ok_or(Error::NotFound)?;
                runtime
                    .observations
                    .authorize(id, &current.admission, subscription, now)?;
                if let Some(ms) = expires_at_ms {
                    let expiry = i64::try_from(ms)
                        .ok()
                        .and_then(DateTime::from_timestamp_millis)
                        .ok_or(Error::InvalidRequest)?;
                    let stored = runtime.observations.tasks.get(&id).ok_or(Error::NotFound)?;
                    let subscriber = stored
                        .subscribers
                        .get(&subscription)
                        .ok_or(Error::NotFound)?;
                    if expiry <= now
                        || expiry
                            > now + chrono::Duration::seconds(super::OBSERVATION_LEASE_SECONDS)
                        || expiry > owner.expires_at
                        || expiry > current.admission.node_expires_at
                        || subscriber.terminal.is_some()
                        || !super::active(stored.task.phase)
                    {
                        return Err(Error::InvalidRequest);
                    }
                    // An absolute deadline is the retry identity. Equal/older requests
                    // never slide the lease or its ten-second renewal threshold.
                    if expiry > subscriber.expires_at {
                        if now < subscriber.renew_after || stored.seen.len() >= 4096 {
                            return Err(Error::ObservationLimit);
                        }
                        let stored = runtime.observations.task_mut(id).ok_or(Error::NotFound)?;
                        let subscriber = stored
                            .subscribers
                            .get_mut(&subscription)
                            .ok_or(Error::NotFound)?;
                        subscriber.expires_at = expiry;
                        subscriber.renew_after = now + chrono::Duration::seconds(10);
                        stored.task.expires_at = stored
                            .subscribers
                            .values()
                            .filter(|s| s.terminal.is_none())
                            .map(|s| s.expires_at)
                            .max()
                            .ok_or(Error::NotFound)?;
                    }
                }
                provider.synchronize(&tx, &runtime.observations.leases(), now)?;
                reauthorize_all(
                    &*provider,
                    &tx,
                    &mut runtime,
                    now,
                    Some(current.admission.report.network_id),
                );
                let task = super::subscriber_view(
                    runtime.observations.tasks.get(&id).ok_or(Error::NotFound)?,
                    subscription,
                )?;
                tx.commit()?;
                runtime.committed();
                Ok(NodeObservationResponse {
                    task,
                    target: current.target,
                })
            })
            .await?
    }

    pub async fn cancel_node_observation(
        &self,
        authorization: String,
        instance: InstanceRequest,
        subscription: ObservationSubscriptionId,
    ) -> Result<NodeObservationCancelled, Error> {
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let owner =
                    provider.node_observation_identity(&tx, &authorization, &instance, now)?;
                let mut runtime =
                    RuntimeTransaction::new(runtime.lock().map_err(|_| Error::RUNTIME)?);
                runtime.observations.expire(now);
                let stored = runtime
                    .observations
                    .tasks
                    .values()
                    .find(|s| s.subscribers.contains_key(&subscription))
                    .ok_or(Error::NotFound)?;
                if !stored.subscribers[&subscription]
                    .principal
                    .same_owner(&owner.principal)
                {
                    return Err(Error::PermissionDenied);
                }
                if stored.subscribers[&subscription].terminal != Some(ObservationPhase::Cancelled) {
                    let stored = stored_request(&runtime, subscription).ok_or(Error::NotFound)?;
                    provider.node_observation(&tx, subscription, &stored, now)?;
                }
                let id = stored.task.id;
                let network = stored.network;
                runtime.observations.cancel_owned(id, subscription, now)?;
                reauthorize_all(&*provider, &tx, &mut runtime, now, Some(network));
                provider.synchronize(&tx, &runtime.observations.leases(), now)?;
                reauthorize_all(&*provider, &tx, &mut runtime, now, Some(network));
                // Executor view, never another subscriber's ID or lease.
                let task = runtime
                    .observations
                    .tasks
                    .get(&id)
                    .filter(|s| {
                        super::active(s.task.phase)
                            && s.subscribers.values().any(|s| s.terminal.is_none())
                    })
                    .map(|s| s.task.clone());
                tx.commit()?;
                runtime.committed();
                Ok(NodeObservationCancelled { task })
            })
            .await?
    }

    pub async fn node_tasks(&self, authorization: String) -> Result<ObservationList, Error> {
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let admission = provider.report(&tx, &authorization, now)?;
                let mut runtime =
                    RuntimeTransaction::new(runtime.lock().map_err(|_| Error::RUNTIME)?);
                reauthorize_all(
                    &*provider,
                    &tx,
                    &mut runtime,
                    now,
                    Some(admission.network_id),
                );
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
    ) -> Result<ObservationTask, Error> {
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let admission = provider.report(&tx, &authorization, now)?;
                let mut runtime =
                    RuntimeTransaction::new(runtime.lock().map_err(|_| Error::RUNTIME)?);
                reauthorize_all(
                    &*provider,
                    &tx,
                    &mut runtime,
                    now,
                    Some(admission.network_id),
                );
                let result_digest = crate::resources::content_digest(
                    &serde_json::to_vec(&result)
                        .map_err(|_| Error::encode("observation result"))?,
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
                let task = runtime.observations.result(&admission, id, result, now)?;
                if let Some(probe) = task
                    .latest
                    .as_ref()
                    .filter(|sample| is_sample && !sample.partial)
                    .and_then(|sample| sample.probe.clone())
                {
                    let occurred_at = probe.completed_at.ok_or(Error::InvalidRequest)?;
                    let event = meshway_model::observations::ObservationEvent {
                        event_id: probe.measurement_id,
                        netmap: task.netmap,
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
                let all_leases = runtime.observations.leases();
                provider.synchronize(&tx, &all_leases, now)?;
                tx.commit()?;
                runtime.committed();
                if !duplicate {
                    let change = if is_sample {
                        Effect::ObservationSample {
                            network_id: admission.network_id,
                            observation: Box::new(task.clone()),
                        }
                    } else {
                        Effect::Observation {
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

/// The owner and immutable selection of an accepted subscriber.
fn stored_request(
    runtime: &Runtime,
    subscription: ObservationSubscriptionId,
) -> Option<StoredSubscription> {
    runtime.observations.tasks.values().find_map(|stored| {
        stored
            .subscribers
            .get(&subscription)
            .map(|subscriber| StoredSubscription {
                observation: stored.task.id,
                principal: subscriber.principal.clone(),
                request: CreateObservation {
                    subscription_id: subscription,
                    source_node_id: stored.task.source_node_id,
                    target_node_id: stored.task.target_node_id,
                    service: stored.task.service.clone(),
                    selection: stored.task.selection.clone(),
                    metrics: stored.task.metrics.clone(),
                    interval_ms: stored.task.interval_ms,
                },
            })
    })
}

async fn wait_deadline(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ObservationOperation {
    Read,
    Renew,
    Cancel,
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
                .map(|(subscription, _)| (stored.task.id, *subscription))
        })
        .collect();
    for (task, subscription) in checks {
        let stored = &runtime.observations.tasks[&task];
        let subscriber = &stored.subscribers[&subscription];
        let request = CreateObservation {
            subscription_id: subscription,
            source_node_id: stored.task.source_node_id,
            target_node_id: stored.task.target_node_id,
            service: stored.task.service.clone(),
            selection: stored.task.selection.clone(),
            metrics: stored.task.metrics.clone(),
            interval_ms: stored.task.interval_ms,
        };
        let current = match &subscriber.principal {
            super::SubscriberPrincipal::UserSession { identity, .. } => {
                provider.observation(tx, identity, &request, now)
            }
            super::SubscriberPrincipal::NodeInstance { .. } => provider
                .node_observation(
                    tx,
                    subscription,
                    &StoredSubscription {
                        observation: task,
                        principal: subscriber.principal.clone(),
                        request: request.clone(),
                    },
                    now,
                )
                .map(|value| value.admission),
        };
        let valid =
            current.as_ref().is_ok_and(|admission| {
                admission.principal.same_owner(&subscriber.principal)
                    && admission.authorization_digest == subscriber.authorization_digest
                    && runtime.observations.tasks.get(&task).is_some_and(|stored| {
                        admission.report.instance_id == stored.task.instance_id
                    })
            });
        if valid
            && let Ok(admission) = current
            && runtime.observations.tasks.get(&task).is_some_and(|stored| {
                stored.task.netmap != admission.netmap
                    || stored.task.unsupported_metrics != admission.unsupported_metrics
            })
        {
            let stored = runtime
                .observations
                .task_mut(task)
                .expect("authorized observation");
            stored.task.netmap = admission.netmap;
            stored.paths = admission.paths;
            stored.task.phase = if stored
                .task
                .metrics
                .iter()
                .all(|metric| admission.unsupported_metrics.contains(metric))
            {
                ObservationPhase::Unsupported
            } else {
                ObservationPhase::Preparing
            };
            stored.task.unsupported_metrics = admission.unsupported_metrics;
            stored.task.path = None;
            stored.task.path_segment = stored.segment_high_water;
            stored.task.latest = None;
        }
        if !valid && let Some(stored) = runtime.observations.task_mut(task) {
            stored.subscribers.remove(&subscription);
        }
    }
    runtime.observations.expire(now);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observations::{SeenWindow, StoredTask, Subscriber};
    use meshway_model::*;
    use std::{
        collections::BTreeMap,
        sync::atomic::{AtomicBool, Ordering},
    };

    fn id<T: TryFrom<uuid::Uuid>>(n: u128) -> T
    where
        T::Error: std::fmt::Debug,
    {
        T::try_from(uuid::Uuid::from_u128(n)).unwrap()
    }
    fn task(n: u128, now: DateTime<Utc>, windows: usize) -> StoredTask {
        let configuration = NetmapRef {
            revision: Revision::MIN,
            serial: meshway_model::Version::MIN,
            digest: Digest::from_bytes([7; 32]),
        };
        StoredTask {
            accepted_at: now - chrono::Duration::seconds(1),
            segment_high_water: None,
            window_high_water: None,
            network: id(1000),
            task: ObservationTask {
                id: id(n),
                source_node_id: id(10 + n),
                target_node_id: id(20 + n),
                service: meshway_configuration::execution::ObservationServiceScope::IcmpEcho,
                selection: ObservationPath::Auto,
                path: None,
                path_segment: None,
                netmap: configuration,
                instance_id: id(30 + n),
                metrics: vec![meshway_model::observations::MetricKind::Rtt],
                interval_ms: 1000,
                phase: ObservationPhase::Preparing,
                unsupported_metrics: vec![],
                expires_at: now + chrono::Duration::seconds(30),
                renew_after: now + chrono::Duration::seconds(10),
                latest: None,
                subscriber_expires_at: None,
                subscription_id: None,
            },
            paths: vec![vec![id(10 + n), id(20 + n)]],
            subscribers: BTreeMap::from([(
                id(200 + n),
                Subscriber {
                    generation: Digest::from_bytes([1; 32]),
                    principal: super::super::SubscriberPrincipal::UserSession {
                        session: id(500),
                        identity: ReadIdentity::UserSession {
                            digest: [2; 32],
                            network: id(1000),
                        },
                    },
                    authorization_digest: Digest::from_bytes([3; 32]),
                    expires_at: now + chrono::Duration::seconds(30),
                    renew_after: now + chrono::Duration::seconds(10),
                    terminal: None,
                },
            )]),
            seen: Arc::new(
                (0..windows)
                    .map(|i| {
                        (
                            id(10000 + i as u128),
                            SeenWindow {
                                digest: Digest::from_bytes([4; 32]),
                                version: Version::MIN,
                                partial: false,
                            },
                        )
                    })
                    .collect(),
            ),
        }
    }

    #[test]
    fn rollback_journals_only_changed_tasks_and_preserves_large_unrelated_window_index() {
        let now = Utc::now();
        let mut runtime =
            Runtime::new(std::sync::Arc::new(crate::state::facts::NoNetmaps)).unwrap();
        runtime.observations.tasks.insert(id(1), task(1, now, 1));
        runtime.observations.tasks.insert(id(2), task(2, now, 4096));
        let runtime = Mutex::new(runtime);
        let original =
            serde_json::to_vec(&runtime.lock().unwrap().observations.tasks[&id(1)].task).unwrap();
        let affected_index = Arc::as_ptr(&runtime.lock().unwrap().observations.tasks[&id(1)].seen);
        let unrelated_index = Arc::as_ptr(&runtime.lock().unwrap().observations.tasks[&id(2)].seen);
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE parent(id INTEGER PRIMARY KEY); CREATE TABLE pending(parent INTEGER REFERENCES parent(id) DEFERRABLE INITIALLY DEFERRED);").unwrap();
        let failure = (|| -> Result<(), Error> {
            let tx = db.transaction()?;
            let mut staged = RuntimeTransaction::new(runtime.lock().unwrap());
            // Includes repeated whole-network expiry sweeps with no due change.
            // The old full-cache checkpoint increased BOTH Arc strong counts.
            for _ in 0..4 {
                staged.observations.expire(now);
                assert_eq!(
                    Arc::strong_count(&staged.observations.tasks[&id(1)].seen),
                    1
                );
                assert_eq!(
                    Arc::strong_count(&staged.observations.tasks[&id(2)].seen),
                    1
                );
            }
            let target = &staged.observations.tasks[&id(1)].task;
            let admission = TaskAdmission {
                observation_id: Some(id(1)),
                unsupported_metrics: vec![],
                report: crate::state::ReportAdmission {
                    network_id: id(1000),
                    node_id: target.source_node_id,
                    instance_id: target.instance_id,
                    instance_generation: meshway_model::Version::MIN,
                    lease_expires_at: now + chrono::Duration::minutes(2),
                    execution: Some(target.netmap),
                    configurations: vec![target.netmap],
                    contexts: vec![],
                    mesh_addresses: Default::default(),
                },
                netmap: target.netmap,
                paths: vec![],
                node_expires_at: now + chrono::Duration::minutes(2),
                principal: super::super::SubscriberPrincipal::UserSession {
                    session: id(500),
                    identity: ReadIdentity::UserSession {
                        digest: [2; 32],
                        network: id(1000),
                    },
                },
                authorization_digest: Digest::from_bytes([3; 32]),
                authorization_expires_at: now + chrono::Duration::minutes(2),
            };
            staged.observations.get(&admission, id(1), id(201), now)?;
            assert_eq!(
                Arc::strong_count(&staged.observations.tasks[&id(1)].seen),
                1
            );
            assert_eq!(
                Arc::strong_count(&staged.observations.tasks[&id(2)].seen),
                1
            );
            staged.observations.renew(
                &admission,
                id(1),
                id(201),
                now + chrono::Duration::seconds(11),
            )?;
            staged
                .observations
                .cancel(id(500), id(1000), id(1), id(201), now)?;
            assert_eq!(
                Arc::strong_count(&staged.observations.tasks[&id(1)].seen),
                2
            );
            assert_eq!(
                Arc::strong_count(&staged.observations.tasks[&id(2)].seen),
                1
            );
            // A target-only window edit uses COW; rollback must recover the
            // original index itself, not just its length or the public DTO.
            Arc::make_mut(&mut staged.observations.task_mut(id(1)).unwrap().seen).clear();
            tx.execute("INSERT INTO pending VALUES(404)", [])?;
            tx.commit()?;
            staged.committed();
            Ok(())
        })();
        assert!(failure.is_err());
        assert_eq!(
            db.query_row("SELECT count(*) FROM pending", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            0
        );
        let runtime = runtime.lock().unwrap();
        let restored = &runtime.observations.tasks[&id(1)];
        assert_eq!(serde_json::to_vec(&restored.task).unwrap(), original);
        assert_eq!(restored.subscribers[&id(201)].terminal, None);
        assert_eq!(Arc::as_ptr(&restored.seen), affected_index);
        assert_eq!(restored.seen.len(), 1);
        let unrelated = &runtime.observations.tasks[&id(2)];
        assert_eq!(Arc::as_ptr(&unrelated.seen), unrelated_index);
        assert_eq!(Arc::strong_count(&unrelated.seen), 1);
        assert_eq!(unrelated.seen.len(), 4096);
    }

    #[test]
    fn rollback_restores_expired_removals_and_removes_only_new_insertions() {
        let now = Utc::now();
        let mut runtime =
            Runtime::new(std::sync::Arc::new(crate::state::facts::NoNetmaps)).unwrap();
        runtime
            .observations
            .tasks
            .insert(id(1), task(1, now - chrono::Duration::minutes(2), 1));
        runtime.observations.tasks.insert(id(2), task(2, now, 4096));
        let runtime = Mutex::new(runtime);
        {
            let mut staged = RuntimeTransaction::new(runtime.lock().unwrap());
            staged.observations.expire(now);
            assert!(!staged.observations.tasks.contains_key(&id(1)));
            staged.observations.insert_task(id(3), task(3, now, 0));
            assert_eq!(
                Arc::strong_count(&staged.observations.tasks[&id(2)].seen),
                1
            );
            // Dropping the uncommitted SQL operation restores this write set.
        }
        let runtime = runtime.lock().unwrap();
        assert!(runtime.observations.tasks.contains_key(&id(1)));
        assert!(!runtime.observations.tasks.contains_key(&id(3)));
        assert_eq!(
            runtime.observations.tasks[&id(1)].task.phase,
            ObservationPhase::Preparing
        );
        assert_eq!(
            Arc::strong_count(&runtime.observations.tasks[&id(2)].seen),
            1
        );
    }

    async fn until(mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !condition() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("lifecycle boundary");
    }

    struct CountMaintenance(Arc<std::sync::atomic::AtomicUsize>);
    impl ObservationProvider for CountMaintenance {
        fn event_driven_maintenance(&self) -> bool {
            true
        }
        fn synchronize(
            &self,
            _: &Transaction<'_>,
            _: &[SubscriptionLease],
            _: DateTime<Utc>,
        ) -> Result<bool, Error> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(false)
        }
        fn scope(
            &self,
            _: &Transaction<'_>,
            _: &ReadIdentity,
            _: DateTime<Utc>,
        ) -> Result<crate::state::VisibilityScope, Error> {
            Err(Error::PermissionDenied)
        }
        fn report(
            &self,
            _: &Transaction<'_>,
            _: &str,
            _: DateTime<Utc>,
        ) -> Result<crate::state::ReportAdmission, Error> {
            Err(Error::PermissionDenied)
        }
        fn observation(
            &self,
            _: &Transaction<'_>,
            _: &ReadIdentity,
            _: &CreateObservation,
            _: DateTime<Utc>,
        ) -> Result<TaskAdmission, Error> {
            Err(Error::PermissionDenied)
        }
        fn observation_session(
            &self,
            _: &Transaction<'_>,
            _: &ReadIdentity,
            _: DateTime<Utc>,
        ) -> Result<SessionId, Error> {
            Err(Error::PermissionDenied)
        }
    }

    #[tokio::test]
    async fn maintenance_waits_for_committed_changes_instead_of_quarter_second_reauthorization() {
        let temp = tempfile::tempdir().unwrap();
        let app = crate::App::open(crate::Settings::new(
            temp.path().canonicalize().unwrap().join("data"),
        ))
        .await
        .unwrap();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let service = app.observation_service(CountMaintenance(calls.clone()));
        let maintenance = service.start();
        until(|| calls.load(Ordering::SeqCst) == 1).await;
        tokio::time::sleep(std::time::Duration::from_millis(650)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let runtime = service.runtime.clone();
        service
            .handle
            .execute(move |connection| {
                connection.transaction().unwrap().commit().unwrap();
                runtime
                    .lock()
                    .unwrap()
                    .hub
                    .publish_committed(Effect::Progress {
                        network_id: id(1000),
                    });
            })
            .await
            .unwrap();
        until(|| calls.load(Ordering::SeqCst) == 2).await;
        tokio::time::sleep(std::time::Duration::from_millis(650)).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "own completion must not trigger a reauthorization loop"
        );
        maintenance.shutdown().await.unwrap();
        app.shutdown().await.unwrap();
    }

    #[test]
    fn maintenance_deadline_keeps_subscription_expiry_separate_from_terminal_retention() {
        let now = Utc::now();
        let mut observations = crate::observations::Observations::default();
        observations.tasks.insert(id(1), task(1, now, 0));
        assert_eq!(
            observations.next_deadline(),
            Some(now + chrono::Duration::seconds(30))
        );
        observations.expire(now + chrono::Duration::seconds(30));
        assert_eq!(
            observations.next_deadline(),
            Some(now + chrono::Duration::seconds(90))
        );
        observations.expire(now + chrono::Duration::seconds(90));
        assert_eq!(observations.next_deadline(), None);
    }

    #[tokio::test]
    async fn app_owns_one_lazy_worker_and_shutdown_joins_before_storage_closes() {
        let temp = tempfile::tempdir().unwrap();
        let app = crate::App::open(crate::Settings::new(
            temp.path().canonicalize().unwrap().join("data"),
        ))
        .await
        .unwrap();
        let service = app.observations();
        let runtime = Arc::downgrade(&service.runtime);
        let handle = service.handle.clone();
        let count = Arc::strong_count(&service.runtime);
        // No yield: spawned workers cannot finish and hide a duplicate start.
        for _ in 0..32 {
            drop(app.observations());
        }
        assert_eq!(Arc::strong_count(&service.runtime), count);
        drop(service);
        app.shutdown().await.unwrap();
        assert!(
            runtime.upgrade().is_none(),
            "maintenance still owns the runtime after shutdown"
        );
        assert!(matches!(
            handle.execute(|_| ()).await,
            Err(crate::store::ExecutorError::Closed)
        ));
    }

    #[tokio::test]
    async fn maintenance_join_and_cancelled_app_shutdown_preserve_accepted_sql_drain() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap().join("data");
        let mut settings = crate::Settings::new(path.clone());
        // One queued job per background producer (resource scan, traversal GC,
        // observation maintenance), plus the write whose acceptance we prove.
        const QUEUE_CAPACITY: usize = 4;
        settings.database_queue_capacity = QUEUE_CAPACITY;
        let app = crate::App::open(settings).await.unwrap();
        let service = app.observation_service(app.netmap_diagnostics());
        let handle = service.handle.clone();
        let blocker_handle = handle.clone();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let committed = Arc::new(AtomicBool::new(false));
        let completed = committed.clone();
        let blocker = tokio::spawn(async move {
            blocker_handle.execute(move |db| {
                let tx=db.transaction().unwrap();
                tx.execute_batch("CREATE TEMP TABLE accepted_observation_work(value INTEGER); INSERT INTO accepted_observation_work VALUES(1)").unwrap();
                entered.send(()).unwrap();
                blocked.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
                tx.commit().unwrap();
                assert_eq!(db.query_row("SELECT value FROM accepted_observation_work",[],|r|r.get::<_,u32>(0)).unwrap(),1);
                completed.store(true,Ordering::Release);
            }).await.unwrap();
        });
        started.await.unwrap();
        // This provider belongs only to this observation worker; Runtime is
        // also retained by the independently scheduled traversal janitor.
        let before = Arc::strong_count(&service.provider);
        let maintenance = service.start();
        let abort = maintenance.worker.abort_handle();
        // The worker's service clone and SQL closure each retain this provider.
        // The bounded queue has room for all three producer jobs.
        until(|| Arc::strong_count(&service.provider) == before + 2).await;
        let queued_handle = handle.clone();
        let queued_commit = Arc::new(AtomicBool::new(false));
        let queued_completed = queued_commit.clone();
        let (submitting, submitted) = tokio::sync::oneshot::channel();
        let queued = tokio::spawn(async move {
            submitting.send(()).unwrap();
            queued_handle
                .execute(move |db| {
                    let tx = db.transaction().unwrap();
                    tx.execute("INSERT INTO accepted_observation_work VALUES(2)", [])
                        .unwrap();
                    tx.commit().unwrap();
                    queued_completed.store(true, Ordering::Release);
                })
                .await
                .unwrap();
        });
        // This current-thread test resumes after the submitter reaches its
        // reply await: at most three producer jobs occupied the queue, so this
        // write was accepted regardless of timer scheduling.
        submitted.await.unwrap();
        // Saturate any slots unused by background producers. Surplus fillers
        // wait for admission and must receive Closed; accepted ones still drain.
        let mut fillers = Vec::new();
        for _ in 0..QUEUE_CAPACITY {
            let handle = handle.clone();
            let (submitting, submitted) = tokio::sync::oneshot::channel();
            fillers.push(tokio::spawn(async move {
                submitting.send(()).unwrap();
                handle.execute(|_| ()).await
            }));
            submitted.await.unwrap();
        }
        maintenance.shutdown().await.unwrap();
        assert!(abort.is_finished());
        assert_eq!(Arc::strong_count(&service.provider), before + 1);
        assert!(!committed.load(Ordering::Acquire));
        assert!(!queued_commit.load(Ordering::Acquire));
        let closing = tokio::spawn(app.shutdown());
        // Cannot enter the full queue; closing the executor must reject it.
        assert!(matches!(
            handle
                .execute(|_| panic!("new work admitted during close"))
                .await,
            Err(crate::store::ExecutorError::Closed)
        ));
        closing.abort();
        assert!(closing.await.unwrap_err().is_cancelled());
        assert!(matches!(
            crate::App::open(crate::Settings::new(path.clone())).await,
            Err(crate::AppError::AlreadyRunning)
        ));
        release.send(()).unwrap();
        blocker.await.unwrap();
        queued.await.unwrap();
        for filler in fillers {
            assert!(matches!(
                filler.await.unwrap(),
                Ok(()) | Err(crate::store::ExecutorError::Closed)
            ));
        }
        assert!(committed.load(Ordering::Acquire));
        assert!(queued_commit.load(Ordering::Acquire));
        drop(service);
        let reopened = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                match crate::App::open(crate::Settings::new(path.clone())).await {
                    Ok(app) => break app,
                    Err(crate::AppError::AlreadyRunning) => tokio::task::yield_now().await,
                    Err(error) => panic!("unexpected reopen result: {error}"),
                }
            }
        })
        .await
        .unwrap();
        reopened.check_storage().await.unwrap();
        reopened.shutdown().await.unwrap();
        assert!(abort.is_finished());
    }

    #[tokio::test]
    async fn cancelling_maintenance_shutdown_aborts_the_owned_async_task() {
        struct Finished(Arc<AtomicBool>);
        impl Drop for Finished {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let finished = Arc::new(AtomicBool::new(false));
        let marker = Finished(finished.clone());
        let (entered, started) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(async move {
            let _marker = marker;
            entered.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        let abort = worker.abort_handle();
        let (stop, _) = tokio::sync::watch::channel(false);
        let maintenance = ObservationMaintenance { stop, worker };
        started.await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), maintenance.shutdown())
                .await
                .is_err()
        );
        until(|| abort.is_finished()).await;
        assert!(finished.load(Ordering::Acquire));
    }
}
