use super::TaskAdmission;
use crate::state::{
    StateError,
    service::{ReadIdentity, StateProvider, StateService},
    stream::CommittedChange,
};
use chrono::{DateTime, Utc};
use meshway_model::{ObservationId, SessionId};
use meshway_protocol::observations::*;
use rusqlite::Transaction;

/// B computes the authorized full path candidates; A checks current session and
/// source User identity ownership. A network role cannot manufacture a path.
pub trait ObservationProvider: StateProvider {
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
impl<P: ObservationProvider> StateService<P> {
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
                let admission = provider.prepare_observation(&tx, &identity, &request, now)?;
                let mut runtime = runtime.lock().map_err(|_| StateError::Unavailable)?;
                let mut staged = runtime.observations.clone();
                let task = staged.create(&admission, request, now)?;
                tx.commit()?;
                runtime.observations = staged;
                runtime
                    .observation_identities
                    .insert(admission.session_id, identity);
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
        selected: Option<ObservationId>,
        operation: ObservationOperation,
    ) -> Result<ObservationList, StateError> {
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let view = provider.view(&tx, &identity, now)?;
                let session = provider.observation_session(&tx, &identity, now)?;
                let mut runtime = runtime.lock().map_err(|_| StateError::Unavailable)?;
                runtime.observations.expire(now);
                let candidates: Vec<_> = runtime
                    .observations
                    .tasks
                    .values()
                    .filter(|stored| {
                        stored.network == view.scope.network_id
                            && stored.subscribers.contains_key(&session)
                            && selected.is_none_or(|id| id == stored.task.id)
                    })
                    .map(|stored| stored.task.clone())
                    .collect();
                if selected.is_some()
                    && candidates.is_empty()
                    && operation != ObservationOperation::Cancel
                {
                    return Err(StateError::NotFound);
                }
                let mut items = vec![];
                for task in candidates {
                    let request = CreateObservation {
                        source_node_id: task.source_node_id,
                        target_node_id: task.target_node_id,
                        selection: task.selection.clone(),
                        metrics: task.metrics.clone(),
                        interval_ms: task.interval_ms,
                    };
                    let admission = provider.observation(&tx, &identity, &request, now)?;
                    let task = match operation {
                        ObservationOperation::Read => {
                            runtime.observations.get(&admission, task.id, now)?
                        }
                        ObservationOperation::Renew => {
                            runtime.observations.renew(&admission, task.id, now)?
                        }
                        ObservationOperation::Cancel => {
                            runtime
                                .observations
                                .cancel(session, view.scope.network_id, task.id)?;
                            continue;
                        }
                    };
                    items.push(task);
                }
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
                let mut runtime = runtime.lock().map_err(|_| StateError::Unavailable)?;
                reauthorize(&*provider, &tx, &mut runtime, now);
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
                let mut runtime = runtime.lock().map_err(|_| StateError::Unavailable)?;
                reauthorize(&*provider, &tx, &mut runtime, now);
                let result_digest = crate::resources::content_digest(
                    &serde_json::to_vec(&result).map_err(|_| StateError::Invalid)?,
                );
                let duplicate = runtime
                    .observations
                    .tasks
                    .get(&id)
                    .and_then(|task| task.seen.get(&result.sample.measurement_id))
                    .is_some_and(|seen| seen.digest == result_digest);
                let mut staged = runtime.observations.clone();
                let task = staged.result(&admission, id, result, now)?;
                if let Some(probe) = task
                    .latest
                    .as_ref()
                    .filter(|sample| !sample.partial)
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
                tx.commit()?;
                runtime.observations = staged;
                if !duplicate {
                    runtime
                        .hub
                        .publish_committed(CommittedChange::ObservationSample {
                            network_id: admission.network_id,
                            observation: Box::new(task.clone()),
                        });
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
    runtime: &mut crate::state::service::Runtime<P>,
    now: DateTime<Utc>,
) {
    let checks: Vec<_> = runtime
        .observations
        .tasks
        .values()
        .flat_map(|stored| {
            stored.subscribers.keys().map(|session| {
                (
                    *session,
                    CreateObservation {
                        source_node_id: stored.task.source_node_id,
                        target_node_id: stored.task.target_node_id,
                        selection: stored.task.selection.clone(),
                        metrics: stored.task.metrics.clone(),
                        interval_ms: stored.task.interval_ms,
                    },
                )
            })
        })
        .collect();
    for (session, request) in checks {
        let valid = runtime
            .observation_identities
            .get(&session)
            .is_some_and(|identity| provider.observation(tx, identity, &request, now).is_ok());
        if !valid {
            runtime.observations.revoke_session(session);
            runtime.observation_identities.remove(&session);
        }
    }
}
