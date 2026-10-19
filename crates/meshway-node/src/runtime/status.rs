//! Project the coordinator's intent and the control owner's expiring snapshot.
use super::intent::Desired;
use chrono::{DateTime, Utc};
use meshway_control::StatusSnapshot;
use meshway_model::Version;
use meshway_protocol::ipc::{DataPlane, NetworkMode, Phase, Problem, Status, SyncState};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

#[derive(Clone, Copy)]
pub(super) struct RotationOutcome {
    pub id: meshway_model::RequestId,
    pub result: Result<(), meshway_protocol::ipc::ErrorCode>,
}

#[derive(Clone)]
pub(super) struct Report {
    pub revision: Version,
    pub phase: Phase,
    pub control: Option<StatusSnapshot>,
    pub enrollment_url: Option<String>,
    pub problem: Option<Problem>,
    pub retry_at: Option<DateTime<Utc>>,
    pub observed_at: DateTime<Utc>,
    pub rotation: Option<RotationOutcome>,
    pub logout: Option<RotationOutcome>,
}
impl Report {
    pub fn initial(revision: Version) -> Self {
        Self {
            revision,
            phase: Phase::Starting,
            control: None,
            enrollment_url: None,
            problem: None,
            retry_at: None,
            observed_at: Utc::now(),
            rotation: None,
            logout: None,
        }
    }
}
/// The packet plane's state as Status shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlaneState {
    phase: Phase,
    applied: Option<meshway_model::state::NetmapRef>,
    data_plane: DataPlane,
}

/// When the shown [`PlaneState`] last changed (Status `observedAt`). Both
/// [`View::track`], which sees every input change, and each Status read
/// advance it; neither restamps an unchanged state.
struct Clock {
    state: Option<PlaneState>,
    since: DateTime<Utc>,
}
impl Clock {
    fn observe(&mut self, state: PlaneState) -> DateTime<Utc> {
        if self.state != Some(state) {
            self.state = Some(state);
            self.since = Utc::now();
        }
        self.since
    }
}

#[derive(Clone)]
pub(crate) struct View {
    pub(super) desired: watch::Receiver<Desired>,
    pub(super) report: watch::Receiver<Report>,
    pub(super) server: String,
    pub(super) network_mode: NetworkMode,
    pub(super) network: watch::Receiver<super::network::Report>,
    pub(super) sync: watch::Receiver<super::control::SyncStatus>,
    clock: Arc<Mutex<Clock>>,
}

/// One consistent read of the inputs and what they derive.
struct Derived {
    desired: Desired,
    report: Report,
    network: super::network::Report,
    current: bool,
    active: bool,
    network_current: bool,
    lease: Option<meshway_model::NetmapLease>,
    plane: PlaneState,
}

impl View {
    pub(super) fn new(
        desired: watch::Receiver<Desired>,
        report: watch::Receiver<Report>,
        server: String,
        network_mode: NetworkMode,
        network: watch::Receiver<super::network::Report>,
        sync: watch::Receiver<super::control::SyncStatus>,
    ) -> Self {
        Self {
            desired,
            report,
            server,
            network_mode,
            network,
            sync,
            clock: Arc::new(Mutex::new(Clock {
                state: None,
                since: Utc::now(),
            })),
        }
    }

    fn observe(&self, state: PlaneState) -> DateTime<Utc> {
        match self.clock.lock() {
            Ok(mut clock) => clock.observe(state),
            Err(poisoned) => poisoned.into_inner().observe(state),
        }
    }

    /// Stamp each change of the shown packet plane state when it happens,
    /// read or not: Control's report, the network owner's report, the DNS
    /// owner's fault behind it and the intent. Ends at `stop` or once an
    /// input's owner is gone.
    ///
    /// Each turn waits for a change it has not seen. The DNS guard in a
    /// network report is a live view its holders only borrow, so its own
    /// seen version lags behind every send since the DNS owner started; the
    /// tracker's clone is marked seen before the inputs are read, else
    /// `changed` stays ready and the turn never yields.
    pub(super) async fn track(mut self, mut stop: watch::Receiver<bool>) {
        loop {
            let mut guard = self
                .network
                .borrow()
                .dns_guard
                .clone()
                .filter(|guard| guard.has_changed().is_ok());
            if let Some(guard) = &mut guard {
                guard.mark_unchanged();
            }
            let derived = self.derive();
            self.observe(derived.plane);
            tokio::select! {
                _ = stop.wait_for(|stopped| *stopped) => return,
                changed = self.desired.changed() => if changed.is_err() { return },
                changed = self.report.changed() => if changed.is_err() { return },
                changed = self.network.changed() => if changed.is_err() { return },
                _ = async {
                    match guard {
                        // Closed: one more turn reads it, then it is filtered out.
                        Some(mut guard) => { let _ = guard.changed().await; }
                        None => std::future::pending().await,
                    }
                } => {},
            }
        }
    }

    fn derive(&self) -> Derived {
        let mut desired = self.desired.borrow().clone();
        desired.shutdown |= self.desired.has_changed().is_err();
        let report = self.report.borrow().clone();
        let network = self.network.borrow().clone();
        let control = report.control.as_ref().map(StatusSnapshot::status);
        let current = report.revision == desired.revision;
        let active = current && desired.up && !desired.shutdown;
        let lease = control.as_ref().and_then(|c| c.netmap);
        // The installation is of the netmap Control currently authorizes.
        let network_current = active
            && network.current()
            && lease.is_some_and(|lease| Some(lease.netmap) == network.netmap);
        let data_plane = if self.network_mode == NetworkMode::ControlOnly {
            DataPlane::ControlOnly
        } else if !current || desired.shutdown {
            DataPlane::Preparing
        } else if !desired.up {
            if report.phase == Phase::Blocked {
                DataPlane::Failed
            } else if report.phase == Phase::Down {
                DataPlane::Stopped
            } else {
                DataPlane::Preparing
            }
        } else if report.phase == Phase::Blocked {
            DataPlane::Failed
        } else if network_current || network.plane != DataPlane::Ready {
            network.plane
        } else {
            DataPlane::Suspended
        };
        let applied = network_current.then(|| network.applied()).flatten();
        let mut phase = if desired.shutdown {
            Phase::Stopping
        } else if !current {
            if desired.up {
                Phase::Starting
            } else {
                Phase::Stopping
            }
        } else if !desired.up {
            if matches!(
                report.phase,
                Phase::Starting | Phase::Blocked | Phase::Leaving
            ) {
                report.phase
            } else {
                Phase::Down
            }
        } else {
            report.phase
        };
        if active && phase != Phase::Blocked && self.network_mode == NetworkMode::Tun {
            // The data plane runs on its netmap lease alone: a Server outage
            // (Retrying) does not stop a Running Node.
            let ready = applied.is_some()
                && (data_plane == DataPlane::Ready
                    || (data_plane == DataPlane::Stopped && network.completion == Some(Ok(()))));
            if ready
                && matches!(
                    phase,
                    Phase::ControlReady | Phase::Retrying | Phase::Starting
                )
            {
                phase = Phase::Running;
            } else if data_plane == DataPlane::Failed && phase == Phase::ControlReady {
                // A failure that recovers by itself retries; only one with
                // no automatic way back needs an operator.
                phase = if network.recovering() {
                    Phase::Retrying
                } else {
                    Phase::Blocked
                };
            } else if !ready && phase == Phase::ControlReady {
                phase = Phase::Synchronizing;
            }
        }
        if active && phase == Phase::ControlReady && lease.is_none() {
            phase = Phase::Synchronizing;
        }
        Derived {
            plane: PlaneState {
                phase,
                applied,
                data_plane,
            },
            desired,
            report,
            network,
            current,
            active,
            network_current,
            lease,
        }
    }

    pub fn status(&self) -> Status {
        let Derived {
            desired,
            report,
            network,
            current,
            active,
            network_current,
            lease,
            plane,
        } = self.derive();
        let observed_at = self.observe(plane);
        let PlaneState {
            phase,
            applied,
            data_plane,
        } = plane;
        let control = report.control.as_ref().map(StatusSnapshot::status);
        let sync = self.sync.borrow().clone();
        let sync_state = if !active || !sync.running {
            SyncState::Disconnected
        } else if sync
            .last_sync
            .is_some_and(|at| at.elapsed() < super::control::SYNC_STALE)
        {
            SyncState::Connected
        } else {
            SyncState::Reconnecting
        };
        let exit_dns = network
            .exit_dns
            .clone()
            .filter(|fact| network_current && network.netmap == Some(fact.netmap));
        let exit = (active
            && network.netmap.is_some()
            && (network.exit_capability.is_some() || exit_dns.is_some()))
        .then(|| meshway_protocol::ipc::ExitState {
            capability: network.exit_capability.clone().map(|mut capability| {
                if !network_current {
                    capability.set_installed(false);
                }
                capability
            }),
            installed_role: network_current
                .then(|| {
                    network
                        .exit_capability
                        .as_ref()
                        .filter(|c| c.installed())
                        .map(|c| c.role_generation)
                })
                .flatten(),
            active_flows: network_current.then_some(network.exit_flows).flatten(),
            dns_unavailable: exit_dns.and_then(|fact| match fact.status {
                meshway_model::exit::ExitDnsStatus::Serving => None,
                meshway_model::exit::ExitDnsStatus::Unavailable { reason } => Some(reason),
            }),
            cleanup_pending: network
                .exit_cleanups
                .as_ref()
                .and_then(|cleanups| cleanups.borrow().clone())
                .map(retry_pending),
        });
        let content = network
            .content
            .as_ref()
            .filter(|_| network_current)
            .map(|content| &content.netmap);
        let known = network
            .known_peers
            .as_ref()
            .and_then(|known| known.borrow().clone());
        let peers = content
            .map(|netmap| {
                super::peers::peers(&super::peers::Inputs {
                    netmap,
                    facts: network.facts.as_ref(),
                    routes: &network.routes,
                    relays: &network.relays,
                    idle_paths: &network.idle_paths,
                    rtts: &network.rtts,
                    known: known.as_ref(),
                    now: Utc::now(),
                })
            })
            .unwrap_or_default();
        let sockets = sockets(content.and(network.facts.as_ref()), network.netmap);
        // The DNS owner's fault the worker has not withdrawn for yet: the
        // plane withdraws for it, so it is already the cause.
        let network_failure = match network.problem {
            Some(problem) => Some((problem, network.problem_detail.clone())),
            None => network.dns_fault().map(|detail| (Problem::Dns, detail)),
        };
        let network_error = network_failure
            .filter(|_| active)
            .map(|(problem, detail)| meshway_protocol::ipc::NetworkError { problem, detail });
        Status {
            process_id: std::process::id(),
            revision: desired.revision,
            desired_up: desired.up,
            phase,
            server: self.server.clone(),
            observed_at,
            control_observed_at: Some(report.observed_at),
            enrollment_request_id: control.as_ref().and_then(|c| c.enrollment_request_id),
            enrollment_url: if current && desired.up {
                report.enrollment_url
            } else {
                None
            },
            identity: control.as_ref().and_then(|c| c.binding.clone()),
            authenticated: active && control.as_ref().is_some_and(|c| c.authenticated),
            sync: sync_state,
            last_sync_at: sync.last_sync_at,
            sync_error: sync.error.filter(|_| active && sync.running),
            instance: if active {
                control.as_ref().and_then(|c| c.instance.clone())
            } else {
                None
            },
            netmap: lease.filter(|_| active).and_then(|lease| {
                Some(meshway_protocol::ipc::NetmapStatus {
                    netmap: lease.netmap,
                    issued_at: timestamp(lease.issued_at_ms)?,
                    expires_at: timestamp(lease.expires_at_ms)?,
                    missing_artifacts: control
                        .as_ref()
                        .map(|c| {
                            c.missing_artifacts
                                .iter()
                                .map(ToString::to_string)
                                .collect()
                        })
                        .unwrap_or_default(),
                    artifact_problem: control.as_ref().and_then(|c| c.artifact_problem),
                    capture: network.capture.clone().filter(|_| network_current),
                    routing: control.as_ref().and_then(|c| c.routing.clone()),
                })
            }),
            saved: control.and_then(|c| c.held),
            applied,
            data_plane,
            network_mode: self.network_mode,
            exit,
            node: content.map(|netmap| super::peers::own(netmap)),
            peers,
            sockets,
            last_error: report.problem.or(network_error.as_ref().map(|e| e.problem)),
            // Each cause stays with its own failure: the packet plane's
            // is `lastErrorDetail` when it is `lastError`, else beside it.
            last_error_detail: network_error
                .as_ref()
                .filter(|_| report.problem.is_none())
                .and_then(|failure| failure.detail.clone()),
            network_error: network_error.filter(|_| report.problem.is_some()),
            retry_at: if current && desired.up {
                report.retry_at
            } else {
                None
            },
            last_network_change: network.last_network_change,
            change_pending: network
                .kernel_changes
                .as_ref()
                .and_then(|changes| changes.borrow().clone())
                .map(|pending| {
                    meshway_protocol::ipc::KernelChange::new(
                        match pending.owner {
                            meshway_underlay::KernelChangeOwner::EgressCapture => {
                                meshway_protocol::ipc::KernelChangeOwner::EgressCapture
                            }
                            meshway_underlay::KernelChangeOwner::Exit => {
                                meshway_protocol::ipc::KernelChangeOwner::Exit
                            }
                        },
                        pending.operation,
                        retry_pending(pending.retry),
                    )
                }),
            control_relay: network
                .control_relay
                .as_ref()
                .filter(|_| self.network_mode == NetworkMode::Tun)
                .and_then(|relay| relay.borrow().clone())
                .map(|state| match state {
                    meshway_underlay::ControlRelayState::Retrying(retry) => {
                        meshway_protocol::ipc::ControlRelay::retrying(retry_pending(retry))
                    }
                    meshway_underlay::ControlRelayState::AtCapacity { limit, refused } => {
                        meshway_protocol::ipc::ControlRelay {
                            state: meshway_protocol::ipc::ControlRelayState::AtCapacity,
                            reason: limit.into(),
                            attempts: u32::try_from(refused).unwrap_or(u32::MAX),
                            retry_at: None,
                        }
                    }
                }),
        }
    }
}

fn retry_pending(pending: meshway_underlay::RetryPending) -> meshway_protocol::ipc::RetryPending {
    meshway_protocol::ipc::RetryPending {
        reason: pending.reason,
        attempts: pending.attempts,
        retry_at: pending.retry_at.into(),
    }
}

fn timestamp(ms: u64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(i64::try_from(ms).ok()?)
}

/// The installed netmap's UDP sockets that report granted buffer sizes,
/// then its listeners whose bind is being retried.
fn sockets(
    facts: Option<&meshway_connectivity::Snapshot>,
    netmap: Option<meshway_model::state::NetmapRef>,
) -> Vec<meshway_protocol::ipc::SocketStatus> {
    let Some(facts) = facts else {
        return Vec::new();
    };
    let bytes = |value: usize| Some(u32::try_from(value).unwrap_or(u32::MAX));
    let mut sockets: Vec<_> = facts
        .sockets
        .iter()
        .filter(|socket| Some(socket.reference.netmap) == netmap)
        .filter_map(|socket| {
            let buffers = socket.buffers?;
            Some(meshway_protocol::ipc::SocketStatus {
                resource: socket.reference.resource.clone(),
                bound_address: Some(
                    meshway_model::SocketAddress::try_from(socket.bound_address).ok()?,
                ),
                receive_buffer: bytes(buffers.receive),
                receive_requested: bytes(buffers.receive_requested),
                send_buffer: bytes(buffers.send),
                send_requested: bytes(buffers.send_requested),
                unavailable: None,
            })
        })
        .collect();
    sockets.sort_by_key(|socket| {
        socket
            .bound_address
            .map(|address| (address.get().is_ipv6(), address.to_string()))
    });
    let mut unavailable: Vec<_> = facts
        .unavailable
        .iter()
        .map(|listener| meshway_protocol::ipc::SocketStatus {
            resource: meshway_model::state::ResourceRef::Listener {
                key: listener.key.clone(),
            },
            bound_address: None,
            receive_buffer: None,
            receive_requested: None,
            send_buffer: None,
            send_requested: None,
            unavailable: Some(listener.detail.clone()),
        })
        .collect();
    unavailable.sort_by(|a, b| a.resource.cmp(&b.resource));
    sockets.extend(unavailable);
    sockets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listener_retrying_its_bind_is_an_unavailable_socket_row() {
        let facts = meshway_connectivity::Snapshot {
            listeners: Vec::new(),
            links: Vec::new(),
            sockets: Vec::new(),
            adjacencies: Default::default(),
            mtu: Default::default(),
            unavailable: vec![meshway_connectivity::ListenerUnavailable {
                key: "lab-udp".into(),
                address: Some("0.0.0.0:35000".parse().unwrap()),
                detail: "listener lab-udp cannot bind 0.0.0.0:35000: Address in use (os error 98)"
                    .into(),
                recoverable: true,
                attempts: 2,
                retry_at: std::time::Instant::now(),
            }],
        };
        let rows = sockets(Some(&facts), None);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].resource,
            meshway_model::state::ResourceRef::Listener {
                key: "lab-udp".into()
            }
        );
        assert!(rows[0].bound_address.is_none() && rows[0].receive_buffer.is_none());
        assert!(
            rows[0]
                .unavailable
                .as_deref()
                .unwrap()
                .contains("Address in use")
        );
        assert!(sockets(None, None).is_empty());
    }

    /// F6a B10: a Control problem is `lastError`, yet the packet plane's
    /// own failure and its cause stay visible beside it; without one, the
    /// packet plane's failure is `lastError` with its detail.
    #[test]
    fn a_control_problem_keeps_the_packet_planes_cause_beside_it() {
        let revision = meshway_model::Version::MIN;
        let (_desired, desired) = watch::channel(Desired {
            revision,
            up: true,
            shutdown: false,
            retry_enrollment: false,
            enrollment_key: None,
            rotation: None,
            reenroll: false,
            logout: None,
        });
        let (reported, report) = watch::channel(Report::initial(revision));
        let mut network = super::super::network::Report::stopped();
        network.plane = DataPlane::Failed;
        network.problem = Some(Problem::Dns);
        network.problem_detail = Some("DNS resolved_link check failed".into());
        let (_network, network) = watch::channel(network);
        let (_sync, sync) = watch::channel(super::super::control::SyncStatus::default());
        let view = View::new(
            desired,
            report,
            "https://server.test".into(),
            NetworkMode::Tun,
            network,
            sync,
        );
        let status = view.status();
        assert_eq!(status.last_error, Some(Problem::Dns));
        assert_eq!(
            status.last_error_detail.as_deref(),
            Some("DNS resolved_link check failed")
        );
        assert!(status.network_error.is_none());

        reported.send_modify(|report| report.problem = Some(Problem::Credentials));
        let status = view.status();
        assert_eq!(status.last_error, Some(Problem::Credentials));
        assert!(status.last_error_detail.is_none(), "Control names no cause");
        let failure = status.network_error.expect("the packet plane's failure");
        assert_eq!(failure.problem, Problem::Dns);
        assert_eq!(
            failure.detail.as_deref(),
            Some("DNS resolved_link check failed")
        );
    }

    fn view(
        network: super::super::network::Report,
    ) -> (
        View,
        watch::Sender<Report>,
        watch::Sender<super::super::network::Report>,
        watch::Sender<Desired>,
    ) {
        let revision = meshway_model::Version::MIN;
        let (desired_tx, desired) = watch::channel(Desired {
            revision,
            up: true,
            shutdown: false,
            retry_enrollment: false,
            enrollment_key: None,
            rotation: None,
            reenroll: false,
            logout: None,
        });
        let (reported, report) = watch::channel(Report::initial(revision));
        let (network_tx, network) = watch::channel(network);
        let (_sync, sync) = watch::channel(super::super::control::SyncStatus::default());
        let view = View::new(
            desired,
            report,
            "https://server.test".into(),
            NetworkMode::Tun,
            network,
            sync,
        );
        (view, reported, network_tx, desired_tx)
    }

    /// H4 (S4 decision 2, OPS17 ③): `observedAt` is when the shown packet
    /// plane state last changed; Control republishing the same phase moves
    /// only `controlObservedAt`.
    #[tokio::test(flavor = "current_thread")]
    async fn observed_at_is_the_last_change_of_the_shown_state() {
        let (view, reported, network, _desired) = view(super::super::network::Report::stopped());
        let (stop, stopped) = watch::channel(false);
        let clock = tokio::spawn(view.clone().track(stopped));
        tokio::task::yield_now().await;
        let first = view.status();
        assert_eq!(first.phase, Phase::Starting);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        // Control republishes the same phase (a retried sync failure).
        reported.send_modify(|report| report.observed_at = Utc::now());
        tokio::task::yield_now().await;
        let again = view.status();
        assert_eq!(
            again.observed_at, first.observed_at,
            "no change, no restamp"
        );
        assert!(again.control_observed_at > first.control_observed_at);
        // A change is stamped when it happens, read or not.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let before = Utc::now();
        reported.send_modify(|report| report.phase = Phase::Synchronizing);
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let changed = view.status();
        assert_eq!(changed.phase, Phase::Synchronizing);
        assert!(
            changed.observed_at >= before
                && changed.observed_at < Utc::now() - chrono::Duration::milliseconds(10),
            "stamped by the tracker at the change, not by this read"
        );
        network.send_modify(|report| report.plane = DataPlane::Preparing);
        tokio::task::yield_now().await;
        assert!(view.status().observed_at > changed.observed_at);
        stop.send_replace(true);
        clock.await.unwrap();
    }

    /// X18: the tracker sleeps while no input changes. The DNS owner's
    /// guard receiver is only borrowed, so a report carries it with every
    /// send since the owner started unseen (each hold sends `None`); the
    /// tracker still must not wake for those, only for a later send.
    #[tokio::test(flavor = "current_thread")]
    async fn the_tracker_is_not_woken_by_a_dns_guard_send_it_has_seen() {
        let mut network = super::super::network::Report::stopped();
        network.plane = DataPlane::Ready;
        network.completion = Some(Ok(()));
        network.content = Some(std::sync::Arc::new(meshway_test_server::single_node(
            "a", None,
        )));
        let (fault, guard) = watch::channel(None);
        // A hold before the report was published: the receiver every
        // report clones from has not seen it.
        fault.send_replace(None);
        assert!(guard.has_changed().unwrap());
        network.dns_guard = Some(guard);
        let (view, _reported, _network, _desired) = view(network);
        let (stop, stopped) = watch::channel(false);
        let polls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = polls.clone();
        let mut track = Box::pin(view.clone().track(stopped));
        let clock = tokio::spawn(std::future::poll_fn(move |cx| {
            counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            track.as_mut().poll(cx)
        }));
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }
        let idle = polls.load(std::sync::atomic::Ordering::Relaxed);
        assert!(
            idle <= 2,
            "an unchanged input woke the tracker {idle} times"
        );
        // A later fault is a change: one turn reads it, then it sleeps again.
        fault.send_replace(Some(meshway_dns::SystemDnsError::Backend));
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }
        let woken = polls.load(std::sync::atomic::Ordering::Relaxed) - idle;
        assert!((1..=2).contains(&woken), "one change, {woken} turns");
        stop.send_replace(true);
        clock.await.unwrap();
    }

    /// H4 (OPS17 ②): the DNS owner's fault before the worker withdrew for
    /// it already is the packet plane's cause: `suspended` never shows
    /// with a null `lastError`.
    #[test]
    fn a_dns_fault_the_worker_has_not_reported_is_already_the_cause() {
        let mut network = super::super::network::Report::stopped();
        network.plane = DataPlane::Ready;
        network.completion = Some(Ok(()));
        network.content = Some(std::sync::Arc::new(meshway_test_server::single_node(
            "a", None,
        )));
        let (fault, guard) = watch::channel(None);
        let (detail, details) = watch::channel(None);
        network.dns_guard = Some(guard);
        network.dns_detail = Some(details);
        let (view, _reported, _network, _desired) = view(network);
        assert_eq!(view.status().last_error, None);
        fault.send_replace(Some(meshway_dns::SystemDnsError::Backend));
        detail.send_replace(Some(
            meshway_dns::SystemDnsFailure::new(
                meshway_dns::SystemDnsError::Backend,
                "observe_deadline",
            )
            .expected("an observation within 2000 ms")
            .actual("abandoned after 2101 ms"),
        ));
        let status = view.status();
        assert_eq!(status.data_plane, DataPlane::Suspended);
        assert!(status.applied.is_none());
        assert_eq!(status.last_error, Some(Problem::Dns));
        assert!(
            status
                .last_error_detail
                .as_deref()
                .is_some_and(|detail| detail.contains("observe_deadline")),
            "{:?}",
            status.last_error_detail
        );
    }

    /// H4 decision 1: a failed plane whose failure recovers by itself is
    /// `retrying`; `blocked` only when nothing brings it back (a proven
    /// ownership conflict, retries spent), unless a kernel change or exit
    /// cleanup is being retried right now.
    #[test]
    fn a_transient_failure_retries_and_only_a_persistent_one_blocks() {
        let mut network = super::super::network::Report::stopped();
        network.plane = DataPlane::Failed;
        network.problem = Some(Problem::Dns);
        let (view, reported, network, _desired) = view(network);
        reported.send_modify(|report| report.phase = Phase::ControlReady);
        let status = view.status();
        assert_eq!(
            (status.phase, status.data_plane),
            (Phase::Retrying, DataPlane::Failed)
        );
        network.send_modify(|report| {
            report.problem = Some(Problem::Underlay);
            report.persistent = true;
        });
        assert_eq!(view.status().phase, Phase::Blocked);
        let (cleanup, cleanups) = watch::channel(None);
        network.send_modify(|report| report.exit_cleanups = Some(cleanups));
        assert_eq!(view.status().phase, Phase::Blocked);
        cleanup.send_replace(Some(meshway_underlay::RetryPending {
            reason: "kernel netlink operation failed".into(),
            attempts: 2,
            retry_at: std::time::SystemTime::now(),
        }));
        assert_eq!(
            view.status().phase,
            Phase::Retrying,
            "a cleanup being retried"
        );
    }
}
