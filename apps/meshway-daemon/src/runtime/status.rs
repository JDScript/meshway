//! Project the coordinator's intent and the control owner's expiring snapshot.
use super::intent::Desired;
use chrono::{DateTime, Utc};
use meshway_control::StatusSnapshot;
use meshway_ipc::{DataPlane, NetworkMode, Phase, Problem, Status};
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
    pub(super) events: watch::Receiver<super::control::events::EventStatus>,
}
impl View {
    pub fn status(&self) -> Status {
        let mut desired = self.desired.borrow().clone();
        desired.shutdown |= self.desired.has_changed().is_err();
        let report = self.report.borrow().clone();
        let control = report.control.as_ref().map(StatusSnapshot::status);
        let current = report.revision == desired.revision;
        let network = self.network.borrow().clone();
        let events = self.events.borrow().clone();
        let network_bound = control
            .as_ref()
            .is_some_and(|control| network.bound_to(control));
        let network_current = current
            && desired.up
            && !desired.shutdown
            && control
                .as_ref()
                .is_some_and(|control| network.current(control));
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
        } else if network_current
            || (report.phase != Phase::Blocked
                && control
                    .as_ref()
                    .is_some_and(|control| network.same_instance(control)))
        {
            // Component installation state is distinct from execution authority.
            // A target handoff may invalidate output while the installed TUN is
            // retained. The worker alone reports actual preparing/suspended.
            network.plane
        } else if report.phase == Phase::Blocked
            || (network_bound && network.plane == DataPlane::Failed)
        {
            DataPlane::Failed
        } else if !network_bound
            && control
                .as_ref()
                .is_some_and(|control| control.instance.is_some())
        {
            DataPlane::Preparing
        } else if network.plane == DataPlane::Ready {
            DataPlane::Suspended
        } else {
            network.plane
        };
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
        } else if matches!(report.phase, Phase::ControlReady | Phase::Running)
            && control
                .as_ref()
                .is_none_or(|c| !c.authenticated || c.instance.is_none())
        {
            Phase::Synchronizing
        } else {
            report.phase
        };
        if current && desired.up && !desired.shutdown && self.network_mode == NetworkMode::Tun {
            if network_current && data_plane == DataPlane::Ready && phase == Phase::ControlReady {
                phase = Phase::Running;
            } else if data_plane == DataPlane::Failed
                && matches!(phase, Phase::ControlReady | Phase::Running)
            {
                phase = Phase::Blocked;
            } else if matches!(phase, Phase::ControlReady | Phase::Running)
                && !(network_current
                    && data_plane == DataPlane::Stopped
                    && network.applied().is_some())
            {
                phase = Phase::Synchronizing;
            }
        }
        let exit = (current
            && network_bound
            && desired.up
            && !desired.shutdown
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
                            .guard
                            .as_ref()
                            .and_then(|g| g.configuration().ok())
                            .and_then(|cfg| cfg.exit_role.as_ref())
                            .filter(|_| {
                                network
                                    .exit_capability
                                    .as_ref()
                                    .is_some_and(|c| !c.installed_families.is_empty())
                            })
                            .map(|role| role.generation)
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
            authenticated: current
                && desired.up
                && !desired.shutdown
                && control.as_ref().is_some_and(|c| c.authenticated),
            event_stream: if desired.up && !desired.shutdown {
                events.state
            } else {
                meshway_ipc::EventStreamState::Disconnected
            },
            last_event_at: events.last_event_at,
            instance: if current && desired.up && !desired.shutdown {
                control.as_ref().and_then(|c| c.instance.clone())
            } else {
                None
            },
            saved: control.and_then(|c| c.cached),
            applied: if network_current {
                network.applied()
            } else {
                None
            },
            data_plane,
            network_mode: self.network_mode,
            exit,
            last_error: report.problem.or(if current && network_bound {
                network.problem
            } else {
                None
            }),
            retry_at: if current && desired.up {
                report.retry_at
            } else {
                None
            },
        }
    }
}
