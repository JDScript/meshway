//! Project the coordinator's intent and the control owner's expiring snapshot.
use super::intent::Desired;
use chrono::{DateTime, Utc};
use meshway_control::StatusSnapshot;
use meshway_ipc::{DataPlane, NetworkMode, Phase, Problem, Status, SyncState};
use meshway_model::Version;
use tokio::sync::watch;

#[derive(Clone, Copy)]
pub(super) struct RotationOutcome {
    pub id: meshway_model::RequestId,
    pub result: Result<(), meshway_ipc::ErrorCode>,
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
            if matches!(report.phase, Phase::Starting | Phase::Blocked) {
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
        let exit = (active
            && network.netmap.is_some()
            && (network.exit_capability.is_some() || network.exit_dns_unavailable))
            .then(|| meshway_ipc::ExitState {
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
                dns_unavailable: network
                    .exit_dns_unavailable
                    .then_some(meshway_model::exit::ExitUnavailableReason::DnsUnavailable),
            });
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
            instance: if active {
                control.as_ref().and_then(|c| c.instance.clone())
            } else {
                None
            },
            netmap: lease.filter(|_| active).and_then(|lease| {
                Some(meshway_ipc::NetmapStatus {
                    netmap: lease.netmap,
                    issued_at: timestamp(lease.issued_at_ms)?,
                    expires_at: timestamp(lease.expires_at_ms)?,
                })
            }),
            saved: control.and_then(|c| c.held),
            applied,
            data_plane,
            network_mode: self.network_mode,
            exit,
            last_error: report
                .problem
                .or(if active { network.problem } else { None }),
            retry_at: if current && desired.up {
                report.retry_at
            } else {
                None
            },
        }
    }
}

fn timestamp(ms: u64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(i64::try_from(ms).ok()?)
}
