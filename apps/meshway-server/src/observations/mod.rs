//! Short-lived diagnostic demand. Results never extend a lease, select a path
//! outside the compiled candidates, or synthesize a successful measurement.
pub mod history;
pub mod service;
mod undo;
use crate::error::Error;
use crate::state::ReportAdmission;
use chrono::{DateTime, Duration, Utc};
use meshway_model::{Digest, NetworkId, ObservationId, ObservationSubscriptionId, SessionId};
use meshway_protocol::observations::*;
use std::collections::{BTreeMap, BTreeSet};

pub const OBSERVATION_LEASE_SECONDS: i64 = 30;
pub const MAX_TASKS_PER_NETWORK: usize = 128;
pub const MAX_TASKS_PER_SESSION: usize = 8;
pub const MAX_TASKS: usize = 1024;
pub const MAX_SUBSCRIPTIONS: usize = 4096;

#[derive(Clone)]
pub struct TaskAdmission {
    pub observation_id: Option<ObservationId>,
    pub unsupported_metrics: Vec<meshway_model::observations::MetricKind>,
    pub report: ReportAdmission,
    /// Prepared compiler target; distinct from the Node's actual execution lease.
    pub configuration: meshway_model::ConfigurationRef,
    pub paths: Vec<Vec<meshway_model::NodeId>>,
    pub node_expires_at: DateTime<Utc>,
    pub session_id: SessionId,
    pub authorization_digest: Digest,
    pub session_expires_at: DateTime<Utc>,
}
#[derive(Clone)]
struct Subscriber {
    generation: Digest,
    session: SessionId,
    identity: crate::state::service::ReadIdentity,
    authorization_digest: Digest,
    expires_at: DateTime<Utc>,
    renew_after: DateTime<Utc>,
    terminal: Option<ObservationPhase>,
}
#[derive(Clone)]
struct StoredTask {
    accepted_at: DateTime<Utc>,
    network: NetworkId,
    task: ObservationTask,
    paths: Vec<Vec<meshway_model::NodeId>>,
    subscribers: BTreeMap<ObservationSubscriptionId, Subscriber>,
    seen: std::sync::Arc<BTreeMap<meshway_model::EventId, SeenWindow>>,
}
#[derive(Clone)]
struct SeenWindow {
    digest: Digest,
    version: meshway_model::Version,
    partial: bool,
}
#[derive(Default)]
pub struct Observations {
    tasks: BTreeMap<ObservationId, StoredTask>,
    undo: Option<undo::ObservationUndo>,
}
impl Observations {
    pub(crate) fn leases(&self) -> Vec<service::SubscriptionLease> {
        self.tasks
            .values()
            .filter(|s| active(s.task.phase))
            .flat_map(|s| s.subscribers.iter())
            .filter(|(_, s)| s.terminal.is_none())
            .map(|(id, s)| service::SubscriptionLease {
                id: *id,
                expires_at: s.expires_at,
            })
            .collect()
    }
    /// Private lifetime tokens prevent a client reusing an old request UUID
    /// after terminal retention from reviving that lifetime's queued samples.
    pub(crate) fn active_subscriptions(
        &self,
        tasks: &[ObservationTask],
    ) -> BTreeMap<ObservationSubscriptionId, Digest> {
        tasks
            .iter()
            .filter(|task| active(task.phase))
            .filter_map(|task| {
                let id = task.subscription_id?;
                let subscriber = self.tasks.get(&task.id)?.subscribers.get(&id)?;
                let mut bytes = subscriber.generation.as_bytes().to_vec();
                bytes.extend_from_slice(task.configuration.digest.as_bytes());
                Some((id, crate::resources::content_digest(&bytes)))
            })
            .collect()
    }
    pub(crate) fn visible_tasks(
        &self,
        network: NetworkId,
        scope: &crate::state::VisibilityScope,
        identity: &crate::state::service::ReadIdentity,
    ) -> Vec<ObservationTask> {
        self.tasks
            .values()
            .filter(|stored| {
                stored.network == network
                    && scope.visible.contains(&stored.task.source_node_id)
                    && scope.visible.contains(&stored.task.target_node_id)
                    && stored
                        .task
                        .path
                        .as_ref()
                        .is_none_or(|path| path.iter().all(|node| scope.visible.contains(node)))
            })
            .flat_map(|stored| {
                stored
                    .subscribers
                    .iter()
                    .filter(|(_, subscriber)| {
                        subscriber.identity.fingerprint() == identity.fingerprint()
                            && subscriber.authorization_digest
                                == scope.observation_authorization_digest
                            && scope.full.contains(&stored.task.source_node_id)
                            && scope
                                .current
                                .get(&stored.task.source_node_id)
                                .is_some_and(|node| {
                                    node.instance_id == Some(stored.task.instance_id)
                                        && !node.revoked
                                        && (!active(stored.task.phase)
                                            || node.configurations.first()
                                                == Some(&stored.task.configuration))
                                })
                    })
                    .filter_map(|(subscription, _)| subscriber_view(stored, *subscription).ok())
            })
            .collect()
    }
    pub fn create(
        &mut self,
        admission: &TaskAdmission,
        identity: crate::state::service::ReadIdentity,
        request: CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<ObservationTask, Error> {
        self.expire(now);
        if admission.session_expires_at <= now {
            return Err(Error::InvalidSession);
        }
        if admission.node_expires_at <= now {
            return Err(Error::StateConflict);
        }
        if request.source_node_id != admission.report.node_id
            || request.metrics.is_empty()
            || request.metrics.len() > 3
            || !(1000..=30_000).contains(&request.interval_ms)
            || request
                .metrics
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != request.metrics.len()
        {
            return Err(Error::InvalidRequest);
        }
        let paths: Vec<_> = admission
            .paths
            .iter()
            .filter(|path| {
                path.first() == Some(&request.source_node_id)
                    && path.last() == Some(&request.target_node_id)
            })
            .cloned()
            .collect();
        if paths.is_empty() {
            return Err(Error::PermissionDenied);
        }
        if let ObservationPath::Explicit { path } = &request.selection
            && !paths.contains(path)
        {
            return Err(Error::PermissionDenied);
        }
        let configuration = admission.configuration.clone();
        let expires_at = (now + Duration::seconds(OBSERVATION_LEASE_SECONDS))
            .min(admission.session_expires_at)
            .min(admission.node_expires_at);
        let session_tasks = self
            .tasks
            .values()
            .flat_map(|task| task.subscribers.values())
            .filter(|subscriber| subscriber.session == admission.session_id)
            .count();
        // The client supplies a distinct UUID per tab demand; retries keep it.
        // A reused UUID with different input/owner is a conflict, never renewal.
        for stored in self.tasks.values() {
            if let Some(subscriber) = stored.subscribers.get(&request.subscription_id) {
                if stored.network != admission.report.network_id
                    || subscriber.identity.fingerprint() != identity.fingerprint()
                    || subscriber.session != admission.session_id
                    || subscriber.authorization_digest != admission.authorization_digest
                    || stored.task.source_node_id != request.source_node_id
                    || stored.task.target_node_id != request.target_node_id
                    || stored.task.service != request.service
                    || stored.task.selection != request.selection
                    || stored.task.metrics != request.metrics
                    || stored.task.interval_ms != request.interval_ms
                {
                    return Err(Error::StateConflict);
                }
                return subscriber_view(stored, request.subscription_id);
            }
        }
        if session_tasks >= MAX_TASKS_PER_SESSION
            || self
                .tasks
                .values()
                .map(|task| task.subscribers.len())
                .sum::<usize>()
                >= MAX_SUBSCRIPTIONS
        {
            return Err(Error::ObservationLimit);
        }
        let subscriber = Subscriber {
            generation: crate::resources::content_digest(
                crate::accounts::random_uuid()?.as_bytes(),
            ),
            session: admission.session_id,
            identity,
            authorization_digest: admission.authorization_digest,
            expires_at,
            renew_after: now + Duration::seconds(10),
            terminal: None,
        };
        // Reuse only exact source, authorized publication, mode, metrics and
        // interval. Each subscriber retains its own revocation/lease context.
        let reusable = self
            .tasks
            .values()
            .find(|stored| {
                stored.network == admission.report.network_id
                    && stored.task.source_node_id == request.source_node_id
                    && stored.task.target_node_id == request.target_node_id
                    && stored.task.service == request.service
                    && stored.task.selection == request.selection
                    && stored.task.metrics == request.metrics
                    && stored.task.interval_ms == request.interval_ms
                    && stored.task.configuration == configuration
                    && stored.task.instance_id == admission.report.instance_id
                    && stored.task.expires_at > now
                    && active(stored.task.phase)
            })
            .map(|stored| stored.task.id);
        if let Some(id) = reusable {
            let stored = self.tasks.get(&id).ok_or(Error::NotFound)?;
            if stored.subscribers.len() >= 64 || session_tasks >= MAX_TASKS_PER_SESSION {
                return Err(Error::ObservationLimit);
            }
            let stored = self.task_mut(id).ok_or(Error::NotFound)?;
            stored
                .subscribers
                .insert(request.subscription_id, subscriber);
            stored.task.expires_at = stored.task.expires_at.max(expires_at);
            return subscriber_view(stored, request.subscription_id);
        }
        if self
            .tasks
            .values()
            .filter(|task| task.network == admission.report.network_id)
            .count()
            >= MAX_TASKS_PER_NETWORK
            || self.tasks.len() >= MAX_TASKS
        {
            return Err(Error::ObservationLimit);
        }
        let id = admission.observation_id.unwrap_or(
            ObservationId::new(crate::accounts::random_uuid()?)
                .map_err(|_| Error::Internal("generated observation ID"))?,
        );
        let task = ObservationTask {
            id,
            source_node_id: request.source_node_id,
            target_node_id: request.target_node_id,
            service: request.service,
            selection: request.selection,
            path: None,
            path_segment: None,
            configuration,
            instance_id: admission.report.instance_id,
            metrics: request.metrics.clone(),
            interval_ms: request.interval_ms,
            phase: if request
                .metrics
                .iter()
                .all(|metric| admission.unsupported_metrics.contains(metric))
            {
                ObservationPhase::Unsupported
            } else {
                ObservationPhase::Preparing
            },
            unsupported_metrics: admission.unsupported_metrics.clone(),
            expires_at,
            renew_after: now + Duration::seconds(10),
            latest: None,
            subscriber_expires_at: None,
            subscription_id: None,
        };
        self.insert_task(
            id,
            StoredTask {
                accepted_at: now,
                network: admission.report.network_id,
                task: task.clone(),
                paths,
                subscribers: BTreeMap::from([(request.subscription_id, subscriber)]),
                seen: std::sync::Arc::default(),
            },
        );
        subscriber_view(
            self.tasks
                .get(&id)
                .ok_or(Error::Internal("accepted observation task missing"))?,
            request.subscription_id,
        )
    }
    pub fn get(
        &mut self,
        admission: &TaskAdmission,
        id: ObservationId,
        subscription: ObservationSubscriptionId,
        now: DateTime<Utc>,
    ) -> Result<ObservationTask, Error> {
        self.expire(now);
        self.authorize(id, admission, subscription, now)?;
        let stored = self.tasks.get(&id).ok_or(Error::NotFound)?;
        subscriber_view(stored, subscription)
    }
    pub fn renew(
        &mut self,
        admission: &TaskAdmission,
        id: ObservationId,
        subscription: ObservationSubscriptionId,
        now: DateTime<Utc>,
    ) -> Result<ObservationTask, Error> {
        self.authorize(id, admission, subscription, now)?;
        let stored = self.tasks.get(&id).ok_or(Error::NotFound)?;
        if stored.seen.len() >= 4096
            || stored
                .subscribers
                .get(&subscription)
                .is_none_or(|subscriber| {
                    now < subscriber.renew_after
                        || subscriber.terminal.is_some()
                        || subscriber.expires_at <= now
                })
            || !active(stored.task.phase)
        {
            return Err(Error::ObservationLimit);
        }
        let expires = (now + Duration::seconds(OBSERVATION_LEASE_SECONDS))
            .min(admission.session_expires_at)
            .min(admission.node_expires_at);
        let stored = self.task_mut(id).ok_or(Error::NotFound)?;
        let subscriber = stored
            .subscribers
            .get_mut(&subscription)
            .ok_or(Error::NotFound)?;
        subscriber.expires_at = expires;
        subscriber.renew_after = now + Duration::seconds(10);
        stored.task.expires_at = stored
            .subscribers
            .values()
            .filter(|s| s.terminal.is_none())
            .map(|s| s.expires_at)
            .max()
            .ok_or(Error::NotFound)?;
        stored.task.renew_after = now + Duration::seconds(10);
        subscriber_view(stored, subscription)
    }
    pub fn cancel(
        &mut self,
        session: SessionId,
        network: NetworkId,
        id: ObservationId,
        subscription: ObservationSubscriptionId,
        now: DateTime<Utc>,
    ) -> Result<(), Error> {
        let stored = self
            .tasks
            .get(&id)
            .filter(|task| task.network == network)
            .ok_or(Error::NotFound)?;
        let subscriber = stored
            .subscribers
            .get(&subscription)
            .filter(|s| s.session == session)
            .ok_or(Error::NotFound)?;
        if subscriber.terminal.is_some() {
            return Ok(());
        }
        let stored = self.task_mut(id).ok_or(Error::NotFound)?;
        let subscriber = stored
            .subscribers
            .get_mut(&subscription)
            .ok_or(Error::NotFound)?;
        subscriber.terminal = Some(ObservationPhase::Cancelled);
        subscriber.expires_at = now;
        if stored.subscribers.values().all(|s| s.terminal.is_some()) {
            if active(stored.task.phase) {
                stored.task.phase = ObservationPhase::Cancelled;
                stored.task.expires_at = now;
            }
        } else {
            stored.task.expires_at = stored
                .subscribers
                .values()
                .filter(|s| s.terminal.is_none())
                .map(|subscriber| subscriber.expires_at)
                .max()
                .ok_or(Error::NotFound)?;
        }
        Ok(())
    }
    pub fn revoke_session(&mut self, session: SessionId) {
        let affected: Vec<_> = self
            .tasks
            .iter()
            .filter(|(_, task)| task.subscribers.values().any(|s| s.session == session))
            .map(|(id, _)| *id)
            .collect();
        for id in affected {
            let task = self.task_mut(id).expect("selected observation");
            task.subscribers
                .retain(|_, subscriber| subscriber.session != session);
        }
        self.expire(Utc::now());
    }
    pub fn node_tasks(
        &mut self,
        admission: &ReportAdmission,
        now: DateTime<Utc>,
    ) -> Vec<ObservationTask> {
        self.expire(now);
        self.tasks
            .values()
            .filter(|task| {
                task.network == admission.network_id
                    && task.task.source_node_id == admission.node_id
                    && task.task.instance_id == admission.instance_id
                    && admission.lease_expires_at > now
                    && task.task.expires_at > now
                    && active(task.task.phase)
                    && task.subscribers.values().any(|s| s.terminal.is_none())
                    && admission.execution.as_ref() == Some(&task.task.configuration)
            })
            .map(|task| task.task.clone())
            .collect()
    }
    pub fn result(
        &mut self,
        admission: &ReportAdmission,
        id: ObservationId,
        result: ObservationResult,
        now: DateTime<Utc>,
    ) -> Result<ObservationTask, Error> {
        self.expire(now);
        let stored = self.tasks.get(&id).ok_or(Error::NotFound)?;
        if stored.network != admission.network_id
            || stored.task.source_node_id != admission.node_id
            || stored.task.instance_id != admission.instance_id
            || result.instance_id != admission.instance_id
            || result.configuration != stored.task.configuration
            || admission.execution.as_ref() != Some(&result.configuration)
            || admission.lease_expires_at <= now
            || stored.task.expires_at <= now
            || !stored.subscribers.values().any(|s| s.terminal.is_none())
        {
            return Err(Error::InvalidSession);
        }
        let digest = crate::resources::content_digest(
            &serde_json::to_vec(&result).map_err(|_| Error::encode("observation result"))?,
        );
        let (path, path_segment, sample) = match result.result {
            ObservationUpdate::Phase { phase } => {
                let phase = match phase {
                    ReportedObservationPhase::Preparing => ObservationPhase::Preparing,
                    ReportedObservationPhase::Failed => ObservationPhase::Failed,
                    ReportedObservationPhase::Completed => ObservationPhase::Completed,
                    ReportedObservationPhase::Unsupported => ObservationPhase::Unsupported,
                };
                if !active(stored.task.phase) && stored.task.phase != phase
                    || stored.task.phase == ObservationPhase::Running
                        && phase == ObservationPhase::Preparing
                {
                    return Err(Error::StateConflict);
                }
                if stored.task.phase == phase && (active(phase) || stored.task.latest.is_none()) {
                    return Ok(stored.task.clone());
                }
                let stored = self.task_mut(id).ok_or(Error::NotFound)?;
                stored.task.phase = phase;
                if !active(phase) {
                    stored.task.latest = None;
                }
                return Ok(stored.task.clone());
            }
            ObservationUpdate::Sample {
                path,
                path_segment,
                sample,
            } => (path, path_segment, *sample),
        };
        if !active(stored.task.phase) {
            return Err(Error::StateConflict);
        }
        if !stored.paths.contains(&path)
            || !admission.current()?.paths.contains(&path)
            || path_segment.get() == 0
        {
            return Err(Error::InvalidRequest);
        }
        if let ObservationPath::Explicit { path: selected } = &stored.task.selection
            && *selected != path
        {
            return Err(Error::InvalidRequest);
        }
        if let Some(segment) = stored.task.path_segment
            && (path_segment < segment
                || path_segment == segment && stored.task.path.as_ref() != Some(&path))
        {
            return Err(Error::StateConflict);
        }
        if sample.probe.is_none() && sample.traffic.is_none() && sample.probe_counts.is_none()
            || sample.window_start >= sample.window_end
            || sample.window_start < stored.accepted_at
            || sample.window_end > now + Duration::seconds(30)
            || sample.window_end > stored.task.expires_at
            || now - sample.window_start > Duration::minutes(2)
        {
            return Err(Error::InvalidRequest);
        }
        if sample.traffic.is_some()
            && stored
                .task
                .unsupported_metrics
                .contains(&meshway_model::observations::MetricKind::Traffic)
        {
            // Scoped business counters have no production capability yet. A
            // missing counter is unknown, never fabricated zero traffic.
            return Err(Error::InvalidRequest);
        }
        if let Some(probe) = &sample.probe {
            if probe.kind != meshway_model::state::ProbeKind::Path
                || probe.started_at < sample.window_start
                || probe.completed_at.is_some_and(|at| at > sample.window_end)
                || !stored.task.metrics.iter().any(|metric| {
                    matches!(
                        metric,
                        meshway_model::observations::MetricKind::Rtt
                            | meshway_model::observations::MetricKind::ProbeLoss
                    )
                })
            {
                return Err(Error::InvalidRequest);
            }
            if probe.outcome == meshway_model::state::ProbeOutcome::Pending && !sample.partial {
                return Err(Error::InvalidRequest);
            }
            if probe.measurement_id != sample.measurement_id
                || probe.path != path
                || probe.publication_id != result.configuration.execution_publication
            {
                return Err(Error::InvalidRequest);
            }
            crate::state::validation::validate_probe(admission, probe, now)?;
        }
        if let Some(counts) = &sample.probe_counts {
            let total = u64::from(counts.replied_within_deadline)
                + u64::from(counts.timeout)
                + u64::from(counts.pending);
            if total != u64::from(counts.sent) || sample.partial != (counts.pending > 0) {
                return Err(Error::InvalidRequest);
            }
        }
        if sample.traffic.is_some()
            && !stored
                .task
                .metrics
                .contains(&meshway_model::observations::MetricKind::Traffic)
            || sample.probe_counts.is_some()
                && !stored
                    .task
                    .metrics
                    .contains(&meshway_model::observations::MetricKind::ProbeLoss)
        {
            return Err(Error::InvalidRequest);
        }
        if let Some(previous) = stored.seen.get(&sample.measurement_id) {
            if previous.digest == digest {
                return Ok(stored.task.clone());
            }
            let old = stored.task.latest.as_ref().ok_or(Error::StateConflict)?;
            if !previous.partial
                || sample.version <= previous.version
                || old.measurement_id != sample.measurement_id
                || old.window_start != sample.window_start
                || old.window_end != sample.window_end
                || stored.task.path_segment != Some(path_segment)
            {
                return Err(Error::StateConflict);
            }
            if let (Some(old), Some(new)) = (&old.probe_counts, &sample.probe_counts)
                && (old.sent != new.sent
                    || old.replied_within_deadline > new.replied_within_deadline
                    || old.timeout > new.timeout)
            {
                return Err(Error::StateConflict);
            }
        }
        if !stored.seen.contains_key(&sample.measurement_id) && stored.seen.len() >= 4096 {
            return Err(Error::ObservationLimit);
        }
        if stored.task.latest.as_ref().is_some_and(|previous| {
            previous.measurement_id != sample.measurement_id
                && previous.window_end > sample.window_start
        }) {
            return Err(Error::StateConflict);
        }
        let stored = self.task_mut(id).ok_or(Error::NotFound)?;
        std::sync::Arc::make_mut(&mut stored.seen).insert(
            sample.measurement_id,
            SeenWindow {
                digest,
                version: sample.version,
                partial: sample.partial,
            },
        );
        stored.task.path = Some(path);
        stored.task.path_segment = Some(path_segment);
        stored.task.latest = Some(sample);
        stored.task.phase = ObservationPhase::Running;
        Ok(stored.task.clone())
    }
    pub fn expire(&mut self, now: DateTime<Utc>) {
        let affected: Vec<_> = self
            .tasks
            .iter()
            .filter(|(_, task)| {
                task.subscribers.values().any(|s| {
                    s.expires_at <= now && s.terminal.is_none()
                        || s.expires_at + Duration::minutes(1) <= now
                }) || active(task.task.phase)
                    && task
                        .subscribers
                        .values()
                        .filter(|s| s.terminal.is_none())
                        .map(|s| s.expires_at)
                        .max()
                        != Some(task.task.expires_at)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in affected {
            let task = self.task_mut(id).expect("selected observation");
            for subscriber in task.subscribers.values_mut() {
                if subscriber.expires_at <= now && subscriber.terminal.is_none() {
                    subscriber.terminal = Some(if active(task.task.phase) {
                        ObservationPhase::Expired
                    } else {
                        task.task.phase
                    });
                }
            }
            task.subscribers
                .retain(|_, s| s.expires_at + Duration::minutes(1) > now);
            if task.subscribers.values().all(|s| s.terminal.is_some()) && active(task.task.phase) {
                task.task.phase = ObservationPhase::Expired;
                task.task.latest = None;
            } else if active(task.task.phase)
                && let Some(expiry) = task
                    .subscribers
                    .values()
                    .filter(|s| s.terminal.is_none())
                    .map(|s| s.expires_at)
                    .max()
            {
                task.task.expires_at = expiry;
            }
        }
        let removed: Vec<_> = self
            .tasks
            .iter()
            .filter(|(_, task)| task.task.expires_at + Duration::minutes(1) <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in removed {
            self.remove_task(id);
        }
    }
    fn authorize(
        &mut self,
        id: ObservationId,
        admission: &TaskAdmission,
        subscription: ObservationSubscriptionId,
        now: DateTime<Utc>,
    ) -> Result<(), Error> {
        let task = self.tasks.get(&id).ok_or(Error::NotFound)?;
        let subscriber = task.subscribers.get(&subscription).ok_or(Error::NotFound)?;
        let error = if admission.session_expires_at <= now {
            Some(Error::InvalidSession)
        } else if task.network != admission.report.network_id
            || subscriber.session != admission.session_id
            || subscriber.authorization_digest != admission.authorization_digest
        {
            Some(Error::PermissionDenied)
        } else if task.task.instance_id != admission.report.instance_id
            || task.task.configuration != admission.configuration
            || admission.node_expires_at <= now
            || !admission
                .report
                .configurations
                .contains(&task.task.configuration)
        {
            Some(Error::StateConflict)
        } else {
            None
        };
        if let Some(error) = error {
            self.task_mut(id)
                .ok_or(Error::NotFound)?
                .subscribers
                .remove(&subscription);
            return Err(error);
        }
        Ok(())
    }
}

fn subscriber_view(
    stored: &StoredTask,
    subscription: ObservationSubscriptionId,
) -> Result<ObservationTask, Error> {
    let subscriber = stored
        .subscribers
        .get(&subscription)
        .ok_or(Error::NotFound)?;
    let mut task = stored.task.clone();
    task.subscriber_expires_at = Some(subscriber.expires_at);
    task.subscription_id = Some(subscription);
    if let Some(phase) = subscriber.terminal {
        task.phase = phase;
        task.latest = None;
    }
    task.renew_after = subscriber.renew_after;
    Ok(task)
}
pub(crate) fn active(phase: ObservationPhase) -> bool {
    matches!(
        phase,
        ObservationPhase::Preparing | ObservationPhase::Running
    )
}
