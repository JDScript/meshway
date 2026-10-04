//! Project the coordinator's intent and the control owner's expiring snapshot.
use super::intent::Desired;
use chrono::{DateTime, Utc};
use meshway_control::StatusSnapshot;
use meshway_ipc::{DataPlane, Phase, Problem, Status};
use meshway_model::Version;
use tokio::sync::watch;

#[derive(Clone)]
pub(super) struct Report {
    pub revision: Version,
    pub phase: Phase,
    pub control: Option<StatusSnapshot>,
    pub enrollment_url: Option<String>,
    pub problem: Option<Problem>,
    pub retry_at: Option<DateTime<Utc>>,
    pub observed_at: DateTime<Utc>,
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
        }
    }
}
#[derive(Clone)]
pub(crate) struct View {
    pub(super) desired: watch::Receiver<Desired>,
    pub(super) report: watch::Receiver<Report>,
    pub(super) server: String,
}
impl View {
    pub fn status(&self) -> Status {
        let mut desired = self.desired.borrow().clone();
        desired.shutdown |= self.desired.has_changed().is_err();
        let report = self.report.borrow().clone();
        let control = report.control.as_ref().map(StatusSnapshot::status);
        let current = report.revision == desired.revision;
        let phase = if desired.shutdown {
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
        } else if report.phase == Phase::ControlReady
            && control
                .as_ref()
                .is_none_or(|c| !c.authenticated || c.instance.is_none())
        {
            Phase::Synchronizing
        } else {
            report.phase
        };
        Status {
            process_id: std::process::id(),
            revision: desired.revision,
            desired_up: desired.up,
            phase,
            server: self.server.clone(),
            observed_at: report.observed_at,
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
            instance: if current && desired.up && !desired.shutdown {
                control.as_ref().and_then(|c| c.instance.clone())
            } else {
                None
            },
            saved: control.and_then(|c| c.cached),
            applied: None,
            data_plane: DataPlane::NotImplemented,
            last_error: report.problem,
            retry_at: if current && desired.up {
                report.retry_at
            } else {
                None
            },
        }
    }
}
