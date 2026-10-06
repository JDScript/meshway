//! Host adapter for the shared observation executor. No HTTP or cipher owner.
use super::Request;
use crate::runtime::PingRequest;
use crate::runtime::ping::Progress;
use meshway_connectivity::{LinkFact, Manager};
use meshway_dataplane::secure::{PendingFrame, SecureNode};
use meshway_diagnostics::{Output, Owner};
use meshway_ipc::{PingError, PingEvent};
use meshway_model::{
    EventId, ObservationId, ObservationSubscriptionId, observations::ProbeChannel,
};
use meshway_protocol::observations::ObservationUpdate;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

struct Local {
    observation: ObservationId,
    request: PingRequest,
    sent: u32,
    success: u32,
    timeout: u32,
    failed: u32,
    samples: u16,
    preparing_since: Instant,
    next_progress: Instant,
    lease: tokio::sync::watch::Receiver<Option<meshway_control::ObservationLease>>,
    progress: tokio::sync::watch::Sender<Progress>,
}
impl Local {
    fn lease_ended(&self) -> bool {
        self.lease
            .borrow()
            .as_ref()
            .is_some_and(|lease| !lease.is_current())
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
    paused: std::collections::BTreeSet<ObservationId>,
    effective_pauses: std::collections::BTreeSet<ObservationId>,
}

impl Diagnostics {
    pub fn plan(&mut self, plan: Option<super::ObservationPlan>) {
        if plan.is_none() {
            self.owner.suspend();
            for local in self.locals.values() {
                local
                    .progress
                    .send_if_modified(|progress| progress.replace(Progress::Setup));
            }
        }
        self.pending_plan = plan;
    }

    pub fn synchronize(&mut self, installed: &meshway_control::ExecutionSnapshot) {
        let Some(plan) = &self.pending_plan else {
            return;
        };
        if plan.snapshot.ensure_current().is_err() {
            self.pending_plan = None;
            return;
        }
        if !plan.snapshot.same_execution(installed) {
            return;
        }
        let plan = self.pending_plan.take().expect("matching plan");
        tracing::debug!(publication = %plan.snapshot.reference().execution_publication, tasks = plan.tasks.len(), "Synchronizing observation plan with installed execution");
        if self.owner.synchronize(plan.snapshot, &plan.tasks).is_err() {
            self.reject_tasks();
        }
        self.apply_pauses();
    }

    pub fn set_paused(&mut self, paused: std::collections::BTreeSet<ObservationId>) {
        self.paused = paused;
        self.apply_pauses();
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
        for local in self.locals.values() {
            let _ = local.request.events.try_send(PingEvent::Failed {
                code: PingError::Protocol,
            });
            local.request.cancellation.cancel();
        }
    }
    pub fn detach(&mut self, subscription: ObservationSubscriptionId) {
        self.locals.remove(&subscription);
        self.apply_pauses();
    }
    pub fn attach(
        &mut self,
        subscription: ObservationSubscriptionId,
        observation: ObservationId,
        request: PingRequest,
        lease: tokio::sync::watch::Receiver<Option<meshway_control::ObservationLease>>,
        progress: tokio::sync::watch::Sender<Progress>,
    ) {
        if self.locals.contains_key(&subscription) {
            return;
        }
        if self.locals.len() >= 8 {
            let _ = request.events.try_send(PingEvent::Failed {
                code: PingError::Busy,
            });
            request.cancellation.cancel();
            return;
        }
        self.locals.insert(
            subscription,
            Local {
                observation,
                request,
                sent: 0,
                success: 0,
                timeout: 0,
                failed: 0,
                samples: 0,
                preparing_since: Instant::now(),
                next_progress: Instant::now(),
                lease,
                progress,
            },
        );
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
        gateway: &mpsc::Sender<Request>,
    ) {
        // Drain already-completed admissions before considering probe timers.
        // Packet work may have deferred this consumer without delaying wire I/O.
        while let Some(sent) = self.sends.try_join_next() {
            self.finish_send(sent);
        }
        // Local disconnect stops delivery immediately. Pause the shared task
        // while Control settles cancellation; its returned shared task resumes.
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
        if let Ok(carriers) = manager.carriers() {
            for action in self.owner.actions(
                secure,
                &carriers,
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

    #[test]
    fn shared_task_pause_is_released_in_both_detach_resume_orders() {
        let observation = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        let subscription = "00000000-0000-4000-8000-000000000002".parse().unwrap();
        for resume_first in [false, true] {
            let mut diagnostics = Diagnostics::default();
            let (events, _received) = mpsc::channel(8);
            let cancellation = tokio_util::sync::CancellationToken::new();
            let (_lease, lease) = tokio::sync::watch::channel(None);
            let (progress, _progress) = tokio::sync::watch::channel(Progress::Setup);
            diagnostics.attach(
                subscription,
                observation,
                PingRequest {
                    id: "00000000-0000-4000-8000-000000000003".parse().unwrap(),
                    target: meshway_protocol::NodeObservationTarget::Name {
                        name: "target".into(),
                    },
                    count: 2,
                    events,
                    cancellation: cancellation.clone(),
                },
                lease,
                progress,
            );
            cancellation.cancel();
            diagnostics.set_paused([observation].into_iter().collect());
            if resume_first {
                diagnostics.set_paused(Default::default());
                assert!(diagnostics.pause_causes().contains(&observation));
                diagnostics.detach(subscription);
            } else {
                diagnostics.detach(subscription);
                assert!(diagnostics.pause_causes().contains(&observation));
                diagnostics.set_paused(Default::default());
            }
            assert!(diagnostics.pause_causes().is_empty());
            assert!(diagnostics.effective_pauses.is_empty());
        }
    }

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
