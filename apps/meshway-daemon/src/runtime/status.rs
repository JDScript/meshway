//! Project the coordinator's intent and the control owner's expiring snapshot.
use super::intent::Desired;
use chrono::{DateTime, Utc};
use meshway_control::StatusSnapshot;
use meshway_model::Version;
use meshway_protocol::ipc::{DataPlane, NetworkMode, Phase, Problem, Status, SyncState};
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
#[derive(Clone)]
pub(crate) struct View {
    pub(super) desired: watch::Receiver<Desired>,
    pub(super) report: watch::Receiver<Report>,
    pub(super) server: String,
    pub(super) network_mode: NetworkMode,
    pub(super) network: watch::Receiver<super::network::Report>,
    pub(super) sync: watch::Receiver<super::control::SyncStatus>,
}
impl View {
    pub fn status(&self) -> Status {
        let mut desired = self.desired.borrow().clone();
        desired.shutdown |= self.desired.has_changed().is_err();
        let report = self.report.borrow().clone();
        let control = report.control.as_ref().map(StatusSnapshot::status);
        let current = report.revision == desired.revision;
        let network = self.network.borrow().clone();
        let sync = self.sync.borrow().clone();
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
                phase = Phase::Blocked;
            } else if !ready && phase == Phase::ControlReady {
                phase = Phase::Synchronizing;
            }
        }
        if active && phase == Phase::ControlReady && lease.is_none() {
            phase = Phase::Synchronizing;
        }
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
                    capability.installed_families.clear();
                }
                capability
            }),
            installed_role: network_current
                .then(|| {
                    network
                        .exit_capability
                        .as_ref()
                        .filter(|c| !c.installed_families.is_empty())
                        .map(|c| c.role_generation)
                })
                .flatten(),
            active_flows: network_current.then_some(network.exit_flows).flatten(),
            dns_unavailable: exit_dns.and_then(|fact| match fact.status {
                meshway_model::exit::ExitDnsStatus::Serving => None,
                meshway_model::exit::ExitDnsStatus::Unavailable { reason } => Some(reason),
            }),
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
        Status {
            process_id: std::process::id(),
            revision: desired.revision,
            desired_up: desired.up,
            phase,
            server: self.server.clone(),
            observed_at: if network_current {
                report.observed_at.max(network.observed_at)
            } else {
                report.observed_at
            },
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
            last_error: report
                .problem
                .or(if active { network.problem } else { None }),
            last_error_detail: if report.problem.is_none() && active {
                network
                    .problem_detail
                    .clone()
                    .filter(|_| network.problem.is_some())
            } else {
                None
            },
            retry_at: if current && desired.up {
                report.retry_at
            } else {
                None
            },
            last_network_change: network.last_network_change,
        }
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
}
