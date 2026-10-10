//! Host adapter for the shared observation executor. No HTTP or cipher owner.
//! Server tasks (Console observations) and local `meshway ping` share one
//! executor; a ping is a local task decided on the installed netmap alone,
//! never published, and its results go back to IPC only.
use super::Request;
use crate::runtime::PingRequest;
use crate::runtime::ping::Progress;
use meshway_authority::ExecutionSnapshot;
use meshway_connectivity::{LinkFact, Manager};
use meshway_dataplane::secure::{PendingFrame, SecureNode};
use meshway_diagnostics::{Output, Owner};
use meshway_ipc::{PingError, PingEvent};
use meshway_model::{
    EventId, NodeId, ObservationId, ObservationSubscriptionId, observations::ProbeChannel,
};
use meshway_protocol::{
    NodeObservationTarget, NodeObservationTargetView,
    observations::{ObservationPath, ObservationPhase, ObservationTask, ObservationUpdate},
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

struct Local {
    observation: ObservationId,
    task: ObservationTask,
    request: PingRequest,
    sent: u32,
    success: u32,
    timeout: u32,
    failed: u32,
    samples: u16,
    preparing_since: Instant,
    next_progress: Instant,
    progress: tokio::sync::watch::Sender<Progress>,
}
impl Local {
    fn lease_ended(&self) -> bool {
        chrono::Utc::now() >= self.task.expires_at
    }
}
struct Publication {
    output: Output,
    pending: Option<oneshot::Receiver<Result<(), ()>>>,
    retry: Instant,
}
struct Sent {
    observation: Option<ObservationId>,
    measurement: Option<EventId>,
    cancellation: tokio_util::sync::CancellationToken,
    result: Result<meshway_connectivity::SendReceipt, ()>,
}
#[derive(Default)]
pub(super) struct Diagnostics {
    pub owner: Owner,
    locals: BTreeMap<ObservationSubscriptionId, Local>,
    publications: BTreeMap<ObservationId, Publication>,
    sends: tokio::task::JoinSet<Sent>,
    epoch: tokio_util::sync::CancellationToken,
    pending_plan: Option<super::ObservationPlan>,
    /// Server tasks of the installed netmap.
    server: Vec<ObservationTask>,
    /// The snapshot and task set the executor was last synchronized with.
    synchronized: Option<ExecutionSnapshot>,
    dirty: bool,
    paused: std::collections::BTreeSet<ObservationId>,
    effective_pauses: std::collections::BTreeSet<ObservationId>,
}

fn random_uuid() -> Option<uuid::Uuid> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).ok()?;
    Some(uuid::Builder::from_random_bytes(bytes).into_uuid())
}

/// Resolve a ping target on the netmap: a configured hostname or Mesh FQDN
/// from its visible projection, or an exact Mesh IP. MagicDNS enablement only
/// controls system DNS handling; diagnostic admission is checked separately.
fn resolve(
    snapshot: &ExecutionSnapshot,
    target: &NodeObservationTarget,
) -> Result<NodeObservationTargetView, PingError> {
    let netmap = &snapshot.netmap().map_err(|_| PingError::NotRunning)?.netmap;
    let configuration = &netmap.projection.configuration;
    let domain = &configuration.dns.domain;
    let found = configuration.nodes.iter().find(|(id, node)| {
        **id != netmap.node_id
            && netmap.projection.inventory.contains_key(id)
            && match target {
                NodeObservationTarget::Name { name } => {
                    let name = name.trim_end_matches('.').to_ascii_lowercase();
                    let fqdn = meshway_configuration::node_dns_name(&node.hostname, domain).ok();
                    name == node.hostname || fqdn.as_deref() == Some(name.as_str())
                }
                NodeObservationTarget::MeshIp { address } => address
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| match ip {
                        std::net::IpAddr::V4(ip) => node.addresses.ipv4 == Some(ip),
                        std::net::IpAddr::V6(ip) => node.addresses.ipv6 == Some(ip),
                    }),
            }
    });
    let (id, node) = found.ok_or(PingError::TargetNotFound)?;
    Ok(NodeObservationTargetView {
        node_id: *id,
        hostname: node.hostname.clone(),
        addresses: node.addresses.clone(),
    })
}

impl Diagnostics {
    pub fn plan(&mut self, plan: Option<super::ObservationPlan>) {
        if let Some(plan) = &plan {
            for task in &plan.tasks {
                tracing::debug!(observation = %task.id,
                    netmap_serial = plan.snapshot.reference().serial.get(),
                    stage = "installation_pending", "Diagnostic plan awaits matching installation");
            }
        }
        if plan.is_none() {
            self.server.clear();
            self.dirty = true;
        }
        self.pending_plan = plan;
    }

    /// Server tasks executing toward `target` on a diagnostic circuit.
    pub fn observations_toward(&self, target: NodeId) -> Vec<ObservationId> {
        self.server
            .iter()
            .filter(|task| task.target_node_id == target)
            .map(|task| task.id)
            .collect()
    }

    /// Bind the executor to the installed netmap: Server tasks assigned on
    /// this content plus every live local ping.
    pub fn synchronize(&mut self, installed: &ExecutionSnapshot) {
        if let Some(plan) = &self.pending_plan {
            if plan.snapshot.ensure_current().is_err() {
                self.pending_plan = None;
            } else if plan.snapshot.same_netmap(installed) {
                let plan = self.pending_plan.take().expect("matching plan");
                self.server = plan.tasks;
                self.dirty = true;
            }
        }
        if !self.dirty
            && self.synchronized.as_ref().is_some_and(|old| {
                old.same_netmap(installed) && old.instance() == installed.instance()
            })
        {
            return;
        }
        self.dirty = false;
        let reference = installed.reference();
        let instance = installed.instance();
        self.server.retain(|task| {
            task.netmap == reference
                && instance
                    .as_ref()
                    .is_some_and(|current| current.instance_id == task.instance_id)
        });
        // Local measurements are re-admitted by this Node's current netmap;
        // an HTTP instance replacement does not end the IPC caller's request.
        for local in self.locals.values_mut() {
            local.task.netmap = reference;
            if let Some(instance) = &instance {
                local.task.instance_id = instance.instance_id;
            }
        }
        let mut tasks = self.server.clone();
        tasks.extend(
            self.locals
                .values()
                .filter(|local| !local.request.cancellation.is_cancelled())
                .map(|local| local.task.clone()),
        );
        tracing::debug!(
            netmap_serial = installed.reference().serial.get(),
            tasks = tasks.len(),
            "Synchronizing observation tasks with installed netmap"
        );
        if self.owner.synchronize(installed.clone(), &tasks).is_err() {
            self.reject_tasks();
        }
        self.synchronized = Some(installed.clone());
        self.apply_pauses();
    }

    /// Start a local ping on the installed netmap. Denied and unsupported
    /// targets fail at once; otherwise preparation begins locally.
    pub fn ping(
        &mut self,
        snapshot: Option<&ExecutionSnapshot>,
        session: crate::runtime::PingSession,
    ) {
        let crate::runtime::PingSession {
            request, progress, ..
        } = session;
        if request.cancellation.is_cancelled() || request.events.is_closed() {
            return;
        }
        let fail = |request: &PingRequest, code| {
            let _ = request.events.try_send(PingEvent::Failed { code });
            request.cancellation.cancel();
        };
        let Some(snapshot) = snapshot.filter(|s| s.ensure_current().is_ok()) else {
            return fail(&request, PingError::NotRunning);
        };
        if self.locals.len() >= 8 {
            return fail(&request, PingError::Busy);
        }
        let target = match resolve(snapshot, &request.target) {
            Ok(target) => target,
            Err(code) => return fail(&request, code),
        };
        match meshway_dataplane::secure::diagnostic_paths(snapshot, target.node_id, None) {
            Ok(_) => {}
            Err(meshway_dataplane::secure::Error::Unsupported) => {
                return fail(&request, PingError::Unsupported);
            }
            Err(meshway_dataplane::secure::Error::Policy) => {
                return fail(&request, PingError::PermissionDenied);
            }
            Err(_) => return fail(&request, PingError::NotRunning),
        }
        let (Some(subscription), Some(observation), Some(instance)) =
            (random_uuid(), random_uuid(), random_uuid())
        else {
            return fail(&request, PingError::Failed);
        };
        let (Ok(subscription), Ok(observation)) = (
            ObservationSubscriptionId::new(subscription),
            ObservationId::new(observation),
        ) else {
            return fail(&request, PingError::Failed);
        };
        let instance_id = match snapshot.instance() {
            Some(instance) => instance.instance_id,
            None => match meshway_model::InstanceId::new(instance) {
                Ok(id) => id,
                Err(_) => return fail(&request, PingError::Failed),
            },
        };
        let Ok(netmap) = snapshot.netmap() else {
            return fail(&request, PingError::NotRunning);
        };
        let now = chrono::Utc::now();
        let budget =
            meshway_diagnostics::SETUP_TIMEOUT + Duration::from_secs(u64::from(request.count) + 10);
        let expires_at = now + chrono::Duration::from_std(budget).unwrap_or_default();
        let task = ObservationTask {
            id: observation,
            source_node_id: netmap.netmap.node_id,
            target_node_id: target.node_id,
            service: meshway_configuration::execution::ObservationServiceScope::IcmpEcho,
            selection: ObservationPath::Auto,
            path: None,
            path_segment: None,
            netmap: snapshot.reference(),
            instance_id,
            metrics: vec![
                meshway_model::observations::MetricKind::Rtt,
                meshway_model::observations::MetricKind::ProbeLoss,
            ],
            interval_ms: 1000,
            phase: ObservationPhase::Preparing,
            unsupported_metrics: Vec::new(),
            expires_at,
            renew_after: expires_at,
            latest: None,
            subscriber_expires_at: None,
            subscription_id: None,
        };
        tracing::info!(request = %request.id, observation = %observation, target = %target.node_id,
            "Local ping accepted on the installed netmap");
        if request
            .events
            .try_send(PingEvent::Accepted {
                subscription_id: subscription,
                observation_id: observation,
                target,
            })
            .is_err()
        {
            request.cancellation.cancel();
            return;
        }
        self.locals.insert(
            subscription,
            Local {
                observation,
                task,
                request,
                sent: 0,
                success: 0,
                timeout: 0,
                failed: 0,
                samples: 0,
                preparing_since: Instant::now(),
                next_progress: Instant::now(),
                progress,
            },
        );
        self.dirty = true;
    }

    fn pause_causes(&self) -> std::collections::BTreeSet<ObservationId> {
        let mut paused = self.paused.clone();
        paused.extend(
            self.locals
                .values()
                .filter(|local| {
                    !local.lease_ended()
                        && (local.request.cancellation.is_cancelled()
                            || local.request.events.is_closed())
                })
                .map(|local| local.observation),
        );
        paused
    }

    fn apply_pauses(&mut self) {
        let paused = self.pause_causes();
        if paused != self.effective_pauses {
            tracing::debug!(paused = ?paused, "Observation cancellation barriers changed");
            self.effective_pauses = paused.clone();
        }
        self.owner.set_paused(&paused);
    }
    pub fn reject_tasks(&mut self) {
        self.owner.clear();
        self.publications.clear();
        self.server.clear();
        for local in self.locals.values() {
            let _ = local.request.events.try_send(PingEvent::Failed {
                code: PingError::Protocol,
            });
            local.request.cancellation.cancel();
        }
    }

    pub async fn stop(&mut self) {
        self.epoch.cancel();
        self.owner.clear();
        self.publications.clear();
        for local in self.locals.values() {
            let _ = local.request.events.try_send(PingEvent::Failed {
                code: PingError::Stopped,
            });
            local.request.cancellation.cancel();
        }
        self.locals.clear();
        self.pending_plan = None;
        self.server.clear();
        self.synchronized = None;
        self.dirty = true;
        self.paused.clear();
        self.effective_pauses.clear();
        while self.sends.join_next().await.is_some() {}
        self.epoch = tokio_util::sync::CancellationToken::new();
    }

    pub fn invalidate(&mut self) {
        self.epoch.cancel();
        self.epoch = tokio_util::sync::CancellationToken::new();
        self.owner.suspend();
        for local in self.locals.values() {
            local
                .progress
                .send_if_modified(|progress| progress.replace(Progress::Setup));
        }
        self.publications.clear();
        self.synchronized = None;
        self.dirty = true;
    }

    pub fn wake_at(&self) -> Option<tokio::time::Instant> {
        let poll = (!self.locals.is_empty() || !self.publications.is_empty())
            .then(|| Instant::now() + Duration::from_millis(100));
        self.owner
            .wake_at()
            .into_iter()
            .chain(poll)
            .min()
            .map(Into::into)
    }

    pub fn drive(
        &mut self,
        secure: &mut SecureNode,
        manager: &Manager,
        carriers: Option<
            &std::collections::BTreeMap<meshway_model::NodeId, meshway_connectivity::Carrier>,
        >,
        gateway: &mpsc::Sender<Request>,
    ) {
        // Drain already-completed admissions before considering probe timers.
        // Packet work may have deferred this consumer without delaying wire I/O.
        while let Some(sent) = self.sends.try_join_next() {
            self.finish_send(sent);
        }
        let before = self.locals.len();
        self.locals
            .retain(|_, local| !local.request.cancellation.is_cancelled());
        if self.locals.len() != before {
            self.dirty = true;
            // Remove cancelled local tasks from the shared executor before
            // producing outputs: disappearance from the IPC map must never
            // reclassify a local result as a Server-owned task.
            if let Some(snapshot) = self.synchronized.clone() {
                self.synchronize(&snapshot);
            }
        }
        // Local disconnect stops delivery immediately.
        self.apply_pauses();
        for local in self
            .locals
            .values_mut()
            .filter(|local| !local.request.cancellation.is_cancelled())
        {
            let progress = if self.owner.preparing(local.observation) {
                Progress::Setup
            } else {
                Progress::Waiting
            };
            local.progress.send_if_modified(|old| old.replace(progress));
            if local.lease_ended() {
                let _ = local.request.events.try_send(PingEvent::Failed {
                    code: PingError::Timeout,
                });
                local.request.cancellation.cancel();
                continue;
            }
            if local.request.events.is_closed() {
                local.request.cancellation.cancel();
                continue;
            }
            if self.owner.preparing(local.observation) && Instant::now() >= local.next_progress {
                local.next_progress = Instant::now() + Duration::from_secs(1);
                if local.preparing_since.elapsed() >= meshway_diagnostics::SETUP_TIMEOUT {
                    let _ = local.request.events.try_send(PingEvent::Failed {
                        code: PingError::Timeout,
                    });
                    local.request.cancellation.cancel();
                } else if let Ok(elapsed_ms) =
                    meshway_model::Sequence::new(local.preparing_since.elapsed().as_millis() as u64)
                    && local
                        .request
                        .events
                        .try_send(PingEvent::Preparing { elapsed_ms })
                        .is_err()
                {
                    local.request.cancellation.cancel();
                }
            }
        }
        self.apply_pauses();
        if let Some(carriers) = carriers {
            use meshway_connectivity::AdjacencyInterest;
            let interest = manager.interest();
            for peer in self.owner.adjacency_interests() {
                interest.interest(peer);
            }
            for action in self.owner.actions(
                secure,
                carriers,
                meshway_diagnostics::MAX_TASKS.saturating_sub(self.sends.len()),
            ) {
                self.send(
                    manager,
                    action.frame,
                    Some(action.observation_id),
                    action.measurement_id,
                    action.cancellation,
                );
            }
        }
        self.flush(gateway);
    }

    fn send(
        &mut self,
        manager: &Manager,
        frame: PendingFrame,
        observation: Option<ObservationId>,
        measurement: Option<EventId>,
        cancellation: tokio_util::sync::CancellationToken,
    ) {
        // This owned future shares the same Manager's captured channel/scope.
        let circuit = frame.circuit();
        if let Some(observation) = observation {
            tracing::debug!(%observation, ?measurement, circuit = ?frame.circuit(), carrier = ?frame.carrier(), "Diagnostic frame dispatched");
        }
        let send = manager.send_with_receipt(frame);
        let epoch = self.epoch.clone();
        self.sends.spawn(async move {
            let result = tokio::select! { biased;
                _ = epoch.cancelled() => Err(()),
                _ = cancellation.cancelled() => Err(()),
                result = send => result.map_err(|error| {
                    tracing::debug!(?circuit, ?observation, ?measurement, %error, "Diagnostic frame send failed");
                }),
            };
            Sent {
                observation,
                measurement,
                cancellation,
                result,
            }
        });
    }

    pub fn forward(&mut self, manager: &Manager, frame: PendingFrame) -> Result<(), ()> {
        if self.sends.len() >= meshway_diagnostics::MAX_TASKS {
            return Err(());
        }
        self.send(manager, frame, None, None, self.epoch.clone());
        Ok(())
    }

    pub fn has_sends(&self) -> bool {
        !self.sends.is_empty()
    }

    pub async fn sent(&mut self) {
        if let Some(sent) = self.sends.join_next().await {
            self.finish_send(sent);
        }
    }

    fn finish_send(&mut self, sent: Result<Sent, tokio::task::JoinError>) {
        let sent = match sent {
            Ok(sent) => sent,
            Err(_) => {
                self.invalidate();
                for local in self.locals.values() {
                    let _ = local.request.events.try_send(PingEvent::Failed {
                        code: PingError::Failed,
                    });
                    local.request.cancellation.cancel();
                }
                return;
            }
        };
        if !sent.cancellation.is_cancelled()
            && let Some(observation) = sent.observation
        {
            tracing::debug!(%observation, measurement = ?sent.measurement, admitted = sent.result.is_ok(), "Diagnostic frame completion received");
            let receipt = sent
                .result
                .and_then(|receipt| Ok((channel(&receipt.link)?, receipt.admitted_at)));
            self.owner
                .complete_send(observation, sent.measurement, receipt);
        }
    }

    fn flush(&mut self, gateway: &mpsc::Sender<Request>) {
        self.publications
            .retain(|_, publication| publication.output.is_current());
        let outputs: Vec<_> = self
            .owner
            .outputs()
            .filter(|output| !self.publications.contains_key(&output.task.id))
            .cloned()
            .collect();
        for output in outputs {
            let local_task = self
                .locals
                .values()
                .any(|local| local.observation == output.task.id);
            for local in self.locals.values_mut().filter(|local| {
                local.observation == output.task.id
                    && !local.request.cancellation.is_cancelled()
                    && !local.lease_ended()
            }) {
                match &output.result.result {
                    ObservationUpdate::Sample { sample, .. } => {
                        if local
                            .request
                            .events
                            .try_send(PingEvent::Sample {
                                observation_id: output.task.id,
                                sample: sample.as_ref().clone(),
                            })
                            .is_err()
                        {
                            local.request.cancellation.cancel();
                            continue;
                        }
                        local.samples = local.samples.saturating_add(1);
                        local.preparing_since = Instant::now();
                        if let Some(counts) = &sample.probe_counts {
                            local.sent += counts.sent;
                            local.success += counts.replied_within_deadline;
                            local.timeout += counts.timeout;
                        }
                        local.failed += u32::from(sample.probe.as_ref().is_some_and(|probe| {
                            probe.outcome == meshway_model::state::ProbeOutcome::Failed
                        }));
                        if local.samples >= local.request.count {
                            let _ = local.request.events.try_send(PingEvent::Completed {
                                sent: local.sent,
                                success: local.success,
                                timeout: local.timeout,
                                failed: local.failed,
                            });
                            local.request.cancellation.cancel();
                        }
                    }
                    ObservationUpdate::Phase { phase } => {
                        let _ = local.request.events.try_send(PingEvent::Failed {
                            code: if matches!(phase, meshway_protocol::observations::ReportedObservationPhase::Unsupported) { PingError::Unsupported } else { PingError::Failed },
                        });
                        local.request.cancellation.cancel();
                    }
                }
            }
            if local_task {
                // Local results go back to IPC only.
                self.owner.acknowledge(output.task.id);
                continue;
            }
            self.publications.insert(
                output.task.id,
                Publication {
                    output,
                    pending: None,
                    retry: Instant::now(),
                },
            );
        }
        let mut acknowledged = Vec::new();
        for (id, publication) in &mut self.publications {
            if let Some(pending) = &mut publication.pending {
                match pending.try_recv() {
                    Ok(Ok(())) => {
                        acknowledged.push(*id);
                        continue;
                    }
                    Ok(Err(())) | Err(oneshot::error::TryRecvError::Closed) => {
                        publication.pending = None;
                        publication.retry = Instant::now() + Duration::from_secs(1);
                    }
                    Err(oneshot::error::TryRecvError::Empty) => continue,
                }
            }
            if Instant::now() < publication.retry {
                continue;
            }
            let (reply, pending) = oneshot::channel();
            if gateway
                .try_send(Request::ObservationResult {
                    output: Box::new(publication.output.clone()),
                    reply,
                })
                .is_ok()
            {
                publication.pending = Some(pending);
            }
            publication.retry = Instant::now() + Duration::from_secs(1);
        }
        for id in acknowledged {
            self.publications.remove(&id);
            self.owner.acknowledge(id);
        }
    }
}

pub(super) fn channel(link: &LinkFact) -> Result<ProbeChannel, ()> {
    Ok(ProbeChannel {
        link_id: link.id.into(),
        socket_id: link.socket_id.into(),
        peer_node_id: link.peer,
        transport: link.carrier.protocol,
        local_address: meshway_model::SocketAddress::try_from(link.local).map_err(|_| ())?,
        remote_address: meshway_model::SocketAddress::try_from(link.remote).map_err(|_| ())?,
        acquisition: link.acquisition.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stop_cancels_and_joins_every_admitted_sender() {
        let mut diagnostics = Diagnostics::default();
        let completed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for _ in 0..meshway_diagnostics::MAX_TASKS {
            let epoch = diagnostics.epoch.clone();
            let completed = completed.clone();
            diagnostics.sends.spawn(async move {
                epoch.cancelled().await;
                completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Sent {
                    observation: None,
                    measurement: None,
                    cancellation: epoch,
                    result: Err(()),
                }
            });
        }
        tokio::time::timeout(Duration::from_secs(2), diagnostics.stop())
            .await
            .unwrap();
        assert_eq!(
            completed.load(std::sync::atomic::Ordering::SeqCst),
            meshway_diagnostics::MAX_TASKS
        );
        assert!(!diagnostics.has_sends());
    }
}
