//! Structural notifications schedule real samples, never measurements or grants.
use super::Report;
use meshway_connectivity::LinkFact;
use meshway_model::{NetmapRef, NodeId, Sequence};
use meshway_protocol::{InstanceRequest, execution::ApplyFailure};
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{sync::watch, time::Instant};

pub(super) const CHANGE_INTERVAL: Duration = Duration::from_secs(1);

#[derive(PartialEq, Eq)]
struct ExitKey {
    instance: meshway_model::InstanceId,
    netmap: NetmapRef,
    supported: Vec<meshway_model::exit::ExitFamily>,
    installed: Vec<meshway_model::exit::ExitFamily>,
    status: meshway_model::exit::ExitStatus,
}

#[derive(PartialEq, Eq)]
struct Listener {
    key: String,
    protocol: meshway_model::TransportProtocol,
    local: SocketAddr,
}

#[derive(PartialEq, Eq)]
pub(super) struct Key {
    netmap: Option<NetmapRef>,
    plane: meshway_protocol::ipc::DataPlane,
    problem: Option<meshway_protocol::ipc::Problem>,
    completion: Option<Result<(), ApplyFailure>>,
    generation: Sequence,
    interface_epoch: u64,
    dns_live: bool,
    exit: Option<ExitKey>,
    exit_dns: Option<meshway_model::exit::ExitDns>,
    listeners: BTreeMap<[u8; 32], Listener>,
    links: BTreeMap<[u8; 32], LinkFact>,
    sockets: Vec<meshway_connectivity::SocketFact>,
    dependencies: BTreeMap<NodeId, u32>,
    circuits: BTreeMap<[u8; 32], (Vec<NodeId>, meshway_model::state::ReporterRole)>,
}
impl Key {
    fn of(report: &Report) -> Self {
        let dns_live = report
            .dns_guard
            .as_ref()
            .is_none_or(|guard| guard.has_changed().is_ok() && guard.borrow().is_none());
        let mut key = Self {
            netmap: report.netmap,
            plane: report.plane,
            problem: report.problem,
            completion: report.completion,
            generation: report.generation,
            interface_epoch: report.interface_epoch,
            dns_live,
            exit: report.exit_capability.as_ref().map(|exit| ExitKey {
                instance: exit.instance_id,
                netmap: exit.netmap,
                supported: exit.supported_families.clone(),
                installed: exit.installed_families.clone(),
                status: exit.status.clone(),
            }),
            exit_dns: report.exit_dns.clone(),
            listeners: BTreeMap::new(),
            links: BTreeMap::new(),
            sockets: Vec::new(),
            dependencies: BTreeMap::new(),
            circuits: BTreeMap::new(),
        };
        if dns_live
            && report.plane == meshway_protocol::ipc::DataPlane::Ready
            && let Some(facts) = &report.facts
        {
            key.listeners = facts
                .listeners
                .iter()
                .map(|listener| {
                    (
                        listener.socket_id,
                        Listener {
                            key: listener.key.clone(),
                            protocol: listener.protocol,
                            local: listener.local,
                        },
                    )
                })
                .collect();
            key.links = facts
                .links
                .iter()
                .map(|link| (link.id, link.clone()))
                .collect();
            key.dependencies = report.dependencies.clone();
            key.circuits = report.circuits.clone();
            key.sockets = facts.sockets.clone();
        }
        key
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) struct Publisher {
    reports: watch::Sender<Report>,
    changes: watch::Sender<Arc<Key>>,
}
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl Publisher {
    pub fn borrow(&self) -> watch::Ref<'_, Report> {
        self.reports.borrow()
    }
    pub fn send_replace(&self, report: Report) {
        let key = Key::of(&report);
        let changed = **self.changes.borrow() != key;
        self.reports.send_replace(report);
        if changed {
            self.changes.send_replace(Arc::new(key));
        }
    }
    pub fn sample(&self, report: Report) -> Sample {
        self.send_replace(report.clone());
        Sample {
            report,
            key: self.changes.borrow().clone(),
        }
    }
}

pub(super) struct Sample {
    pub report: Report,
    key: Arc<Key>,
}

#[derive(Clone)]
struct Mark {
    key: Arc<Key>,
    receipt: Arc<()>,
}
impl Mark {
    fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.key, &other.key) && Arc::ptr_eq(&self.receipt, &other.receipt)
    }
}
struct Pending {
    instance: InstanceRequest,
    mark: Option<Mark>,
    #[cfg(target_os = "linux")]
    candidates: Vec<meshway_protocol::connectivity::CandidateReference>,
}

pub(super) struct Schedule {
    changes: watch::Receiver<Arc<Key>>,
    receipt: Arc<()>,
    handled: Mark,
    last_attempt: Option<Instant>,
    pending: Option<Pending>,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) fn channel(reports: watch::Sender<Report>) -> (Publisher, Schedule) {
    let key = Arc::new(Key::of(&reports.borrow()));
    let (changes, received) = watch::channel(key.clone());
    let receipt = Arc::new(());
    let handled = Mark {
        key,
        receipt: receipt.clone(),
    };
    (
        Publisher { reports, changes },
        Schedule {
            changes: received,
            receipt,
            handled,
            last_attempt: None,
            pending: None,
        },
    )
}

impl Schedule {
    fn latest(&self) -> Mark {
        Mark {
            key: self.changes.borrow().clone(),
            receipt: self.receipt.clone(),
        }
    }
    fn next_attempt(&self) -> Instant {
        self.last_attempt
            .map_or_else(Instant::now, |at| at + CHANGE_INTERVAL)
    }
    fn due(&self, routine: bool) -> bool {
        let ready = self
            .last_attempt
            .is_none_or(|at| Instant::now() >= at + CHANGE_INTERVAL);
        ready && (routine || self.pending.is_some() || !self.handled.same(&self.latest()))
    }
    pub fn defer(&mut self) {
        let now = Instant::now();
        if self
            .last_attempt
            .is_none_or(|at| at + CHANGE_INTERVAL <= now)
        {
            self.last_attempt = Some(now);
        }
    }
    pub fn receipt_completed(&mut self) {
        self.receipt = Arc::new(());
    }
    pub async fn wait(&mut self) -> Result<(), ()> {
        loop {
            let key = self.changes.borrow_and_update().clone();
            if self.changes.has_changed().is_err() {
                return Err(());
            }
            if self.pending.is_some()
                || !self.handled.same(&Mark {
                    key,
                    receipt: self.receipt.clone(),
                })
            {
                tokio::time::sleep_until(self.next_attempt()).await;
                return Ok(());
            }
            self.changes.changed().await.map_err(|_| ())?;
        }
    }
    fn acknowledge(&mut self, mark: Option<Mark>) {
        // A historical sample can only acknowledge its own event identities.
        // In particular, ABA transitions keep distinct Arc identities.
        if let Some(mark) = mark {
            self.handled = mark;
        }
    }
    fn delivered(&mut self, retried: bool) -> bool {
        let pending = self.pending.take().expect("sample kept before dispatch");
        self.acknowledge(pending.mark);
        !retried
    }
}

impl super::Handle {
    /// True only when this invocation submits its freshly obtained sample.
    /// A pending retry is owned by Control and retains its original sequence,
    /// body and observation time; its success never acknowledges a new sample.
    pub async fn report_state(
        &mut self,
        shared: &tokio::sync::Mutex<meshway_control::Control>,
        settings: &crate::settings::Settings,
        routine: bool,
    ) -> Result<bool, meshway_control::Error> {
        use meshway_control::{Error, shared as control};
        let Some(instance) =
            shared
                .lock()
                .await
                .status()
                .instance
                .map(|instance| InstanceRequest {
                    instance_id: instance.instance_id,
                    generation: instance.generation,
                })
        else {
            return Ok(false);
        };
        if self
            .reporting
            .pending
            .as_ref()
            .is_some_and(|pending| pending.instance != instance)
        {
            self.reporting.pending = None;
        }
        if !self.reporting.due(routine) {
            return Ok(false);
        }
        self.reporting.last_attempt = Some(Instant::now());
        if self.reporting.pending.is_some() {
            match control::retry_state_report(shared).await {
                Ok(accepted) => {
                    self.acknowledge_traversal(&accepted);
                    return Ok(self.reporting.delivered(true));
                }
                // A cancelled preflight may not have accepted a report yet.
                // Clearing this scheduling marker creates no acknowledgement.
                Err(Error::Lifecycle) => {
                    self.reporting.pending = None;
                    return Ok(false);
                }
                Err(error) => {
                    if matches!(error, Error::StateScopeChanged | Error::StaleReport) {
                        self.reporting.pending = None;
                    }
                    return Err(error);
                }
            }
        }
        let sample = match self.sample().await {
            Ok(sample) => Some(sample),
            Err(super::SampleError::Timeout) => {
                // Installation can keep the worker busy beyond the sampling
                // budget. No observation is not an observation of absence:
                // leave the complete State, its timestamps and grants alone.
                // Do not acknowledge this change; a later attempt samples it.
                tracing::debug!("Runtime State sample timed out; skipping complete report");
                return Ok(false);
            }
            // A closed worker is stopped, rather than temporarily unobserved.
            Err(super::SampleError::Closed) => None,
        };
        let receipt = self.reporting.receipt.clone();
        let facts = sample
            .as_ref()
            .map_or_else(Report::stopped, |sample| sample.report.clone());
        let mut state = facts.state(&*shared.lock().await, settings);
        if let Some(netmap) = &facts.content {
            crate::runtime::local_endpoints::sample(
                &mut state,
                &netmap.netmap,
                &facts.owned_interfaces,
            );
        }
        // Guard changes between sampling and projection must not acknowledge
        // the earlier positive observation with a now-empty outgoing report.
        let mark = sample
            .filter(|sample| *sample.key == Key::of(&facts))
            .map(|sample| Mark {
                key: sample.key,
                receipt,
            });
        #[cfg(target_os = "linux")]
        let candidates = shared
            .lock()
            .await
            .status()
            .binding
            .map(|binding| super::traversal::candidates(&state, binding.node_id, &instance))
            .unwrap_or_default();
        self.reporting.pending = Some(Pending {
            instance,
            mark,
            #[cfg(target_os = "linux")]
            candidates,
        });
        match control::report_runtime_state(shared, state, chrono::Utc::now()).await {
            Ok(accepted) => {
                self.acknowledge_traversal(&accepted);
                Ok(self.reporting.delivered(false))
            }
            Err(Error::Pending) => {
                // Defensive recovery if Control already kept an older body.
                // Its marker is unknown, so retry it without acknowledging ours.
                self.reporting
                    .pending
                    .as_mut()
                    .expect("pending report")
                    .mark = None;
                #[cfg(target_os = "linux")]
                self.reporting
                    .pending
                    .as_mut()
                    .expect("pending report")
                    .candidates
                    .clear();
                match control::retry_state_report(shared).await {
                    Ok(accepted) => {
                        self.acknowledge_traversal(&accepted);
                        Ok(self.reporting.delivered(true))
                    }
                    Err(error) => {
                        if matches!(error, Error::StateScopeChanged | Error::StaleReport) {
                            self.reporting.pending = None;
                        }
                        Err(error)
                    }
                }
            }
            Err(error) => {
                if matches!(error, Error::StateScopeChanged | Error::StaleReport) {
                    self.reporting.pending = None;
                }
                Err(error)
            }
        }
    }

    fn acknowledge_traversal(&self, accepted: &meshway_protocol::StateAccepted) {
        #[cfg(target_os = "linux")]
        if let Some(pending) = &self.reporting.pending {
            tracing::debug!(operation = "report_runtime_state", source_instance = %pending.instance.instance_id,
                sequence = accepted.sequence.get(), duplicate = accepted.duplicate,
                netmap_serial = ?pending.mark.as_ref().and_then(|mark| mark.key.netmap.map(|reference| reference.serial.get())),
                candidate_count = pending.candidates.len(), "State candidate publication acknowledged");
            self.traversal_acknowledged
                .send_replace(super::traversal::Acknowledged {
                    sequence: Some(accepted.sequence),
                    candidates: pending.candidates.clone(),
                });
        }
        #[cfg(not(target_os = "linux"))]
        let _ = accepted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_times_do_not_notify_but_real_resource_changes_do() {
        let (reports, _) = watch::channel(Report::stopped());
        let (publisher, mut schedule) = channel(reports);
        let first = schedule.changes.borrow_and_update().clone();
        let mut report = Report::stopped();
        report.observed_at += chrono::Duration::seconds(1);
        report.facts_observed_at += chrono::Duration::seconds(2);
        publisher.send_replace(report.clone());
        assert!(!schedule.changes.has_changed().unwrap());
        assert!(Arc::ptr_eq(&first, &schedule.latest().key));
        report.plane = meshway_protocol::ipc::DataPlane::Ready;
        report.facts = Some(meshway_connectivity::Snapshot {
            mtu: Default::default(),
            listeners: vec![meshway_connectivity::ListenerFact {
                socket_id: [1; 32],
                key: "native".into(),
                protocol: meshway_model::TransportProtocol::NativeUdp,
                local: "127.0.0.1:35000".parse().unwrap(),
                buffers: None,
            }],
            links: vec![],
            sockets: vec![],
            adjacencies: BTreeMap::new(),
        });
        publisher.send_replace(report.clone());
        let physical = schedule.changes.borrow_and_update().clone();
        report.facts_observed_at += chrono::Duration::seconds(1);
        publisher.send_replace(report.clone());
        assert!(!schedule.changes.has_changed().unwrap());
        report.facts.as_mut().unwrap().listeners[0].socket_id = [2; 32];
        publisher.send_replace(report);
        assert!(!Arc::ptr_eq(&physical, &schedule.latest().key));
    }

    #[test]
    fn late_sample_cannot_clear_a_new_ready_transition_even_after_aba() {
        let (reports, _) = watch::channel(Report::stopped());
        let (publisher, mut schedule) = channel(reports);
        let sample = publisher.sample(Report::stopped());
        let old = Mark {
            key: sample.key,
            receipt: schedule.receipt.clone(),
        };
        let mut ready = Report::stopped();
        ready.plane = meshway_protocol::ipc::DataPlane::Ready;
        publisher.send_replace(ready);
        publisher.send_replace(Report::stopped());
        schedule.acknowledge(Some(old));
        assert!(schedule.due(false));
    }

    #[test]
    fn retrying_old_sample_does_not_acknowledge_new_facts_or_receipt() {
        let (reports, _) = watch::channel(Report::stopped());
        let (publisher, mut schedule) = channel(reports);
        let old = schedule.latest();
        schedule.pending = Some(Pending {
            instance: InstanceRequest {
                instance_id: "00000000-0000-4000-8000-000000000001".parse().unwrap(),
                generation: meshway_model::Version::MIN,
            },
            mark: Some(old),
            #[cfg(target_os = "linux")]
            candidates: Vec::new(),
        });
        let mut ready = Report::stopped();
        ready.plane = meshway_protocol::ipc::DataPlane::Ready;
        publisher.send_replace(ready);
        let ready_sample = schedule.latest();
        schedule.receipt_completed();
        assert!(!schedule.delivered(true));
        assert!(schedule.due(false));
        schedule.acknowledge(Some(ready_sample));
        assert!(schedule.due(false));
        schedule.acknowledge(None);
        assert!(schedule.due(false));
        schedule.last_attempt = Some(Instant::now());
        assert!(!schedule.due(false));
    }
}
