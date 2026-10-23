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
    /// Why, when Control names the cause of `problem`.
    pub problem_detail: Option<String>,
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
            problem_detail: None,
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

/// The live owner facts a network report carries, as the tracker waits on
/// them. Each is a view its holders only borrow, so its own seen version
/// lags behind every send since the owner started: the tracker's clones are
/// marked seen before the inputs are read, else `changed` stays ready and a
/// turn never yields. A closed one is dropped (one more turn reads it).
struct Owners {
    dns: Option<watch::Receiver<super::network::DnsFact>>,
    ownership: Option<watch::Receiver<meshway_model::recovery::Health>>,
    relay: Option<watch::Receiver<Option<meshway_underlay::ControlRelayState>>>,
    changes: Option<watch::Receiver<Option<meshway_underlay::KernelChangePending>>>,
    cleanups: Option<watch::Receiver<Option<meshway_underlay::RetryPending>>>,
}

impl Owners {
    fn of(network: &super::network::Report) -> Self {
        fn seen<T>(receiver: &Option<watch::Receiver<T>>) -> Option<watch::Receiver<T>> {
            let mut receiver = receiver
                .clone()
                .filter(|receiver| receiver.has_changed().is_ok())?;
            receiver.mark_unchanged();
            Some(receiver)
        }
        Self {
            dns: seen(&network.dns_guard),
            ownership: seen(&network.ownership),
            relay: seen(&network.control_relay),
            changes: seen(&network.kernel_changes),
            cleanups: seen(&network.exit_cleanups),
        }
    }

    async fn changed(mut self) {
        async fn one<T>(receiver: &mut Option<watch::Receiver<T>>) {
            match receiver {
                Some(receiver) => {
                    let _ = receiver.changed().await;
                }
                None => std::future::pending().await,
            }
        }
        tokio::select! {
            _ = one(&mut self.dns) => {},
            _ = one(&mut self.ownership) => {},
            _ = one(&mut self.relay) => {},
            _ = one(&mut self.changes) => {},
            _ = one(&mut self.cleanups) => {},
        }
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
            let owners = Owners::of(&self.network.borrow());
            let derived = self.derive();
            self.observe(derived.plane);
            tokio::select! {
                _ = stop.wait_for(|stopped| *stopped) => return,
                changed = self.desired.changed() => if changed.is_err() { return },
                changed = self.report.changed() => if changed.is_err() { return },
                changed = self.network.changed() => if changed.is_err() { return },
                () = owners.changed() => {},
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
                // A failure its owner retries is `retrying`; `blocked` comes
                // from an owner's proven inconsistency alone (below).
                phase = Phase::Retrying;
            } else if !ready && phase == Phase::ControlReady {
                phase = Phase::Synchronizing;
            }
        }
        if active && phase == Phase::ControlReady && lease.is_none() {
            phase = Phase::Synchronizing;
        }
        // `blocked` exactly while an owner holds a proven inconsistency (or
        // a refused netmap), whatever else runs; it ends with that proof.
        if active
            && self.network_mode == NetworkMode::Tun
            && !matches!(phase, Phase::Blocked | Phase::Leaving)
            && owners(&network)
                .iter()
                .any(|owner| owner.health.is_blocked())
        {
            phase = Phase::Blocked;
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
        let sockets = sockets(
            content.and(network.facts.as_ref()),
            &content
                .map(|_| network.unavailable_listeners())
                .unwrap_or_default(),
            network.netmap,
        );
        // An owner's proven inconsistency, or the cause of a withdrawn
        // plane (kernel ownership first, else the earliest), from the
        // owners' own live facts; otherwise the worker's last outcome. The
        // DNS owner's fault the worker has not withdrawn for yet is already
        // the cause: the plane withdraws for it.
        let owned = owners(&network);
        let blocked = owned.iter().any(|owner| owner.health.is_blocked());
        let owned = cause(&owned, blocked)
            .filter(|_| blocked || data_plane == DataPlane::Failed)
            .map(|owner| (owner.problem, owner.detail.clone()));
        let network_failure = match (owned, network.problem) {
            (Some(owned), _) => Some(owned),
            (None, Some(problem)) => Some((problem, network.problem_detail.clone())),
            (None, None) => network.dns_fault().map(|detail| (Problem::Dns, detail)),
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
            // Each cause stays with its own failure: Control's when it is
            // `lastError`; the packet plane's when it is, else beside it.
            last_error_detail: match report.problem {
                Some(_) => report.problem_detail.clone(),
                None => network_error
                    .as_ref()
                    .and_then(|failure| failure.detail.clone()),
            },
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
                    // Blocked shows as retrying: the owner reads it again
                    // every 60 s; `reason` names the foreign cause.
                    meshway_underlay::ControlRelayState::Retrying(retry) => {
                        meshway_protocol::ipc::ControlRelay::retrying(retry_pending(retry))
                    }
                    meshway_underlay::ControlRelayState::Blocked(blocked) => {
                        meshway_protocol::ipc::ControlRelay::retrying(retry_pending(
                            meshway_underlay::RetryPending {
                                retry_at: blocked.reread_at.map_or_else(
                                    std::time::SystemTime::now,
                                    meshway_model::recovery::wall,
                                ),
                                reason: blocked.reason,
                                attempts: blocked.attempts,
                            },
                        ))
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
            local_dns_domains: network
                .dns_guard
                .as_ref()
                // The installed system plan's, whatever Control holds now.
                .filter(|_| active)
                .map(|guard| guard.borrow().local_domains.clone())
                .filter(|domains| !domains.is_empty()),
        }
    }
}

/// One fail-closed owner's health, as Status reads it: the problem it is
/// reported as and why. Read from the owners' own live facts (kernel
/// ownership, the DNS owner, the Control relay, the listeners, the worker's
/// own installation retry), never from a snapshot of them.
struct Owned {
    problem: Problem,
    health: meshway_model::recovery::Health,
    detail: Option<String>,
    /// Kernel ownership: every other owner fails because it did.
    kernel: bool,
}

/// Every owner that is not healthy now (see [`Owned`]).
fn owners(network: &super::network::Report) -> Vec<Owned> {
    use meshway_model::recovery::Health;
    let mut owners = Vec::new();
    let reason = |health: &Health| match health {
        Health::Healthy => None,
        Health::Retrying(retrying) => Some(retrying.reason.clone()),
        Health::Blocked(blocked) => Some(blocked.reason.clone()),
    };
    if let Some(ownership) = network
        .ownership
        .as_ref()
        .filter(|ownership| ownership.has_changed().is_ok())
    {
        let health = ownership.borrow().clone();
        if !health.is_healthy() {
            owners.push(Owned {
                problem: Problem::Underlay,
                detail: reason(&health),
                health,
                kernel: true,
            });
        }
    }
    if let Some(guard) = network
        .dns_guard
        .as_ref()
        .filter(|guard| guard.has_changed().is_ok())
    {
        let fact = guard.borrow();
        if fact.fault.is_some() && !fact.health.is_healthy() {
            owners.push(Owned {
                problem: Problem::Dns,
                health: fact.health.clone(),
                detail: fact.detail.as_ref().map(ToString::to_string),
                kernel: false,
            });
        }
    }
    // The owners' own records: when each failure began, not this read.
    if let Some(Some(meshway_underlay::ControlRelayState::Blocked(blocked))) = network
        .control_relay
        .as_ref()
        .map(|relay| relay.borrow().clone())
    {
        owners.push(Owned {
            problem: Problem::Underlay,
            detail: Some(format!("controlRelay: {}", blocked.reason)),
            health: Health::Blocked(blocked),
            kernel: false,
        });
    }
    for listener in network
        .unavailable_listeners()
        .into_iter()
        .filter(|listener| listener.health.is_blocked())
    {
        owners.push(Owned {
            problem: Problem::Transport,
            health: listener.health,
            detail: Some(listener.detail),
            kernel: false,
        });
    }
    if !network.apply.is_healthy() {
        owners.push(Owned {
            problem: network.problem.unwrap_or(Problem::Apply),
            health: network.apply.clone(),
            detail: network.problem_detail.clone(),
            kernel: false,
        });
    }
    owners
}

/// The cause Status shows among `owners`: kernel ownership first (every
/// other owner failed because it did), else the earliest failure.
fn cause(owners: &[Owned], blocked: bool) -> Option<&Owned> {
    owners
        .iter()
        .filter(|owner| !blocked || owner.health.is_blocked())
        .min_by_key(|owner| (!owner.kernel, owner.health.since()))
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
    unavailable: &[meshway_connectivity::ListenerUnavailable],
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
    let mut unavailable: Vec<_> = unavailable
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
                attempts: 2,
                retry_at: std::time::Instant::now(),
                health: retrying("held"),
            }],
        };
        let rows = sockets(Some(&facts), &facts.unavailable, None);
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
        assert!(sockets(None, &[], None).is_empty());
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
        let (fault, guard) = watch::channel(super::super::network::DnsFact::default());
        // A hold before the report was published: the receiver every
        // report clones from has not seen it.
        fault.send_replace(super::super::network::DnsFact::default());
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
        fault.send_modify(|fact| fact.fault = Some(meshway_dns::SystemDnsError::Backend));
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
        let (fault, guard) = watch::channel(super::super::network::DnsFact::default());
        network.dns_guard = Some(guard);
        let (view, _reported, _network, _desired) = view(network);
        assert_eq!(view.status().last_error, None);
        fault.send_modify(|fact| {
            fact.fault = Some(meshway_dns::SystemDnsError::Backend);
            fact.detail = Some(
                meshway_dns::SystemDnsFailure::new(
                    meshway_dns::SystemDnsError::Backend,
                    "observe_deadline",
                )
                .expected("an observation within 2000 ms")
                .actual("abandoned after 2101 ms"),
            );
        });
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

    /// D5: the DNS owner's coexisting local domains are Status's, live, and
    /// omitted while there are none.
    #[test]
    fn local_dns_domains_are_read_live_from_the_dns_owner() {
        let mut network = super::super::network::Report::stopped();
        network.plane = DataPlane::Ready;
        network.completion = Some(Ok(()));
        network.content = Some(std::sync::Arc::new(meshway_test_server::single_node(
            "a", None,
        )));
        let (fact, guard) = watch::channel(super::super::network::DnsFact::default());
        network.dns_guard = Some(guard);
        let (view, _reported, _network, _desired) = view(network);
        assert_eq!(view.status().local_dns_domains, None);
        let local = meshway_protocol::ipc::LocalDnsDomain {
            domain: "tko.oraclevcn.com".into(),
            link_index: 2,
            link_name: Some("enp0s6".into()),
        };
        fact.send_modify(|fact| fact.local_domains = vec![local.clone()]);
        assert_eq!(view.status().local_dns_domains, Some(vec![local]));
        fact.send_modify(|fact| fact.local_domains.clear());
        assert_eq!(view.status().local_dns_domains, None);
    }

    fn retrying(reason: &str) -> meshway_model::recovery::Health {
        meshway_model::recovery::Health::Retrying(meshway_model::recovery::Retrying {
            check: "test",
            reason: reason.into(),
            since: std::time::Instant::now(),
            attempts: 1,
            retry_at: std::time::Instant::now() + std::time::Duration::from_secs(5),
        })
    }

    fn blocked(reason: &str) -> meshway_model::recovery::Health {
        meshway_model::recovery::Health::Blocked(meshway_model::recovery::Blocked {
            check: "test",
            reason: reason.into(),
            since: std::time::Instant::now(),
            attempts: 1,
            reread_at: Some(std::time::Instant::now() + std::time::Duration::from_secs(60)),
        })
    }

    /// RC-1 M8 (Q3, G1, G8, G12): `phase` follows every owner's live health.
    /// A withdrawn plane whose owner retries is `retrying`, never running
    /// out into `blocked`; `blocked` exactly while an owner holds a proven
    /// inconsistency (kernel ownership's conflict, foreign DNS settings, a
    /// foreign relay socket) or a refused netmap, read live, not from the
    /// worker's last report; the cause is kernel ownership first (Q7).
    #[test]
    fn a_transient_failure_retries_and_only_a_proven_one_blocks() {
        let mut network = super::super::network::Report::stopped();
        network.plane = DataPlane::Failed;
        network.problem = Some(Problem::Tun);
        network.apply = retrying("Tun");
        let (ownership, ownerships) = watch::channel(meshway_model::recovery::Health::Healthy);
        network.ownership = Some(ownerships);
        let (dns, guard) = watch::channel(super::super::network::DnsFact::default());
        network.dns_guard = Some(guard);
        let (relay, relays) = watch::channel(None);
        network.control_relay = Some(relays);
        let (view, reported, network, _desired) = view(network);
        reported.send_modify(|report| report.phase = Phase::ControlReady);
        let status = view.status();
        assert_eq!(
            (status.phase, status.data_plane, status.last_error),
            (Phase::Retrying, DataPlane::Failed, Some(Problem::Tun)),
            "retried without running out"
        );
        // Kernel ownership ends unavailable, then is proven a conflict: the
        // live health, not a report, makes it blocked.
        ownership.send_replace(retrying(
            "kernelOwnership: unavailable (monitor_unavailable)",
        ));
        dns.send_modify(|fact| {
            fact.fault = Some(meshway_dns::SystemDnsError::Backend);
            fact.health = retrying("anchor_ended");
        });
        let status = view.status();
        assert_eq!(status.phase, Phase::Retrying);
        assert_eq!(status.last_error, Some(Problem::Underlay), "kernel first");
        ownership.send_replace(blocked("kernelOwnership: conflict (monitor)"));
        let status = view.status();
        assert_eq!(
            (status.phase, status.last_error),
            (Phase::Blocked, Some(Problem::Underlay))
        );
        assert!(
            status
                .last_error_detail
                .as_deref()
                .is_some_and(|detail| detail.contains("conflict (monitor)")),
            "{:?}",
            status.last_error_detail
        );
        // The conflict read again and gone: retrying again; then foreign DNS
        // link settings are proven on their own.
        ownership.send_replace(meshway_model::recovery::Health::Healthy);
        assert_eq!(view.status().phase, Phase::Retrying);
        dns.send_modify(|fact| fact.health = blocked("DNS link_settings check failed"));
        let status = view.status();
        assert_eq!(
            (status.phase, status.last_error),
            (Phase::Blocked, Some(Problem::Dns))
        );
        dns.send_replace(super::super::network::DnsFact::default());
        // A refused netmap blocks until another offer.
        network.send_modify(|report| {
            report.problem = Some(Problem::Apply);
            report.apply = blocked("Validation");
        });
        assert_eq!(view.status().phase, Phase::Blocked);
        // A running plane with a foreign socket on the relay path (Q9).
        network.send_modify(|report| {
            report.plane = DataPlane::Ready;
            report.problem = None;
            report.apply = meshway_model::recovery::Health::Healthy;
        });
        assert_ne!(view.status().phase, Phase::Blocked);
        let meshway_model::recovery::Health::Blocked(foreign) =
            blocked("cannot listen: a socket on the relay path this owner cannot prove its own")
        else {
            unreachable!()
        };
        relay.send_replace(Some(meshway_underlay::ControlRelayState::Blocked(foreign)));
        let status = view.status();
        assert_eq!(status.phase, Phase::Blocked);
        assert_eq!(
            status.control_relay.map(|relay| relay.state),
            Some(meshway_protocol::ipc::ControlRelayState::Retrying),
            "the IPC shape is unchanged"
        );
        assert!(
            status
                .last_error_detail
                .as_deref()
                .is_some_and(|detail| detail.starts_with("controlRelay: ")),
            "{:?}",
            status.last_error_detail
        );
    }

    /// F8a F7: Status projects the relay's and listeners' own health: when
    /// each failure began (the earliest is the cause, read after read) and
    /// a listener's retry owner live, without a new worker report.
    #[test]
    fn owners_keep_when_their_failure_began_and_listeners_are_read_live() {
        let mut network = super::super::network::Report::stopped();
        network.plane = DataPlane::Ready;
        let (relay, relays) = watch::channel(None);
        network.control_relay = Some(relays);
        let (listeners, live) = watch::channel(Vec::new());
        network.listeners = Some(live);
        let (view, reported, _network, _desired) = view(network);
        reported.send_modify(|report| report.phase = Phase::ControlReady);
        let listener = |health| meshway_connectivity::ListenerUnavailable {
            key: "lab-tcp".into(),
            address: Some("0.0.0.0:443".parse().unwrap()),
            detail: "listener lab-tcp cannot bind 0.0.0.0:443: Permission denied".into(),
            attempts: 1,
            retry_at: std::time::Instant::now(),
            health,
        };
        // The listener's permission failure began first, the relay's later.
        listeners.send_replace(vec![listener(blocked("permission"))]);
        std::thread::sleep(std::time::Duration::from_millis(5));
        let meshway_model::recovery::Health::Blocked(foreign) = blocked("foreign socket") else {
            unreachable!()
        };
        relay.send_replace(Some(meshway_underlay::ControlRelayState::Blocked(foreign)));
        for _ in 0..2 {
            let status = view.status();
            assert_eq!(
                (status.phase, status.last_error),
                (Phase::Blocked, Some(Problem::Transport)),
                "the earliest failure is the cause"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // The retry owner bound it: read live.
        listeners.send_replace(Vec::new());
        let status = view.status();
        assert_eq!(status.last_error, Some(Problem::Underlay));
        relay.send_replace(None);
        assert_ne!(view.status().phase, Phase::Blocked);
    }
}
