//! Runtime facts and ordered installation receipts use the same credential,
//! sequence and retry owners as control-only reporting.
use crate::{Control, Error, now};
use meshway_model::{NetmapRef, Sequence, Version, state::NodeState};
use meshway_protocol::{
    InstanceRequest, ReceiptRequest, StateAccepted, StateReport,
    execution::{ApplyFailure, ReceiptPhase},
};
use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

struct Attempt {
    owner: Arc<()>,
    saved: ReceiptRequest,
    saved_acknowledged: AtomicBool,
    rejected: AtomicBool,
    completion: Mutex<Option<ReceiptRequest>>,
}

/// A saved, instance-bound application attempt of one netmap, issued by the
/// one Control owner.
/// It records retry identity, not proof of installation. The runtime supplies
/// actual completion facts; cloning never creates another attempt/sample.
#[derive(Clone)]
pub struct ApplicationTicket(Arc<Attempt>);

impl ApplicationTicket {
    pub fn netmap(&self) -> &NetmapRef {
        &self.0.saved.netmap
    }
    pub fn attempt(&self) -> Version {
        self.0.saved.attempt
    }
    pub fn instance(&self) -> &InstanceRequest {
        &self.0.saved.instance
    }
}
impl fmt::Debug for ApplicationTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApplicationTicket")
            .field("netmap", self.netmap())
            .field("attempt", &self.attempt())
            .finish_non_exhaustive()
    }
}

impl Control {
    /// Confirm saved for this exact attempt of the held netmap before
    /// installation starts. An interrupted waiter retries the same saved
    /// request and timestamp. A new instance starts a new attempt for the
    /// same, already installed netmap.
    pub async fn begin_application(
        &mut self,
        netmap: NetmapRef,
    ) -> Result<ApplicationTicket, Error> {
        let instance = self.current_instance()?;
        if self.netmap_reference() != Some(netmap) {
            return Err(Error::Lifecycle);
        }
        let reusable = self.application.as_ref().filter(|ticket| {
            ticket.0.saved.instance == instance && ticket.0.saved.netmap == netmap
        });
        let next = match reusable {
            Some(ticket)
                if !ticket.0.rejected.load(Ordering::Acquire)
                    && ticket
                        .0
                        .completion
                        .lock()
                        .map_err(|_| Error::Lifecycle)?
                        .is_none() =>
            {
                Some(ticket.clone())
            }
            _ => None,
        };
        let ticket = if let Some(ticket) = next {
            ticket
        } else {
            let attempt = match reusable {
                Some(ticket) => ticket
                    .attempt()
                    .checked_next()
                    .map_err(|_| Error::Lifecycle)?,
                None => Version::MIN,
            };
            let ticket = ApplicationTicket(Arc::new(Attempt {
                owner: self.application_owner.clone(),
                saved: ReceiptRequest {
                    instance,
                    netmap,
                    phase: ReceiptPhase::Saved,
                    attempt,
                    sequence: Sequence::MIN,
                    reported_at_ms: now()?,
                    failure: None,
                },
                saved_acknowledged: AtomicBool::new(false),
                rejected: AtomicBool::new(false),
                completion: Mutex::new(None),
            }));
            self.application = Some(ticket.clone());
            ticket
        };
        if !ticket.0.saved_acknowledged.load(Ordering::Acquire) {
            let response = self
                .client
                .submit_node_configuration_receipt(&ticket.0.saved, self.token()?)
                .await;
            if let Err(error) = self.receipt_response(response) {
                if matches!(error, Error::ReceiptRejected(_)) {
                    ticket.0.rejected.store(true, Ordering::Release);
                }
                return Err(error);
            }
            if self.current_instance()? != ticket.0.saved.instance {
                return Err(Error::Lifecycle);
            }
            ticket.0.saved_acknowledged.store(true, Ordering::Release);
        }
        Ok(ticket)
    }

    /// Preserve the first actual completion's timestamp and sequence through
    /// uncertain delivery. Applied requires live authority for that netmap.
    pub async fn finish_application(
        &mut self,
        ticket: &ApplicationTicket,
        result: Result<(), ApplyFailure>,
        observed_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), Error> {
        let current = self.current_instance()?;
        if ticket.0.rejected.load(Ordering::Acquire) {
            return Err(Error::Pending);
        }
        if !Arc::ptr_eq(&ticket.0.owner, &self.application_owner)
            || ticket.instance() != &current
            || !ticket.0.saved_acknowledged.load(Ordering::Acquire)
        {
            return Err(Error::Lifecycle);
        }
        let (phase, failure) = match result {
            Ok(()) => (ReceiptPhase::Applied, None),
            Err(failure) => (ReceiptPhase::Failed, Some(failure)),
        };
        if phase == ReceiptPhase::Applied {
            self.held_netmap(*ticket.netmap())
                .map_err(|_| Error::Lifecycle)?;
        }
        let request = {
            let mut completion = ticket.0.completion.lock().map_err(|_| Error::Lifecycle)?;
            if let Some(previous) = completion.as_ref() {
                if previous.phase != phase || previous.failure != failure {
                    return Err(Error::Pending);
                }
                previous.clone()
            } else {
                let request = ReceiptRequest {
                    instance: current,
                    netmap: *ticket.netmap(),
                    phase,
                    attempt: ticket.attempt(),
                    sequence: Sequence::MIN.checked_next().map_err(|_| Error::Lifecycle)?,
                    // This attempt's completion is observed no earlier than its
                    // saved receipt: an installation that predates a new
                    // instance is reported when that instance receipts it.
                    reported_at_ms: u64::try_from(observed_at.timestamp_millis())
                        .map_err(|_| Error::Contract)?
                        .max(ticket.0.saved.reported_at_ms),
                    failure,
                };
                *completion = Some(request.clone());
                request
            }
        };
        let response = self
            .client
            .submit_node_configuration_receipt(&request, self.token()?)
            .await;
        if let Err(error) = self.receipt_response(response) {
            if matches!(error, Error::ReceiptRejected(_)) {
                ticket.0.rejected.store(true, Ordering::Release);
            }
            return Err(error);
        }
        self.current_instance()?;
        Ok(())
    }

    /// Report runtime-owned facts with the same per-instance sequence/retry
    /// owner as `report_control_state`. No status/report grants authority.
    pub async fn report_runtime_state(
        &mut self,
        state: NodeState,
        observed_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<StateAccepted, Error> {
        let instance = self.current_instance()?;
        // A retry is the original observation, even if its target has since
        // become historical. Only Server can decide its remaining freshness.
        // New facts must wait rather than replacing an uncertain delivery.
        if let Some(pending) = &self.report {
            if pending.state != state {
                return Err(Error::Pending);
            }
            return self.retry_state_report().await;
        }
        let active = self.instance.as_ref().ok_or(Error::Lifecycle)?;
        if state.capabilities != active.capabilities {
            // Capabilities changed since the instance was acquired: acquire
            // again with them; the observation itself is not wrong.
            return Err(Error::Lifecycle);
        }
        // A fresh host observation belongs to this precise instance and the
        // held netmap, including failure before applied. Never relabel an old
        // probe; uncertain retries above keep their original body.
        let held = self.netmap_reference();
        // An observation taken before the held netmap or instance changed is
        // stale, not a contract break: the host observes again.
        if let Some(exit) = &state.exit_capability
            && (exit.instance_id != instance.instance_id || held != Some(exit.netmap))
        {
            return Err(Error::StaleReport);
        }
        if let Some(saved) = &state.configuration.saved
            && held != Some(*saved)
        {
            return Err(Error::StaleReport);
        }
        if let Some(applied) = &state.configuration.applied {
            self.held_netmap(*applied)?;
        }
        self.report_sequence = self
            .report_sequence
            .checked_add(1)
            .ok_or(Error::Lifecycle)?;
        self.report = Some(StateReport {
            instance_id: instance.instance_id,
            sequence: Sequence::new(self.report_sequence).map_err(|_| Error::Lifecycle)?,
            observed_at,
            state,
        });
        self.retry_state_report().await
    }
}
