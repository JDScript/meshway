//! Bounded source scheduling over the host's existing SecureNode. This owner
//! has no sockets, HTTP client, flow table, keys, or background tasks.
//!
//! A task is bound locally: its paths are the source's own `icmpEcho`
//! decision on the installed netmap, as for a first packet, with no Server
//! round trip. A permitted pair without a path every Node can
//! observe reports `Unsupported`; a denied pair does not bind.
#![forbid(unsafe_code)]

mod measurement;
#[cfg(test)]
mod tests;

use chrono::{DateTime, Utc};
use measurement::Measurement;
use meshway_authority::ExecutionSnapshot;
use meshway_configuration::execution::Carrier;
use meshway_dataplane::secure::{
    CircuitGrant, CircuitRole, CircuitState, PendingFrame, SecureNode, Target,
};
use meshway_model::observations::{MetricKind, ProbeChannel};
use meshway_model::{Digest, EventId, NodeId, ObservationId, Sequence};
use meshway_protocol::observations::*;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

pub const MAX_TASKS: usize = 128;
pub const DEADLINE: Duration = Duration::from_secs(5);
/// Acquisition/handshake waiting is not a measured probe. This budget never
/// grants execution: every attempt still requires the current snapshot/permit.
pub const SETUP_TIMEOUT: Duration = Duration::from_secs(60);
pub const MIN_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("observation task does not match current execution authority")]
    Binding,
    #[error("observation capacity exhausted")]
    Capacity,
    #[error("observation random identity could not be generated")]
    Random,
}

/// A host sends this frame through its existing Manager and returns the exact
/// completion. Handshake actions carry no measurement ID and never count sent.
pub struct Action {
    pub observation_id: ObservationId,
    pub measurement_id: Option<EventId>,
    pub frame: PendingFrame,
    pub cancellation: tokio_util::sync::CancellationToken,
}

/// Immutable publication for both local subscribers and the Server gateway.
#[derive(Clone)]
pub struct Output {
    pub task: ObservationTask,
    pub snapshot: ExecutionSnapshot,
    pub result: ObservationResult,
    invalidated: tokio_util::sync::CancellationToken,
}
impl Output {
    pub fn is_current(&self) -> bool {
        !self.invalidated.is_cancelled()
            && self.snapshot.now_ms().is_ok_and(|now| {
                u64::try_from(self.task.expires_at.timestamp_millis())
                    .is_ok_and(|expiry| now < expiry)
            })
    }
}

struct Task {
    task: ObservationTask,
    snapshot: ExecutionSnapshot,
    path: Vec<NodeId>,
    segment: Sequence,
    circuit: Option<[u8; 32]>,
    preparing: Instant,
    next: Instant,
    setup_wake: Option<Instant>,
    measurement: Option<Measurement>,
    output: Option<Output>,
    paused: bool,
    last_end: Option<DateTime<Utc>>,
    cancellation: tokio_util::sync::CancellationToken,
    sending: bool,
    publication_invalidated: tokio_util::sync::CancellationToken,
    connecting: bool,
    setup_failed: bool,
    preparation_stage: Option<&'static str>,
    /// False when the decision permits the pair but no path is observable.
    supported: bool,
}

impl Task {
    fn grant(&self) -> CircuitGrant {
        CircuitGrant {
            purpose: meshway_dataplane::Purpose::Diagnostic,
            target: Target::Mesh {
                node: self.task.target_node_id,
            },
        }
    }
}

fn preparation_stage(task: &mut Task, stage: &'static str) {
    if task.preparation_stage == Some(stage) {
        return;
    }
    task.preparation_stage = Some(stage);
    tracing::info!(observation = %task.task.id,
        netmap_serial = task.snapshot.reference().serial.get(),
        source_instance = %task.task.instance_id, stage,
        first_hop = ?task.path.get(1), hops = task.path.len().saturating_sub(1),
        elapsed_ms = task.preparing.elapsed().as_millis() as u64,
        "Diagnostic preparation stage changed");
}

impl Drop for Task {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.publication_invalidated.cancel();
    }
}

#[derive(Default)]
pub struct Owner {
    tasks: BTreeMap<ObservationId, Task>,
}

impl Owner {
    /// Replace source tasks atomically after checking every binding. Relays and
    /// terminals need no source task; SecureNode admits their current permits.
    pub fn synchronize(
        &mut self,
        snapshot: ExecutionSnapshot,
        tasks: &[ObservationTask],
    ) -> Result<(), Error> {
        if tasks.len() > MAX_TASKS {
            return Err(Error::Capacity);
        }
        let mut plans = Vec::with_capacity(tasks.len());
        for (index, task) in tasks.iter().enumerate() {
            let (path, supported) = bind(&snapshot, task)?;
            if tasks[..index].iter().any(|old| old.id == task.id) {
                return Err(Error::Binding);
            }
            let mut segment = match (&task.path, task.path_segment) {
                (Some(old), Some(segment)) if old == &path => segment,
                (_, Some(segment)) => segment.checked_next().map_err(|_| Error::Binding)?,
                _ => Sequence::new(1).map_err(|_| Error::Binding)?,
            };
            if let Some(old) = self.tasks.get(&task.id) {
                let reset = !old.snapshot.same_netmap(&snapshot)
                    || old.path != path
                    || (old.task.phase == ObservationPhase::Running
                        && task.phase == ObservationPhase::Preparing);
                segment = segment.max(if reset {
                    old.segment.checked_next().map_err(|_| Error::Binding)?
                } else {
                    old.segment
                });
            }
            plans.push((task, path, segment, supported));
        }
        snapshot.ensure_current().map_err(|_| Error::Binding)?;
        self.tasks
            .retain(|id, _| tasks.iter().any(|task| task.id == *id));
        for (task, path, segment, supported) in plans {
            let now = Instant::now();
            if let Some(old) = self.tasks.get_mut(&task.id)
                && old.snapshot.same_netmap(&snapshot)
                && old.snapshot.ensure_current().is_ok()
                && !old.publication_invalidated.is_cancelled()
                && old.path == path
                && old.task.service == task.service
                && old.task.metrics == task.metrics
                && old.task.interval_ms == task.interval_ms
                && old.segment == segment
                && old.supported == supported
            {
                old.task = task.clone();
                // Do not renew an in-flight measurement or its publication.
                old.snapshot = snapshot.clone();
                continue;
            }
            let last_end = task
                .latest
                .as_ref()
                .map(|s| s.window_end)
                .into_iter()
                .chain(self.tasks.get(&task.id).and_then(|old| old.last_end))
                .max();
            let preparing = self
                .tasks
                .get(&task.id)
                .filter(|old| old.task.instance_id == task.instance_id)
                .map_or(now, |old| old.preparing);
            // Probe pacing survives even an uncertain predecessor admission.
            // A genuinely new task has no prior probe to pace: setup starts on
            // installation/channel events, not after a synthetic first second.
            let next = self
                .tasks
                .get(&task.id)
                .map_or(now, |old| old.next.max(now));
            self.tasks.insert(
                task.id,
                Task {
                    task: task.clone(),
                    snapshot: snapshot.clone(),
                    path,
                    segment,
                    circuit: None,
                    preparing,
                    next,
                    setup_wake: Some(now),
                    measurement: None,
                    output: None,
                    paused: false,
                    last_end,
                    cancellation: tokio_util::sync::CancellationToken::new(),
                    sending: false,
                    publication_invalidated: tokio_util::sync::CancellationToken::new(),
                    connecting: true,
                    setup_failed: false,
                    preparation_stage: None,
                    supported,
                },
            );
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.tasks.clear();
    }

    /// Retain only bounded planning clocks across a handoff. Pending I/O and
    /// publications are cancelled; these records cannot execute on an old grant.
    pub fn suspend(&mut self) {
        for task in self.tasks.values_mut() {
            suspend(task);
        }
    }

    pub fn preparing(&self, id: ObservationId) -> bool {
        self.tasks
            .get(&id)
            .is_none_or(|task| !current(task) || task.connecting)
    }

    pub fn set_paused(&mut self, paused: &std::collections::BTreeSet<ObservationId>) {
        for (id, task) in &mut self.tasks {
            let was_paused = task.paused;
            task.paused = paused.contains(id);
            if task.paused {
                if !was_paused {
                    task.next = task.next.max(Instant::now() + MIN_INTERVAL);
                }
                task.cancellation.cancel();
                task.sending = false;
                task.measurement = None;
            } else if task.cancellation.is_cancelled() {
                task.cancellation = tokio_util::sync::CancellationToken::new();
            }
        }
    }

    pub fn wake_at(&self) -> Option<Instant> {
        self.tasks
            .values()
            .filter(|task| {
                current(task) && !task.paused && !task.setup_failed && task.output.is_none()
            })
            .map(|task| {
                task.measurement.as_ref().map_or(
                    if task.sending {
                        task.preparing + SETUP_TIMEOUT
                    } else if task.connecting {
                        task.setup_wake
                            .unwrap_or(task.preparing + SETUP_TIMEOUT)
                            .max(task.next)
                            .min(task.preparing + SETUP_TIMEOUT)
                    } else {
                        task.next
                    },
                    |m| {
                        if m.send_completed {
                            m.deadline
                        } else {
                            m.deadline.max(Instant::now() + Duration::from_millis(100))
                        }
                    },
                )
            })
            .min()
    }

    /// At most one action per task. The host must bound its own sending queue;
    /// `budget` reserves that capacity before any measurement is created.
    pub fn actions(
        &mut self,
        secure: &mut SecureNode,
        carriers: &BTreeMap<NodeId, Carrier>,
        budget: usize,
    ) -> Vec<Action> {
        let now = Instant::now();
        let mut actions = Vec::new();
        let ready = secure.ready_circuits().unwrap_or_default();
        for (id, task) in &mut self.tasks {
            if !current(task) {
                suspend(task);
                continue;
            }
            if task
                .output
                .as_ref()
                .is_some_and(|output| !output.is_current())
            {
                task.output = None;
            }
            if task.paused || task.setup_failed || task.output.is_some() {
                continue;
            }
            if task.connecting {
                preparing_failure(task, now);
                if task.setup_failed {
                    continue;
                }
            }
            if let Some(measurement) = &task.measurement {
                if now >= measurement.deadline && measurement.send_completed {
                    finish(task, now);
                }
                continue;
            }
            if task.sending {
                preparing_failure(task, now);
                continue;
            }
            if now < task.next {
                continue;
            }
            if actions.len() >= budget {
                preparing_failure(task, now);
                task.next = now + Duration::from_millis(100);
                task.setup_wake = Some(task.next);
                continue;
            }
            let unsupported = !task.supported
                || !task
                    .task
                    .metrics
                    .iter()
                    .any(|m| matches!(m, MetricKind::Rtt | MetricKind::ProbeLoss));
            if unsupported {
                task.output = Some(output(
                    task,
                    ObservationUpdate::Phase {
                        phase: ReportedObservationPhase::Unsupported,
                    },
                ));
                continue;
            }
            let Some(carrier) = task.path.get(1).and_then(|node| carriers.get(node)) else {
                preparation_stage(task, "first_hop_channel");
                task.connecting = true;
                preparing_failure(task, now);
                // Manager emits a carrier event when authenticated adjacency
                // becomes available. Only the original setup deadline remains.
                task.setup_wake = None;
                continue;
            };
            if task.circuit.is_none() {
                // SecureNode alone decides which crypto survived its update.
                // This lookup reuses its authority; it never migrates a sample.
                let grant = task.grant();
                task.circuit = ready
                    .iter()
                    .find(|circuit| {
                        circuit.grant == grant
                            && circuit.path == task.path
                            && circuit.role == CircuitRole::Origin
                    })
                    .map(|circuit| circuit.id);
            }
            let info = task
                .circuit
                .and_then(|id| secure.circuit_info(id).ok().flatten());
            match info {
                Some(info) if info.state == CircuitState::Established => {
                    preparation_stage(task, "path_established");
                    let measurement = match Measurement::new(task, now) {
                        Ok(measurement) => measurement,
                        Err(_) => {
                            task.output = Some(output(
                                task,
                                ObservationUpdate::Phase {
                                    phase: ReportedObservationPhase::Failed,
                                },
                            ));
                            continue;
                        }
                    };
                    let mid = measurement.id;
                    let frame = secure.probe(
                        info.id,
                        mid,
                        measurement.challenge,
                        measurement.deadline,
                        carrier,
                    );
                    match frame {
                        Ok(frame) => {
                            task.measurement = Some(measurement);
                            task.connecting = false;
                            task.preparing = now;
                            task.next = now
                                + Duration::from_millis(u64::from(task.task.interval_ms))
                                    .max(MIN_INTERVAL);
                            task.sending = true;
                            actions.push(Action {
                                observation_id: *id,
                                measurement_id: Some(mid),
                                frame,
                                cancellation: task.cancellation.clone(),
                            });
                        }
                        Err(_) => {
                            task.connecting = true;
                            preparing_failure(task, now);
                            task.next = now + MIN_INTERVAL;
                            task.setup_wake = Some(task.next);
                        }
                    }
                }
                Some(info) if info.state == CircuitState::Handshaking => {
                    preparation_stage(task, "path_handshake");
                    task.connecting = true;
                    match secure.retry_handshake(info.id) {
                        Ok(Some(frame)) => {
                            tracing::debug!(observation = %id, stage = "path_handshake", "Diagnostic handshake retry dispatched");
                            task.sending = true;
                            actions.push(Action {
                                observation_id: *id,
                                measurement_id: None,
                                frame,
                                cancellation: task.cancellation.clone(),
                            });
                        }
                        Ok(None) => {}
                        Err(_) => {
                            // Never re-sign/re-scope cached INIT. Once its exact
                            // retry is inadmissible, a fresh authorized handshake
                            // may be attempted within the same setup budget.
                            secure.close(info.id);
                            task.circuit = None;
                        }
                    }
                    preparing_failure(task, now);
                    task.setup_wake = secure
                        .handshake_retry_at(info.id)
                        .ok()
                        .flatten()
                        .or_else(|| task.circuit.is_none().then_some(now));
                }
                _ => {
                    preparation_stage(task, "path_handshake");
                    task.connecting = true;
                    task.circuit = None;
                    match secure.start_grant(task.grant(), task.path.clone(), carrier) {
                        Ok(frame) => {
                            task.sending = true;
                            task.circuit = Some(frame.circuit());
                            task.setup_wake =
                                secure.handshake_retry_at(frame.circuit()).ok().flatten();
                            actions.push(Action {
                                observation_id: *id,
                                measurement_id: None,
                                frame,
                                cancellation: task.cancellation.clone(),
                            });
                        }
                        Err(_) => {
                            preparing_failure(task, now);
                            task.setup_wake = None;
                        }
                    }
                }
            }
        }
        actions
    }

    /// Completion and reply can arrive in either order. Only transport-admitted
    /// sends count; an uncertain/local I/O failure is never network packet loss.
    fn sent(
        &mut self,
        id: ObservationId,
        measurement_id: EventId,
        receipt: Result<(ProbeChannel, Instant), ()>,
    ) {
        let Some(task) = self
            .tasks
            .get_mut(&id)
            .filter(|task| current(task) && !task.paused)
        else {
            return;
        };
        let Some(measurement) = &mut task.measurement else {
            return;
        };
        if measurement.id != measurement_id || measurement.send_completed {
            return;
        }
        measurement.send_completed = true;
        match receipt {
            Ok(receipt) if task.path.get(1) == Some(&receipt.0.peer_node_id) => {
                task.next = task.next.max(
                    receipt.1
                        + Duration::from_millis(u64::from(task.task.interval_ms)).max(MIN_INTERVAL),
                );
                measurement.receipt = Some(receipt);
            }
            Ok(_) => measurement.local_failure = true,
            Err(()) => measurement.local_failure = true,
        }
        if measurement.local_failure
            || measurement.reply.is_some()
            || Instant::now() >= measurement.deadline
        {
            finish(task, Instant::now());
        }
    }

    pub fn complete_send(
        &mut self,
        id: ObservationId,
        measurement_id: Option<EventId>,
        receipt: Result<(ProbeChannel, Instant), ()>,
    ) {
        if let Some(task) = self.tasks.get_mut(&id) {
            task.sending = false;
        }
        if let Some(measurement_id) = measurement_id {
            self.sent(id, measurement_id, receipt);
        }
    }

    pub fn reply(
        &mut self,
        circuit: [u8; 32],
        measurement_id: EventId,
        challenge: Digest,
        channel: ProbeChannel,
        received_at: Instant,
    ) {
        let Some(task) = self
            .tasks
            .values_mut()
            .find(|task| task.circuit == Some(circuit) && current(task) && !task.paused)
        else {
            return;
        };
        let Some(measurement) = &mut task.measurement else {
            return;
        };
        if measurement.id != measurement_id
            || measurement.challenge != challenge
            || measurement.reply.is_some()
            || received_at > measurement.deadline
            || task.path.get(1) != Some(&channel.peer_node_id)
        {
            return;
        }
        measurement.reply = Some((channel, received_at));
        if measurement.send_completed {
            finish(task, Instant::now());
        }
    }

    /// A retained result pauses this task until every host consumer accepts it.
    pub fn outputs(&self) -> impl Iterator<Item = &Output> {
        self.tasks
            .values()
            .filter(|task| current(task) && !task.paused)
            .filter_map(|task| task.output.as_ref().filter(|output| output.is_current()))
    }

    pub fn acknowledge(&mut self, id: ObservationId) {
        if let Some(task) = self.tasks.get_mut(&id) {
            task.output = None;
            task.preparing = Instant::now();
        }
    }
}

fn output(task: &Task, result: ObservationUpdate) -> Output {
    Output {
        task: task.task.clone(),
        snapshot: task.snapshot.clone(),
        result: ObservationResult {
            instance_id: task.task.instance_id,
            netmap: task.snapshot.reference(),
            result,
        },
        invalidated: task.publication_invalidated.clone(),
    }
}

fn current(task: &Task) -> bool {
    if task.publication_invalidated.is_cancelled() {
        return false;
    }
    // New content invalidates this snapshot; the host binds again.
    task.snapshot.now_ms().is_ok_and(|now| {
        i64::try_from(now).is_ok_and(|now| task.task.expires_at.timestamp_millis() > now)
    })
}

/// Task fields independent of the netmap.
fn valid_task(task: &ObservationTask, now: u64) -> Result<(), Error> {
    if task.expires_at.timestamp_millis() <= i64::try_from(now).map_err(|_| Error::Binding)?
        || !matches!(
            task.phase,
            ObservationPhase::Preparing | ObservationPhase::Running
        )
        || !(1000..=30_000).contains(&task.interval_ms)
        || task.metrics.is_empty()
        || task
            .metrics
            .iter()
            .enumerate()
            .any(|(i, metric)| task.metrics[..i].contains(metric))
        || task
            .unsupported_metrics
            .iter()
            .any(|metric| !task.metrics.contains(metric))
        || (task.path.is_some() && task.path_segment.is_none())
        || (task.phase == ObservationPhase::Running && task.path.is_none())
        || (task.phase == ObservationPhase::Preparing && task.path.is_some())
        || task.path_segment.is_some_and(|segment| segment.get() == 0)
    {
        return Err(Error::Binding);
    }
    Ok(())
}

/// Bind on the source's own netmap decision.
fn bind(
    snapshot: &ExecutionSnapshot,
    task: &ObservationTask,
) -> Result<(Vec<NodeId>, bool), Error> {
    use meshway_configuration::execution::ObservationServiceScope;
    let netmap = snapshot.netmap().map_err(|_| Error::Binding)?;
    let now = snapshot.now_ms().map_err(|_| Error::Binding)?;
    valid_task(task, now)?;
    if task.netmap != snapshot.reference()
        || task.source_node_id != netmap.netmap.node_id
        || task.target_node_id == task.source_node_id
        || snapshot
            .instance()
            .is_some_and(|instance| instance.instance_id != task.instance_id)
    {
        return Err(Error::Binding);
    }
    // Probe circuits are admitted for icmpEcho; other scopes are unsupported.
    if task.service != ObservationServiceScope::IcmpEcho {
        return Ok((Vec::new(), false));
    }
    let explicit = match &task.selection {
        ObservationPath::Auto => None,
        ObservationPath::Explicit { path } => Some(path.as_slice()),
    };
    let paths = match meshway_dataplane::secure::diagnostic_paths(
        snapshot,
        task.target_node_id,
        explicit,
    ) {
        Ok(paths) => paths,
        Err(meshway_dataplane::secure::Error::Unsupported) => return Ok((Vec::new(), false)),
        Err(_) => return Err(Error::Binding),
    };
    if task.path.as_ref().is_some_and(|path| !paths.contains(path)) {
        return Err(Error::Binding);
    }
    let path = paths.into_iter().next().ok_or(Error::Binding)?;
    snapshot.ensure_current().map_err(|_| Error::Binding)?;
    Ok((path, true))
}

fn preparing_failure(task: &mut Task, now: Instant) {
    if now < task.preparing + SETUP_TIMEOUT || task.setup_failed {
        return;
    }
    task.setup_failed = true;
    tracing::info!(observation = %task.task.id,
        netmap_serial = task.snapshot.reference().serial.get(),
        stage = task.preparation_stage, elapsed_ms = now.duration_since(task.preparing).as_millis() as u64,
        "Diagnostic preparation budget expired");
    task.cancellation.cancel();
    task.sending = false;
    task.output = Some(output(
        task,
        ObservationUpdate::Phase {
            phase: ReportedObservationPhase::Failed,
        },
    ));
}

fn suspend(task: &mut Task) {
    task.cancellation.cancel();
    task.publication_invalidated.cancel();
    task.measurement = None;
    task.output = None;
    task.sending = false;
    task.connecting = true;
}

fn finish(task: &mut Task, now: Instant) {
    let Some(measurement) = task.measurement.take() else {
        return;
    };
    let sample = measurement.finish(task, now);
    task.last_end = Some(sample.window_end);
    task.output = Some(output(
        task,
        ObservationUpdate::Sample {
            path: task.path.clone(),
            path_segment: task.segment,
            sample: Box::new(sample),
        },
    ));
}
