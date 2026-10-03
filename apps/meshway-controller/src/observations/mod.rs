//! Short-lived diagnostic demand. Results never extend a lease, select a path
//! outside the compiled candidates, or synthesize a successful measurement.
pub mod history;
pub mod service;
use crate::state::{ReportAdmission, StateError};
use chrono::{DateTime, Duration, Utc};
use meshway_model::{Digest, NetworkId, ObservationId, SessionId};
use meshway_protocol::observations::*;
use std::collections::{BTreeMap, BTreeSet};

pub const OBSERVATION_LEASE_SECONDS: i64 = 30;
pub const MAX_TASKS_PER_NETWORK: usize = 128;
pub const MAX_TASKS_PER_SESSION: usize = 8;

#[derive(Clone)]
pub struct TaskAdmission {
    pub report: ReportAdmission,
    pub session_id: SessionId,
    pub authorization_digest: Digest,
    pub session_expires_at: DateTime<Utc>,
}
#[derive(Clone)]
struct Subscriber {
    authorization_digest: Digest,
    expires_at: DateTime<Utc>,
    renew_after: DateTime<Utc>,
}
#[derive(Clone)]
struct StoredTask {
    network: NetworkId,
    task: ObservationTask,
    paths: Vec<Vec<meshway_model::NodeId>>,
    subscribers: BTreeMap<SessionId, Subscriber>,
    seen: BTreeMap<meshway_model::EventId, SeenWindow>,
}
#[derive(Clone)]
struct SeenWindow {
    digest: Digest,
    version: meshway_model::Version,
    partial: bool,
}
#[derive(Default, Clone)]
pub struct Observations {
    tasks: BTreeMap<ObservationId, StoredTask>,
}
impl Observations {
    pub(crate) fn visible_tasks(
        &self,
        network: NetworkId,
        scope: &crate::state::VisibilityScope,
        identities: &BTreeMap<SessionId, crate::state::service::ReadIdentity>,
        identity: &crate::state::service::ReadIdentity,
    ) -> Vec<ObservationTask> {
        self.tasks
            .values()
            .filter(|stored| {
                stored.network == network
                    && stored.subscribers.iter().any(|(session, subscriber)| {
                        identities
                            .get(session)
                            .is_some_and(|other| other.fingerprint() == identity.fingerprint())
                            && subscriber.authorization_digest == scope.authorization_digest
                    })
                    && scope.visible.contains(&stored.task.source_node_id)
                    && scope.visible.contains(&stored.task.target_node_id)
            })
            .filter_map(|stored| {
                stored
                    .subscribers
                    .iter()
                    .find(|(session, subscriber)| {
                        identities
                            .get(session)
                            .is_some_and(|other| other.fingerprint() == identity.fingerprint())
                            && subscriber.authorization_digest == scope.authorization_digest
                    })
                    .and_then(|(session, _)| subscriber_view(stored, *session).ok())
            })
            .collect()
    }
    pub fn create(
        &mut self,
        admission: &TaskAdmission,
        request: CreateObservation,
        now: DateTime<Utc>,
    ) -> Result<ObservationTask, StateError> {
        self.expire(now);
        if admission.session_expires_at <= now || admission.report.lease_expires_at <= now {
            return Err(StateError::Unauthorized);
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
            return Err(StateError::Invalid);
        }
        let paths: Vec<_> = admission
            .report
            .paths
            .iter()
            .filter(|path| {
                path.first() == Some(&request.source_node_id)
                    && path.last() == Some(&request.target_node_id)
            })
            .cloned()
            .collect();
        if paths.is_empty() {
            return Err(StateError::Forbidden);
        }
        if let ObservationPath::Explicit { path } = &request.selection
            && !paths.contains(path)
        {
            return Err(StateError::Forbidden);
        }
        let configuration = admission
            .report
            .execution
            .clone()
            .ok_or(StateError::Unavailable)?;
        let expires_at = (now + Duration::seconds(OBSERVATION_LEASE_SECONDS))
            .min(admission.session_expires_at)
            .min(admission.report.lease_expires_at);
        let session_tasks = self
            .tasks
            .values()
            .filter(|task| task.subscribers.contains_key(&admission.session_id))
            .count();
        // Reuse only exact source, authorized publication, mode, metrics and
        // interval. Each subscriber retains its own revocation/lease context.
        for stored in self.tasks.values_mut() {
            if stored.network == admission.report.network_id
                && stored.task.source_node_id == request.source_node_id
                && stored.task.target_node_id == request.target_node_id
                && stored.task.selection == request.selection
                && stored.task.metrics == request.metrics
                && stored.task.interval_ms == request.interval_ms
                && stored.task.configuration == configuration
                && stored.task.instance_id == admission.report.instance_id
                && stored.task.expires_at > now
            {
                if stored.subscribers.contains_key(&admission.session_id) {
                    return subscriber_view(stored, admission.session_id);
                }
                if stored.subscribers.len() >= 64 || session_tasks >= MAX_TASKS_PER_SESSION {
                    return Err(StateError::ObservationLimit);
                }
                stored.subscribers.insert(
                    admission.session_id,
                    Subscriber {
                        authorization_digest: admission.authorization_digest,
                        expires_at,
                        renew_after: now + Duration::seconds(10),
                    },
                );
                stored.task.expires_at = stored.task.expires_at.max(expires_at);
                return subscriber_view(stored, admission.session_id);
            }
        }
        if self
            .tasks
            .values()
            .filter(|task| task.network == admission.report.network_id)
            .count()
            >= MAX_TASKS_PER_NETWORK
            || self
                .tasks
                .values()
                .filter(|task| task.subscribers.contains_key(&admission.session_id))
                .count()
                >= MAX_TASKS_PER_SESSION
        {
            return Err(StateError::ObservationLimit);
        }
        let id = ObservationId::new(
            crate::accounts::random_uuid().map_err(|_| StateError::Unavailable)?,
        )
        .map_err(|_| StateError::Unavailable)?;
        let task = ObservationTask {
            id,
            source_node_id: request.source_node_id,
            target_node_id: request.target_node_id,
            selection: request.selection,
            path: None,
            path_segment: None,
            configuration,
            instance_id: admission.report.instance_id,
            metrics: request.metrics,
            interval_ms: request.interval_ms,
            phase: ObservationPhase::Preparing,
            expires_at,
            renew_after: now + Duration::seconds(10),
            latest: None,
            subscriber_expires_at: None,
        };
        self.tasks.insert(
            id,
            StoredTask {
                network: admission.report.network_id,
                task: task.clone(),
                paths,
                subscribers: BTreeMap::from([(
                    admission.session_id,
                    Subscriber {
                        authorization_digest: admission.authorization_digest,
                        expires_at,
                        renew_after: now + Duration::seconds(10),
                    },
                )]),
                seen: BTreeMap::new(),
            },
        );
        subscriber_view(
            self.tasks.get(&id).ok_or(StateError::Unavailable)?,
            admission.session_id,
        )
    }
    pub fn get(
        &mut self,
        admission: &TaskAdmission,
        id: ObservationId,
        now: DateTime<Utc>,
    ) -> Result<ObservationTask, StateError> {
        self.expire(now);
        let stored = self.tasks.get_mut(&id).ok_or(StateError::NotFound)?;
        Self::authorize(stored, admission, now)?;
        subscriber_view(stored, admission.session_id)
    }
    pub fn renew(
        &mut self,
        admission: &TaskAdmission,
        id: ObservationId,
        now: DateTime<Utc>,
    ) -> Result<ObservationTask, StateError> {
        let stored = self.tasks.get_mut(&id).ok_or(StateError::NotFound)?;
        Self::authorize(stored, admission, now)?;
        if stored.seen.len() >= 4096
            || stored
                .subscribers
                .get(&admission.session_id)
                .is_none_or(|subscriber| now < subscriber.renew_after)
        {
            return Err(StateError::ObservationLimit);
        }
        let expires = (now + Duration::seconds(OBSERVATION_LEASE_SECONDS))
            .min(admission.session_expires_at)
            .min(admission.report.lease_expires_at);
        stored.subscribers.insert(
            admission.session_id,
            Subscriber {
                authorization_digest: admission.authorization_digest,
                expires_at: expires,
                renew_after: now + Duration::seconds(10),
            },
        );
        stored.task.expires_at = stored
            .subscribers
            .values()
            .map(|s| s.expires_at)
            .max()
            .ok_or(StateError::NotFound)?;
        stored.task.renew_after = now + Duration::seconds(10);
        subscriber_view(stored, admission.session_id)
    }
    pub fn cancel(
        &mut self,
        session: SessionId,
        network: NetworkId,
        id: ObservationId,
    ) -> Result<(), StateError> {
        let stored = self
            .tasks
            .get_mut(&id)
            .filter(|task| task.network == network)
            .ok_or(StateError::NotFound)?;
        stored.subscribers.remove(&session);
        if stored.subscribers.is_empty() {
            stored.task.phase = ObservationPhase::Cancelled;
            stored.task.expires_at = Utc::now();
        } else {
            stored.task.expires_at = stored
                .subscribers
                .values()
                .map(|subscriber| subscriber.expires_at)
                .max()
                .ok_or(StateError::NotFound)?;
        }
        Ok(())
    }
    pub fn revoke_session(&mut self, session: SessionId) {
        for task in self.tasks.values_mut() {
            task.subscribers.remove(&session);
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
                    && !task.subscribers.is_empty()
                    && admission.configurations.contains(&task.task.configuration)
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
    ) -> Result<ObservationTask, StateError> {
        self.expire(now);
        let stored = self.tasks.get_mut(&id).ok_or(StateError::NotFound)?;
        if stored.network != admission.network_id
            || stored.task.source_node_id != admission.node_id
            || stored.task.instance_id != admission.instance_id
            || result.instance_id != admission.instance_id
            || result.configuration != stored.task.configuration
            || !admission.configurations.contains(&result.configuration)
            || admission.lease_expires_at <= now
            || stored.task.expires_at <= now
            || stored.subscribers.is_empty()
        {
            return Err(StateError::Unauthorized);
        }
        if !stored.paths.contains(&result.path)
            || !admission.paths.contains(&result.path)
            || result.path_segment.get() == 0
        {
            return Err(StateError::Invalid);
        }
        if let ObservationPath::Explicit { path } = &stored.task.selection
            && *path != result.path
        {
            return Err(StateError::Invalid);
        }
        if let Some(segment) = stored.task.path_segment
            && (result.path_segment < segment
                || result.path_segment == segment
                    && stored.task.path.as_ref() != Some(&result.path))
        {
            return Err(StateError::Conflict);
        }
        let sample = &result.sample;
        if sample.window_start >= sample.window_end
            || sample.window_end > now + Duration::seconds(30)
            || sample.window_end > stored.task.expires_at
            || now - sample.window_start > Duration::minutes(2)
        {
            return Err(StateError::Invalid);
        }
        if let Some(probe) = &sample.probe {
            if probe.outcome == meshway_model::state::ProbeOutcome::Pending && !sample.partial {
                return Err(StateError::Invalid);
            }
            if probe.measurement_id != sample.measurement_id
                || probe.path != result.path
                || probe.publication_id != result.configuration.execution_publication
            {
                return Err(StateError::Invalid);
            }
            crate::state::validation::validate_probe(admission, probe, now)?;
        }
        if let Some(counts) = &sample.probe_counts {
            let total = u64::from(counts.replied_within_deadline)
                + u64::from(counts.timeout)
                + u64::from(counts.pending);
            if total != u64::from(counts.sent) || sample.partial != (counts.pending > 0) {
                return Err(StateError::Invalid);
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
            return Err(StateError::Invalid);
        }
        let digest = crate::resources::content_digest(
            &serde_json::to_vec(&result).map_err(|_| StateError::Invalid)?,
        );
        if let Some(previous) = stored.seen.get(&sample.measurement_id) {
            if previous.digest == digest {
                return Ok(stored.task.clone());
            }
            let old = stored.task.latest.as_ref().ok_or(StateError::Conflict)?;
            if !previous.partial
                || sample.version <= previous.version
                || old.measurement_id != sample.measurement_id
                || old.window_start != sample.window_start
                || old.window_end != sample.window_end
                || stored.task.path_segment != Some(result.path_segment)
            {
                return Err(StateError::Conflict);
            }
            if let (Some(old), Some(new)) = (&old.probe_counts, &sample.probe_counts)
                && (old.sent != new.sent
                    || old.replied_within_deadline > new.replied_within_deadline
                    || old.timeout > new.timeout)
            {
                return Err(StateError::Conflict);
            }
        }
        if !stored.seen.contains_key(&sample.measurement_id) && stored.seen.len() >= 4096 {
            return Err(StateError::ObservationLimit);
        }
        if stored.task.latest.as_ref().is_some_and(|previous| {
            previous.measurement_id != sample.measurement_id
                && previous.window_end > sample.window_start
        }) {
            return Err(StateError::Conflict);
        }
        stored.seen.insert(
            sample.measurement_id,
            SeenWindow {
                digest,
                version: sample.version,
                partial: sample.partial,
            },
        );
        stored.task.path = Some(result.path);
        stored.task.path_segment = Some(result.path_segment);
        stored.task.latest = Some(result.sample);
        stored.task.phase = ObservationPhase::Running;
        Ok(stored.task.clone())
    }
    pub fn expire(&mut self, now: DateTime<Utc>) {
        for task in self.tasks.values_mut() {
            task.subscribers.retain(|_, s| s.expires_at > now);
            if task.subscribers.is_empty() && task.task.phase != ObservationPhase::Cancelled {
                task.task.phase = ObservationPhase::Expired;
            }
        }
        self.tasks
            .retain(|_, task| task.task.expires_at + Duration::minutes(1) > now);
    }
    fn authorize(
        task: &mut StoredTask,
        admission: &TaskAdmission,
        now: DateTime<Utc>,
    ) -> Result<(), StateError> {
        let subscriber = task
            .subscribers
            .get(&admission.session_id)
            .ok_or(StateError::NotFound)?;
        if task.network != admission.report.network_id
            || subscriber.expires_at <= now
            || admission.session_expires_at <= now
            || subscriber.authorization_digest != admission.authorization_digest
            || task.task.instance_id != admission.report.instance_id
            || !admission
                .report
                .configurations
                .contains(&task.task.configuration)
        {
            task.subscribers.remove(&admission.session_id);
            return Err(StateError::Unauthorized);
        }
        Ok(())
    }
}

fn subscriber_view(stored: &StoredTask, session: SessionId) -> Result<ObservationTask, StateError> {
    let subscriber = stored
        .subscribers
        .get(&session)
        .ok_or(StateError::NotFound)?;
    let mut task = stored.task.clone();
    task.subscriber_expires_at = Some(subscriber.expires_at);
    task.renew_after = subscriber.renew_after;
    Ok(task)
}
