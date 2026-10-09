//! Instance-scoped local subscriptions. HTTP uses the existing Control gateway;
//! the network worker remains the only measurement executor.
use crate::runtime::{PingRequest, network};
use meshway_control::Control;
use meshway_ipc::{PingError, PingEvent};
use meshway_model::{ObservationSubscriptionId, observations::MetricKind};
use meshway_protocol::{
    CreateNodeObservation, NodeObservationResponse, RenewNodeObservation,
    observations::ObservationPath,
};
use std::{collections::BTreeMap, time::Duration};

struct Subscription {
    queued_at: tokio::time::Instant,
    local: PingRequest,
    create: CreateNodeObservation,
    response: Option<NodeObservationResponse>,
    attached: bool,
    renewal: Option<RenewNodeObservation>,
    retry: Retry,
    cancellation_result: Option<CancellationResult>,
    lease: tokio::sync::watch::Sender<Option<meshway_control::ObservationLease>>,
    progress: tokio::sync::watch::Sender<crate::runtime::ping::Progress>,
    intent: crate::runtime::ping::Intent,
    create_started: bool,
}

enum CancellationResult {
    Completed(Box<meshway_protocol::NodeObservationCancelled>),
    Rejected,
}

struct Retry {
    next: tokio::time::Instant,
    cancelling: bool,
}
impl Retry {
    fn new(now: tokio::time::Instant) -> Self {
        Self {
            next: now,
            cancelling: false,
        }
    }
    fn ready(&mut self, cancelled: bool, now: tokio::time::Instant) -> bool {
        if cancelled && !self.cancelling {
            self.cancelling = true;
            self.next = now;
        }
        now >= self.next
    }
    fn dispatched(&mut self) {
        self.next = tokio::time::Instant::now() + Duration::from_secs(1);
    }
}
#[derive(Default)]
pub(super) struct Subscriptions {
    entries: BTreeMap<ObservationSubscriptionId, Subscription>,
    cursor: Option<ObservationSubscriptionId>,
}
impl Subscriptions {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn accept(&mut self, session: crate::runtime::PingSession, control: &Control) {
        let crate::runtime::PingSession {
            request: local,
            lease,
            progress,
            intent,
        } = session;
        if !intent.current() {
            fail(&local, PingError::Stopped);
            return;
        }
        if local.cancellation.is_cancelled() {
            return;
        }
        if !(1..=1000).contains(&local.count) {
            fail(&local, PingError::Protocol);
            return;
        }
        let Ok(instance) = crate::runtime::ping::source_instance(control) else {
            fail(&local, PingError::NotRunning);
            return;
        };
        if self.entries.len() >= 8
            || self
                .entries
                .values()
                .any(|entry| entry.local.id == local.id)
        {
            fail(&local, PingError::Busy);
            return;
        }
        let Some(subscription_id) = new_id() else {
            fail(&local, PingError::Failed);
            return;
        };
        let create = CreateNodeObservation {
            instance: meshway_protocol::InstanceRequest {
                instance_id: instance.instance_id,
                generation: instance.generation,
            },
            subscription_id,
            target: local.target.clone(),
            service: meshway_configuration::execution::ObservationServiceScope::IcmpEcho,
            selection: ObservationPath::Auto,
            metrics: vec![MetricKind::Rtt, MetricKind::ProbeLoss],
            interval_ms: 1000,
        };
        tracing::info!(subscription = %subscription_id, source_instance = %instance.instance_id,
            stage = "subscription_queued", "Diagnostic preparation stage changed");
        self.entries.insert(
            subscription_id,
            Subscription {
                queued_at: tokio::time::Instant::now(),
                local,
                create,
                response: None,
                attached: false,
                renewal: None,
                retry: Retry::new(tokio::time::Instant::now()),
                cancellation_result: None,
                lease,
                progress,
                intent,
                create_started: false,
            },
        );
    }

    pub async fn advance(
        &mut self,
        control: &mut Control,
        mut network: Option<&mut network::Handle>,
    ) -> bool {
        let mut removed = Vec::new();
        let mut attempted = false;
        let started = tokio::time::Instant::now();
        let mut ids: Vec<_> = self.entries.keys().copied().collect();
        if let Some(cursor) = self.cursor {
            let at = ids.partition_point(|id| *id <= cursor);
            ids.rotate_left(at);
        }
        // Preserve round-robin order within each class; first admission need not
        // wait behind maintenance of an already accepted subscription.
        ids.sort_by_key(|id| self.entries[id].create_started);
        for id in &ids {
            // A slow local observation RPC cannot monopolize lease renewal or
            // the connectivity gateway. Rotate the next turn across clients.
            if started.elapsed() >= Duration::from_millis(100) {
                break;
            }
            self.cursor = Some(*id);
            let Some(entry) = self.entries.get_mut(id) else {
                continue;
            };
            if !entry.intent.current() {
                fail(&entry.local, PingError::Stopped);
            }
            let Some(network) = network.as_deref_mut() else {
                fail(&entry.local, PingError::Unsupported);
                removed.push(*id);
                continue;
            };
            if entry.cancellation_result.is_some() {
                if finish_cancellation(entry, *id, control, network) {
                    removed.push(*id);
                }
                continue;
            }
            let lease_expired = entry
                .lease
                .borrow()
                .as_ref()
                .is_some_and(|lease| !lease.is_current());
            if lease_expired && !entry.local.cancellation.is_cancelled() {
                fail(&entry.local, PingError::Timeout);
            }
            if lease_expired && let Some(response) = &entry.response {
                network.resume_observation(response.task.id);
            }
            if crate::runtime::ping::source_instance(control).is_err()
                && !entry.local.cancellation.is_cancelled()
            {
                fail(&entry.local, PingError::NotRunning);
            }
            if control.status().instance.as_ref().is_none_or(|instance| {
                instance.instance_id != entry.create.instance.instance_id
                    || instance.generation != entry.create.instance.generation
            }) {
                fail(&entry.local, PingError::NotRunning);
                if network.detach_ping(*id).is_ok() {
                    removed.push(*id);
                }
                continue;
            }
            let cancelled =
                entry.local.cancellation.is_cancelled() || entry.local.events.is_closed();
            if cancelled {
                entry.local.cancellation.cancel();
                if !entry.create_started {
                    removed.push(*id);
                    continue;
                }
                if !lease_expired && let Some(response) = &entry.response {
                    network.pause_observation(response.task.id);
                }
            }
            if !entry.retry.ready(cancelled, tokio::time::Instant::now()) {
                continue;
            }
            if cancelled {
                entry.retry.dispatched();
                attempted = true;
                tracing::debug!(operation = "cancel_node_observation", subscription = %id, source_instance = %entry.create.instance.instance_id, observation = ?entry.response.as_ref().map(|r| r.task.id), "Observation subscription cancellation dispatched");
                match tokio::time::timeout(
                    Duration::from_secs(5),
                    control.cancel_node_observation(*id, &entry.create.instance),
                )
                .await
                {
                    Ok(Ok(cancelled)) => {
                        tracing::debug!(operation = "cancel_node_observation", subscription = %id, observation = ?cancelled.task.as_ref().map(|t| t.id), publication = ?cancelled.task.as_ref().map(|t| t.configuration.execution_publication), phase = ?cancelled.task.as_ref().map(|t| t.phase), "Observation subscription cancellation completed");
                        entry.cancellation_result =
                            Some(CancellationResult::Completed(Box::new(cancelled)));
                    }
                    Ok(Err(error)) if terminal(&error) => {
                        entry.cancellation_result = Some(CancellationResult::Rejected);
                    }
                    _ => {}
                }
                if finish_cancellation(entry, *id, control, network) {
                    removed.push(*id);
                }
                continue;
            }
            if entry.response.is_none() {
                match network.ping_readiness(control) {
                    crate::runtime::ping::Readiness::Ready => {}
                    crate::runtime::ping::Readiness::Preparing => continue,
                    crate::runtime::ping::Readiness::Unavailable => {
                        fail(&entry.local, PingError::NotRunning);
                        continue;
                    }
                }
                entry.retry.dispatched();
                attempted = true;
                entry.create_started = true;
                let requested_at = tokio::time::Instant::now();
                tracing::debug!(subscription = %id, stage = "subscription_request", "Diagnostic subscription request dispatched");
                let result = tokio::time::timeout(
                    Duration::from_secs(5),
                    control.create_node_observation(&entry.create),
                )
                .await;
                match result {
                    Ok(Ok(response)) => {
                        tracing::info!(subscription = %id, observation = %response.task.id,
                            publication = %response.task.configuration.execution_publication,
                            phase = ?response.task.phase,
                            queue_ms = requested_at.duration_since(entry.queued_at).as_millis() as u64,
                            request_ms = requested_at.elapsed().as_millis() as u64,
                            stage = "subscription_accepted", "Diagnostic preparation stage changed");
                        if !entry.intent.current() || entry.local.cancellation.is_cancelled() {
                            entry.response = Some(response);
                            continue;
                        }
                        if let Ok(lease) = control.node_observation_lease(*id) {
                            entry.lease.send_replace(Some(lease));
                        }
                        if entry
                            .local
                            .events
                            .try_send(PingEvent::Accepted {
                                subscription_id: *id,
                                observation_id: response.task.id,
                                target: response.target.clone(),
                            })
                            .is_err()
                        {
                            entry.local.cancellation.cancel();
                        }
                        entry.response = Some(response);
                    }
                    Ok(Err(error)) if terminal(&error) => {
                        fail(&entry.local, code(&error));
                        removed.push(*id);
                    }
                    _ => {}
                }
            }
            if let Some(response) = &entry.response {
                if !matches!(
                    response.task.phase,
                    meshway_protocol::observations::ObservationPhase::Preparing
                        | meshway_protocol::observations::ObservationPhase::Running
                ) {
                    fail(
                        &entry.local,
                        if response.task.phase
                            == meshway_protocol::observations::ObservationPhase::Unsupported
                        {
                            PingError::Unsupported
                        } else {
                            PingError::Failed
                        },
                    );
                    continue;
                }
                if !entry.attached {
                    entry.attached = network
                        .attach_ping(
                            *id,
                            response.task.id,
                            entry.local.clone(),
                            entry.lease.subscribe(),
                            entry.progress.clone(),
                        )
                        .is_ok();
                }
                if started.elapsed() < Duration::from_millis(100)
                    && !entry.local.cancellation.is_cancelled()
                    && (response.task.renew_after <= chrono::Utc::now() || entry.renewal.is_some())
                {
                    entry.retry.dispatched();
                    attempted = true;
                    let renewal = entry.renewal.get_or_insert_with(|| RenewNodeObservation {
                        instance: entry.create.instance.clone(),
                        expires_at_ms: u64::try_from(chrono::Utc::now().timestamp_millis())
                            .unwrap_or(0)
                            .saturating_add(30_000),
                    });
                    match tokio::time::timeout(
                        Duration::from_secs(5),
                        control.renew_node_observation(*id, renewal),
                    )
                    .await
                    {
                        Ok(Ok(response)) => {
                            if !entry.intent.current() || entry.local.cancellation.is_cancelled() {
                                entry.response = Some(response);
                                entry.renewal = None;
                                continue;
                            }
                            if let Ok(lease) = control.node_observation_lease(*id) {
                                entry.lease.send_replace(Some(lease));
                            }
                            entry.response = Some(response);
                            entry.renewal = None;
                        }
                        Ok(Err(error)) if terminal(&error) => {
                            fail(&entry.local, code(&error));
                        }
                        _ => {}
                    }
                }
            }
        }
        for id in removed {
            self.entries.remove(&id);
        }
        attempted
    }

    pub fn stop(&mut self) {
        for entry in self.entries.values() {
            fail(&entry.local, PingError::Stopped);
        }
        self.entries.clear();
        self.cursor = None;
    }
}
fn fail(local: &PingRequest, code: PingError) {
    let _ = local.events.try_send(PingEvent::Failed { code });
    local.cancellation.cancel();
}

fn finish_cancellation(
    entry: &Subscription,
    id: ObservationSubscriptionId,
    control: &Control,
    network: &mut network::Handle,
) -> bool {
    let Some(result) = &entry.cancellation_result else {
        return false;
    };
    // Local command pressure must not replay an already completed Server
    // transaction or repeatedly publish/rebind a still-shared observation.
    if network.detach_ping(id).is_err() {
        return false;
    }
    match result {
        CancellationResult::Completed(value) => {
            if let Some(task) = &value.task {
                if control
                    .execution_snapshot()
                    .is_ok_and(|snapshot| snapshot.reference() == task.configuration)
                {
                    network.resume_observation(task.id);
                } else {
                    network.resume_observation_after_refresh(task.id);
                }
            }
        }
        CancellationResult::Rejected => {
            if let Some(response) = &entry.response {
                network.resume_observation_after_refresh(response.task.id);
            }
        }
    }
    true
}
fn terminal(error: &meshway_control::Error) -> bool {
    matches!(
        error,
        meshway_control::Error::Contract
            | meshway_control::Error::Lifecycle
            | meshway_control::Error::Expired
    ) || matches!(
        error,
        meshway_control::Error::Server(
            meshway_control_client::ClientError::Decode { .. }
                | meshway_control_client::ClientError::BodyLimit { .. }
                | meshway_control_client::ClientError::Request
        )
    ) || matches!(http_status(error), Some(400 | 401 | 403 | 404 | 409 | 422))
}
fn code(error: &meshway_control::Error) -> PingError {
    match http_status(error) {
        Some(403) => PingError::PermissionDenied,
        Some(404) => PingError::TargetNotFound,
        Some(429) => PingError::Busy,
        _ if matches!(
            error,
            meshway_control::Error::Contract
                | meshway_control::Error::Server(
                    meshway_control_client::ClientError::Decode { .. }
                        | meshway_control_client::ClientError::BodyLimit { .. }
                        | meshway_control_client::ClientError::Request
                )
        ) =>
        {
            PingError::Protocol
        }
        _ => PingError::Failed,
    }
}
fn http_status(error: &meshway_control::Error) -> Option<u16> {
    match error {
        meshway_control::Error::Server(
            meshway_control_client::ClientError::Http { status, .. }
            | meshway_control_client::ClientError::Network {
                status: Some(status),
            },
        ) => Some(*status),
        _ => None,
    }
}
fn new_id() -> Option<ObservationSubscriptionId> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).ok()?;
    bytes[6] = bytes[6] & 15 | 64;
    bytes[8] = bytes[8] & 63 | 128;
    let h: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
    .parse()
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{PingSession, network::testing::Worker, ping};

    fn local_session(
        target: meshway_protocol::NodeObservationTarget,
    ) -> (
        tokio::sync::watch::Sender<crate::runtime::intent::Desired>,
        PingSession,
        tokio::sync::mpsc::Receiver<PingEvent>,
    ) {
        let (desired, intent) = ping::test_intent();
        let (events, received) = tokio::sync::mpsc::channel(8);
        let local = PingRequest {
            id: new_id().unwrap().to_string().parse().unwrap(),
            target,
            count: 2,
            events,
            cancellation: tokio_util::sync::CancellationToken::new(),
        };
        (
            desired,
            PingSession {
                request: local,
                intent,
                lease: tokio::sync::watch::channel(None).0,
                progress: tokio::sync::watch::channel(ping::Progress::Setup).0,
            },
            received,
        )
    }

    #[tokio::test]
    async fn stale_public_snapshot_waits_for_installation_without_create_then_accepts_same_request()
    {
        let mut fixture = ping::integration::Fixture::open().await;
        let mut worker = Worker::new();
        let ticket = fixture.controls[0].begin_application().await.unwrap();
        let snapshot = fixture.controls[0].execution_snapshot().unwrap();
        worker.install(ticket.clone(), snapshot.clone(), false);
        let (desired, session, mut events) = local_session(fixture.request.target.clone());
        fixture.assert_stale_queue(desired.subscribe());
        let request_id = session.request.id;
        let mut subscriptions = Subscriptions::default();
        let before = fixture.creates();
        assert!(before > 0, "audit must see real Node API creates");
        subscriptions.accept(session, &fixture.controls[0]);
        assert_eq!(subscriptions.entries.len(), 1);
        for _ in 0..3 {
            assert!(
                !subscriptions
                    .advance(&mut fixture.controls[0], Some(&mut worker.handle))
                    .await
            );
            worker.no_commands();
            assert!(events.try_recv().is_err());
            assert_eq!(fixture.creates(), before);
        }
        worker.install(ticket, snapshot, true);
        assert!(
            subscriptions
                .advance(&mut fixture.controls[0], Some(&mut worker.handle))
                .await
        );
        assert_eq!(fixture.creates(), before + 1);
        assert!(matches!(events.try_recv(), Ok(PingEvent::Accepted { .. })));
        worker.attached(request_id);
        subscriptions.stop();
        worker.close().await;
        fixture.close().await;
    }

    #[tokio::test]
    async fn changed_operator_during_real_create_cannot_emit_accept_or_attach() {
        let mut fixture = ping::integration::Fixture::open().await;
        let mut worker = Worker::new();
        let ticket = fixture.controls[0].begin_application().await.unwrap();
        worker.install(
            ticket,
            fixture.controls[0].execution_snapshot().unwrap(),
            true,
        );
        let (desired, session, mut events) = local_session(fixture.request.target.clone());
        let mut subscriptions = Subscriptions::default();
        subscriptions.accept(session, &fixture.controls[0]);
        let gate = fixture.hold_create();
        {
            let advance = subscriptions.advance(&mut fixture.controls[0], Some(&mut worker.handle));
            tokio::pin!(advance);
            tokio::select! {
                _ = gate.entered() => {},
                _ = &mut advance => panic!("create must stay held at the real HTTP boundary"),
            }
            desired.send_modify(|value| {
                value.revision = meshway_model::Version::new(2).unwrap();
                value.up = false;
            });
            gate.release();
            advance.await;
        }
        assert!(events.try_recv().is_err());
        worker.no_commands();
        subscriptions
            .advance(&mut fixture.controls[0], Some(&mut worker.handle))
            .await;
        assert!(matches!(
            events.try_recv(),
            Ok(PingEvent::Failed {
                code: PingError::Stopped
            })
        ));
        assert!(
            subscriptions.is_empty(),
            "completed create is canceled by the original owner"
        );
        worker.close().await;
        fixture.close().await;
    }

    #[tokio::test]
    async fn actual_execution_expiry_rejects_pending_and_new_requests_without_create() {
        // C: the real issued EXECUTION_TTL (60 s) must expire. sleep_until uses
        // its absolute deadline, not a duration measured after fixture setup.
        let mut fixture = ping::integration::Fixture::open().await;
        let mut worker = Worker::new();
        let ticket = fixture.controls[0].begin_application().await.unwrap();
        let snapshot = fixture.controls[0].execution_snapshot().unwrap();
        let expires = snapshot.expires_at();
        worker.install(ticket, snapshot, false);
        let (_desired, session, mut events) = local_session(fixture.request.target.clone());
        let mut subscriptions = Subscriptions::default();
        subscriptions.accept(session, &fixture.controls[0]);
        let before = fixture.creates();
        tokio::time::sleep_until(expires.into()).await;
        assert!(ping::source_instance(&fixture.controls[0]).is_err());
        subscriptions
            .advance(&mut fixture.controls[0], Some(&mut worker.handle))
            .await;
        assert!(matches!(
            events.try_recv(),
            Ok(PingEvent::Failed {
                code: PingError::NotRunning
            })
        ));
        let (_desired, session, mut events) = local_session(fixture.request.target.clone());
        subscriptions.accept(session, &fixture.controls[0]);
        assert!(matches!(
            events.try_recv(),
            Ok(PingEvent::Failed {
                code: PingError::NotRunning
            })
        ));
        assert_eq!(fixture.creates(), before);
        worker.close().await;
        fixture.close().await;
    }

    #[tokio::test]
    async fn pending_and_first_delivery_cannot_cross_operator_revision_or_real_control_stop() {
        let mut fixture = ping::integration::Fixture::open().await;
        let mut worker = Worker::new();
        let ticket = fixture.controls[0].begin_application().await.unwrap();
        worker.install(
            ticket,
            fixture.controls[0].execution_snapshot().unwrap(),
            false,
        );
        let before = fixture.creates();
        let mut subscriptions = Subscriptions::default();
        for accepted in [false, true] {
            let (desired, session, mut events) = local_session(fixture.request.target.clone());
            let mut session = Some(session);
            if accepted {
                subscriptions.accept(session.take().unwrap(), &fixture.controls[0]);
            }
            desired.send_modify(|value| {
                value.up = false;
                value.revision = meshway_model::Version::new(2).unwrap();
            });
            desired.send_modify(|value| {
                value.up = true;
                value.revision = meshway_model::Version::new(3).unwrap();
            });
            if accepted {
                subscriptions
                    .advance(&mut fixture.controls[0], Some(&mut worker.handle))
                    .await;
            } else {
                subscriptions.accept(session.take().unwrap(), &fixture.controls[0]);
            }
            assert!(matches!(
                events.try_recv(),
                Ok(PingEvent::Failed {
                    code: PingError::Stopped
                })
            ));
            assert!(subscriptions.is_empty());
        }
        let (_desired, session, mut events) = local_session(fixture.request.target.clone());
        subscriptions.accept(session, &fixture.controls[0]);
        fixture.controls[0].stop().await.unwrap();
        subscriptions
            .advance(&mut fixture.controls[0], Some(&mut worker.handle))
            .await;
        assert!(matches!(
            events.try_recv(),
            Ok(PingEvent::Failed {
                code: PingError::NotRunning
            })
        ));
        let (_desired, session, mut events) = local_session(fixture.request.target.clone());
        subscriptions.accept(session, &fixture.controls[0]);
        assert!(matches!(
            events.try_recv(),
            Ok(PingEvent::Failed {
                code: PingError::NotRunning
            })
        ));
        assert_eq!(fixture.creates(), before);
        worker.close().await;
        fixture.close().await;
    }
    #[test]
    fn idle_inspection_never_delays_new_cancellation() {
        let now = tokio::time::Instant::now();
        let mut retry = Retry::new(now);
        for _ in 0..10 {
            assert!(retry.ready(false, now));
        }
        assert_eq!(retry.next, now);
        assert!(retry.ready(true, now));
    }
    #[test]
    fn first_cancel_bypasses_previous_operation_backoff_but_retries_stay_paced() {
        let now = tokio::time::Instant::now();
        let mut retry = Retry::new(now);
        retry.dispatched();
        assert!(!retry.ready(false, now));
        assert!(retry.ready(true, now));
        retry.dispatched();
        assert!(!retry.ready(true, now));
        assert!(retry.ready(true, retry.next));
    }
}
