//! Local assembly only: Control verifies and persists the signed netmap,
//! Engine owns its lease and the single reconfiguration, Host owns kernel
//! state, Manager owns adjacency, and SecureNode owns IP flows and circuit
//! crypto. Packets, ping and DNS are decided locally on the desired netmap.
#[cfg(any(target_os = "linux", test))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod diagnostics;
#[cfg(target_os = "linux")]
mod dns;
#[cfg(target_os = "linux")]
mod exit;
mod facts;
#[cfg(target_os = "linux")]
mod packets;
#[cfg(target_os = "linux")]
mod plane;
mod reporting;
mod requirements;
#[cfg(any(target_os = "linux", test))]
mod transitions;
#[cfg(target_os = "linux")]
mod traversal;
#[cfg(target_os = "linux")]
mod worker;

use chrono::{DateTime, Utc};
use meshway_engine::DataPlaneKeys;
use meshway_model::NetmapRef;
use meshway_private_storage::Directory;
use meshway_protocol::execution::ApplyFailure;
use meshway_protocol::ipc::{DataPlane, Problem};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, watch};

#[cfg(any(target_os = "linux", test))]
fn startup_problem(error: &meshway_underlay::Error) -> Problem {
    use meshway_underlay::Error;
    match error {
        Error::Storage(meshway_private_storage::Error::InUse) => Problem::Underlay,
        Error::Journal | Error::JournalVersion { .. } | Error::Storage(_) => Problem::Storage,
        Error::Unsupported => Problem::UnsupportedMode,
        Error::Options | Error::Origin | Error::Resolver => Problem::Configuration,
        _ => Problem::Underlay,
    }
}

/// The detail of a Mesh pool overlap, led by its stable reason.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) fn pool_overlap_detail(overlap: &meshway_underlay::PoolOverlap) -> String {
    format!(
        "meshPoolOverlap: {} address {} is inside a Mesh address pool",
        overlap.interface, overlap.address
    )
}

pub(super) fn capabilities() -> std::collections::BTreeMap<String, meshway_model::state::Capability>
{
    let mut capabilities = meshway_connectivity::capabilities();
    if cfg!(target_os = "linux") {
        capabilities.insert(
            meshway_model::exit::EXIT_IPV4_FEATURE.into(),
            meshway_model::state::Capability::Feature {
                version: meshway_model::exit::EXIT_IPV4_VERSION,
            },
        );
        capabilities.insert(
            "feature:path-observation".into(),
            meshway_model::state::Capability::Feature { version: 1 },
        );
    }
    capabilities
}

/// The worker's actual installation state. `netmap`/`completion` describe the
/// installation of exactly that content; they are independent of instances.
#[derive(Clone)]
pub(super) struct Report {
    pub plane: DataPlane,
    pub problem: Option<Problem>,
    /// The concrete cause of `problem` when one is known (a listener's bind
    /// error); `None` otherwise.
    pub problem_detail: Option<String>,
    /// Engine's installed desired content; `None` once withdrawn.
    pub content: Option<Arc<meshway_configuration::netmap::CompiledNetmap>>,
    /// Lease end of `content` as accepted by Engine; reporting only.
    pub expires_at: Option<DateTime<Utc>>,
    /// Egress capture Engine computed for `content`; reporting only.
    pub capture: Option<meshway_configuration::evaluation::CaptureSummary>,
    pub dns_guard: Option<watch::Receiver<Option<meshway_dns::SystemDnsError>>>,
    /// The check the DNS owner's current fault failed, live beside `dns_guard`.
    pub dns_detail: Option<watch::Receiver<Option<meshway_dns::SystemDnsFailure>>>,
    /// Netmap whose installation this report describes.
    pub netmap: Option<NetmapRef>,
    pub completion: Option<Result<(), ApplyFailure>>,
    /// A failed plane has no automatic way back: kernel ownership ended in a
    /// proven conflict, or nothing retries the failure (validation, retries
    /// spent). Status shows `blocked` for it, else `retrying`.
    pub persistent: bool,
    pub observed_at: DateTime<Utc>,
    pub facts_observed_at: DateTime<Utc>,
    pub facts: Option<meshway_connectivity::Snapshot>,
    pub exit_capability: Option<meshway_model::exit::ExitCapability>,
    /// The stopped exit role's cleanup its owner retries, live (read at
    /// each Status). Set once at Host start, like `kernel_changes`.
    pub exit_cleanups: Option<watch::Receiver<Option<meshway_underlay::RetryPending>>>,
    /// The Host's live retried kernel change (egress capture or exit role);
    /// read at each Status, also while an apply waits for that change. Set
    /// once at Host start; every later report keeps it.
    pub kernel_changes: Option<watch::Receiver<Option<meshway_underlay::KernelChangePending>>>,
    /// The Control relay's listener state, live; set once at Host start
    /// like `kernel_changes`.
    pub control_relay: Option<watch::Receiver<Option<meshway_underlay::ControlRelayState>>>,
    pub exit_flows: Option<u32>,
    /// The DNS owner's exit resolver fact of the installed content.
    pub exit_dns: Option<meshway_model::exit::ExitDns>,
    pub generation: meshway_model::Sequence,
    /// Every classified external network change, and the path changes
    /// among them (`meshway_underlay::NetworkChange`).
    pub interface_epoch: u64,
    pub network_path: u64,
    pub last_network_change: Option<DateTime<Utc>>,
    pub owned_interfaces: Vec<String>,
    pub dependencies: std::collections::BTreeMap<meshway_model::NodeId, u32>,
    pub observation_dependencies:
        std::collections::BTreeMap<meshway_model::NodeId, Vec<meshway_model::ObservationId>>,
    /// Established business circuits this Node takes part in, in
    /// origin-to-terminal order with its role; at most `STATE_MAX_PATHS`.
    pub circuits: std::collections::BTreeMap<
        [u8; 32],
        (
            Vec<meshway_model::NodeId>,
            meshway_model::state::ReporterRole,
        ),
    >,
    /// Complete path of an established circuit this Node ends, by far end.
    pub routes: std::collections::BTreeMap<meshway_model::NodeId, Vec<meshway_model::NodeId>>,
    /// Relayed path a new circuit would take toward a needed peer that has
    /// neither a channel nor a circuit (`SecureNode::relay_route`), by peer.
    pub relays: std::collections::BTreeMap<meshway_model::NodeId, Vec<meshway_model::NodeId>>,
    /// Complete path a new circuit would take toward each visible peer that
    /// has neither a channel nor a circuit (`SecureNode::idle_route`; `None`
    /// when the content gives no path), by peer.
    pub idle_paths:
        std::collections::BTreeMap<meshway_model::NodeId, Option<Vec<meshway_model::NodeId>>>,
    /// Latest successful measured round trip toward each target.
    pub rtts: std::collections::BTreeMap<meshway_model::NodeId, meshway_protocol::ipc::PeerRtt>,
    /// Control's last known peer facts, as the worker reads them.
    pub known_peers: Option<watch::Receiver<Option<meshway_control::KnownPeers>>>,
}
impl Report {
    pub fn stopped() -> Self {
        Self {
            plane: DataPlane::Stopped,
            problem: None,
            problem_detail: None,
            content: None,
            expires_at: None,
            capture: None,
            dns_guard: None,
            dns_detail: None,
            netmap: None,
            completion: None,
            persistent: false,
            observed_at: Utc::now(),
            facts_observed_at: Utc::now(),
            facts: None,
            exit_capability: None,
            exit_cleanups: None,
            kernel_changes: None,
            control_relay: None,
            exit_flows: None,
            exit_dns: None,
            generation: meshway_model::Sequence::MIN,
            interface_epoch: 0,
            network_path: 0,
            last_network_change: None,
            owned_interfaces: Vec::new(),
            dependencies: Default::default(),
            observation_dependencies: Default::default(),
            circuits: Default::default(),
            routes: Default::default(),
            relays: Default::default(),
            idle_paths: Default::default(),
            rtts: Default::default(),
            known_peers: None,
        }
    }
    /// The installation is live: Engine holds its content and DNS is healthy.
    pub fn current(&self) -> bool {
        self.dns_guard
            .as_ref()
            .is_none_or(|guard| guard.has_changed().is_ok() && guard.borrow().is_none())
            && self.content.is_some()
    }
    /// The DNS owner's fault while content is installed, with the check it
    /// failed: the plane withdraws for it (or does already), so it is the
    /// cause Status shows before the worker's own report.
    pub fn dns_fault(&self) -> Option<Option<String>> {
        let guard = self.dns_guard.as_ref()?;
        if self.content.is_none() || (guard.has_changed().is_ok() && guard.borrow().is_none()) {
            return None;
        }
        Some(
            self.dns_detail
                .as_ref()
                .and_then(|detail| detail.borrow().as_ref().map(ToString::to_string)),
        )
    }
    /// The failed plane recovers by itself: its failure is transient, or a
    /// kernel change or exit cleanup is being retried right now.
    pub fn recovering(&self) -> bool {
        !self.persistent
            || self
                .kernel_changes
                .as_ref()
                .is_some_and(|changes| changes.borrow().is_some())
            || self
                .exit_cleanups
                .as_ref()
                .is_some_and(|cleanups| cleanups.borrow().is_some())
    }
    /// The netmap whose every required resource is installed.
    pub fn applied(&self) -> Option<NetmapRef> {
        match self.completion {
            Some(Ok(())) => self.netmap,
            _ => None,
        }
    }
}

pub(super) enum Request {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    ObservationResult {
        output: Box<meshway_dataplane::diagnostics::Output>,
        reply: oneshot::Sender<Result<(), ()>>,
    },
    #[cfg(target_os = "linux")]
    Traversal(traversal::Request),
    /// Complete one trusted DNS association task (exit resolver).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    DnsAssociation(Box<meshway_protocol::DnsAssociationReport>),
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Consumed by the Linux-only worker.
enum Command {
    Ping(crate::runtime::PingSession),
    Netcheck(oneshot::Sender<Option<meshway_connectivity::StunProbe>>),
    Builder(oneshot::Sender<Result<reqwest::ClientBuilder, Problem>>),
    Sample(oneshot::Sender<reporting::Sample>),
    Stop {
        down: bool,
        reply: oneshot::Sender<Result<(), Problem>>,
    },
    Shutdown {
        keep_sink: bool,
    },
}

#[derive(Debug)]
enum SampleError {
    Timeout,
    Closed,
}

/// Server-assigned observation tasks of the installed netmap's source Node.
#[derive(Clone)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct ObservationPlan {
    netmap: NetmapRef,
    tasks: Vec<meshway_protocol::observations::ObservationTask>,
}

/// What Control offers the Engine: the verified netmap, the keys its packet
/// plane uses and the instance binding of State/receipts (not content).
#[derive(Clone, Default)]
pub(super) struct Offer {
    pub netmap: Option<meshway_engine::Netmap>,
    pub keys: Option<DataPlaneKeys>,
    pub instance: Option<meshway_protocol::InstanceRequest>,
    /// Control's UTC high-water mark; Engine's lease clock never goes below it.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub clock_floor_ms: Option<u64>,
}

type OfferKey = (
    NetmapRef,
    meshway_model::NetmapLease,
    Vec<meshway_rules::Digest>,
    Option<Arc<meshway_configuration::netmap::DnsAssociationOverlay>>,
);

/// What identifies an offered netmap: content, lease, the loaded rule-set
/// artifacts (more artifacts for the same content is a new offer) and the
/// DNS association overlay (a changed or cleared overlay is a new offer).
fn offered(netmap: &Option<meshway_engine::Netmap>) -> Option<OfferKey> {
    netmap.as_ref().map(|netmap| {
        (
            netmap.content.reference(),
            netmap.lease,
            netmap.artifacts.keys().copied().collect(),
            netmap.associations.clone(),
        )
    })
}

/// Sync's direct path to the Engine worker: only the verified netmap and its
/// lease change; keys and instance stay Control's offer.
pub(super) struct LeaseSink(Arc<watch::Sender<Offer>>);
impl LeaseSink {
    pub fn offer_netmap(&self, netmap: Option<meshway_engine::Netmap>) {
        self.0.send_if_modified(|current| {
            if offered(&current.netmap) == offered(&netmap) {
                return false;
            }
            current.netmap = netmap;
            true
        });
    }
}

pub(super) struct Handle {
    observation_plans: watch::Sender<Option<ObservationPlan>>,
    commands: mpsc::Sender<Command>,
    offers: Arc<watch::Sender<Offer>>,
    pub report: watch::Receiver<Report>,
    pub requests: mpsc::Receiver<Request>,
    worker: tokio::task::JoinHandle<Result<(), Problem>>,
    reporting: reporting::Schedule,
    /// Control's last known peer facts, published by Control alone.
    peers: watch::Sender<Option<meshway_control::KnownPeers>>,
    #[cfg(target_os = "linux")]
    traversal_acknowledged: watch::Sender<traversal::Acknowledged>,
    /// The Underlay's classified network changes; its relay closes Control
    /// connections on each path change.
    #[cfg(target_os = "linux")]
    network_changes: watch::Receiver<meshway_underlay::NetworkChange>,
}

impl Handle {
    pub async fn open(
        directory: Arc<Directory>,
        origin: &str,
        reports: watch::Sender<Report>,
        options: meshway_underlay::Options,
        dns_settings: meshway_dns::SystemDnsSettings,
    ) -> Result<Self, Problem> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (directory, origin, reports, options, dns_settings);
            Err(Problem::UnsupportedMode)
        }
        #[cfg(target_os = "linux")]
        {
            let mut options = options;
            let mtu = options.mtu;
            // The owned file may still point at our stopped stub after a crash.
            // Recover its original resolver inputs before protected Host bootstrap.
            let reservation = if matches!(
                dns_settings.backend,
                meshway_dns::SystemDnsBackend::OwnedResolvConf { .. }
            ) {
                let reserved =
                    meshway_dns::SystemDnsOwner::reserve(dns_settings.clone(), directory.clone())
                        .await
                        .map_err(|_| Problem::Dns)?;
                if options.bootstrap_nameservers.is_none() && options.bootstrap_hosts.is_empty() {
                    options.bootstrap_nameservers = reserved
                        .bootstrap_nameservers()
                        .await
                        .map_err(|_| Problem::Dns)?;
                }
                Some(reserved)
            } else {
                None
            };
            let mut host = meshway_underlay::Host::open(directory.clone(), options)
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "Underlay startup failed");
                    startup_problem(&error)
                })?;
            if let Err(error) = host.attach_control(origin).await {
                tracing::warn!(%error, "Underlay Control attachment failed");
                if let Err(error) = host.shutdown(true).await {
                    tracing::warn!(%error, "Underlay startup cleanup failed");
                }
                return Err(startup_problem(&error));
            }
            let network_changes = host.network_changes();
            let kernel_changes = host.kernel_changes();
            let exit_cleanups = host.exit_cleanups();
            let control_relay = host.control_relay();
            reports.send_modify(|report| {
                report.kernel_changes = Some(kernel_changes);
                report.exit_cleanups = Some(exit_cleanups);
                report.control_relay = Some(control_relay);
            });
            let (commands, received) = mpsc::channel(8);
            let (observation_plans, plans_rx) = watch::channel(None);
            let (offer_sender, offers) = watch::channel(Offer::default());
            let report = reports.subscribe();
            let (reports, reporting) = reporting::channel(reports);
            let (gateway, requests) = mpsc::channel(16);
            let (traversal_acknowledged, acknowledged) =
                watch::channel(traversal::Acknowledged::default());
            let traversal = traversal::Gateway::new(gateway.clone(), acknowledged);
            let dns = dns::Attachment::new(dns_settings, directory, reservation);
            let (peers, worker_peers) = watch::channel(None);
            // The Host live-probe future is !Send (its monitor registration
            // uses a synchronous guard). One joined owner thread polls it in
            // place; the Host and all its existing async producers stay unique.
            let worker = tokio::task::spawn_blocking(move || {
                tokio::runtime::Handle::current().block_on(worker::run(worker::Inputs {
                    host,
                    commands: received,
                    offers,
                    reports,
                    gateway,
                    dns,
                    traversal,
                    observation_plans: plans_rx,
                    peers: worker_peers,
                    mtu,
                }))
            });
            Ok(Self {
                observation_plans,
                commands,
                offers: Arc::new(offer_sender),
                report,
                requests,
                worker,
                reporting,
                peers,
                traversal_acknowledged,
                network_changes,
            })
        }
    }

    /// How many Underlay path changes this host has seen: each closed the
    /// Control connections it carried.
    pub fn path_changes(&self) -> u64 {
        #[cfg(target_os = "linux")]
        {
            self.network_changes.borrow().path
        }
        #[cfg(not(target_os = "linux"))]
        {
            0
        }
    }

    /// A counter of [`Self::path_changes`] that outlives this borrow.
    pub fn path_counter(&self) -> impl Fn() -> u64 + Send + Sync + 'static {
        #[cfg(target_os = "linux")]
        {
            let changes = self.network_changes.clone();
            move || changes.borrow().path
        }
        #[cfg(not(target_os = "linux"))]
        {
            || 0
        }
    }

    pub async fn client_builder(&self) -> Result<reqwest::ClientBuilder, Problem> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Builder(reply))
            .await
            .map_err(|_| Problem::Worker)?;
        result.await.map_err(|_| Problem::Worker)?
    }

    /// Where Control publishes its last known peer facts.
    pub fn peer_sink(&self) -> watch::Sender<Option<meshway_control::KnownPeers>> {
        self.peers.clone()
    }

    /// Delivery of sync's verified leases straight to the Engine worker.
    pub fn lease_sink(&self) -> LeaseSink {
        LeaseSink(self.offers.clone())
    }

    /// Offer Control's current netmap, keys and instance. An unchanged
    /// offer is not resent; Engine alone decides renewal or reconfiguration.
    pub fn offer(&self, offer: Offer) {
        self.offers.send_if_modified(|current| {
            let unchanged = offered(&current.netmap) == offered(&offer.netmap)
                && current.keys.is_some() == offer.keys.is_some()
                && current.instance == offer.instance;
            if unchanged {
                return false;
            }
            *current = offer;
            true
        });
    }

    /// A receipt was delivered; the next State reflects it.
    pub fn receipt_completed(&mut self) {
        self.reporting.receipt_completed();
    }

    /// Hold the change schedule for one interval.
    pub fn defer_report(&mut self) {
        self.reporting.defer();
    }

    /// Server-assigned tasks for the installed netmap, replacing earlier ones.
    pub fn observation_tasks(
        &self,
        netmap: NetmapRef,
        tasks: Vec<meshway_protocol::observations::ObservationTask>,
    ) {
        self.observation_plans.send_if_modified(|current| {
            let same = current.as_ref().is_some_and(|plan| {
                plan.netmap == netmap
                    && plan.tasks.len() == tasks.len()
                    && plan.tasks.iter().zip(&tasks).all(|(a, b)| {
                        a.id == b.id
                            && a.phase == b.phase
                            && a.path == b.path
                            && a.path_segment == b.path_segment
                            && a.expires_at == b.expires_at
                            && a.interval_ms == b.interval_ms
                    })
            });
            if same {
                return false;
            }
            *current = Some(ObservationPlan { netmap, tasks });
            true
        });
    }

    /// Local IPC diagnostics (ping, netcheck), decided on the installed netmap.
    pub fn local(&self) -> Local {
        Local(self.commands.clone())
    }

    pub async fn stop(&mut self, down: bool) -> Result<(), Problem> {
        self.observation_plans.send_replace(None);
        self.offers.send_replace(Offer::default());
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Stop { down, reply })
            .await
            .map_err(|_| Problem::Worker)?;
        result.await.map_err(|_| Problem::Worker)?
    }

    pub async fn shutdown(self, keep_sink: bool) -> Result<(), Problem> {
        self.offers.send_replace(Offer::default());
        let _ = self.commands.send(Command::Shutdown { keep_sink }).await;
        self.worker.await.map_err(|_| Problem::Worker)?
    }

    async fn sample(&self) -> Result<reporting::Sample, SampleError> {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            let (reply, sample) = oneshot::channel();
            self.commands
                .send(Command::Sample(reply))
                .await
                .map_err(|_| SampleError::Closed)?;
            sample.await.map_err(|_| SampleError::Closed)
        })
        .await
        .map_err(|_| SampleError::Timeout)?
    }

    pub async fn work(&mut self) -> Work {
        tokio::select! { biased;
            result = self.reporting.wait() => match result { Ok(()) => Work::Report, Err(()) => Work::Closed },
            request = self.requests.recv() => match request { Some(request) => Work::Request(request), None => Work::Closed },
        }
    }
}

/// Local IPC connects directly to the packet owner, independently of HTTP
/// maintenance and long-poll authentication. Admission stays with SecureNode.
#[derive(Clone)]
pub(crate) struct Local(mpsc::Sender<Command>);
impl Local {
    /// Ask every traversal socket of the installed Manager to observe now;
    /// `None` without an installed packet plane or when the worker is busy.
    pub async fn netcheck(&self) -> Option<meshway_connectivity::StunProbe> {
        let (reply, probe) = oneshot::channel();
        self.0.try_send(Command::Netcheck(reply)).ok()?;
        tokio::time::timeout(std::time::Duration::from_secs(1), probe)
            .await
            .ok()?
            .ok()?
    }

    pub fn send(&self, session: crate::runtime::PingSession) {
        if let Err(
            mpsc::error::TrySendError::Full(Command::Ping(session))
            | mpsc::error::TrySendError::Closed(Command::Ping(session)),
        ) = self.0.try_send(Command::Ping(session))
        {
            let _ = session
                .request
                .events
                .try_send(meshway_protocol::ipc::PingEvent::Failed {
                    code: meshway_protocol::ipc::PingError::Busy,
                });
            session.request.cancellation.cancel();
        }
    }
}

pub(super) enum Work {
    Report,
    Request(Request),
    Closed,
}

/// Run one queued request on the Control owner.
pub(super) async fn request(
    control: &tokio::sync::Mutex<meshway_control::Control>,
    request: Request,
) -> Result<(), meshway_control::Error> {
    match request {
        Request::ObservationResult { output, mut reply } => {
            if reply.is_closed() || !output.is_current() {
                return Ok(());
            }
            let result = tokio::select! { biased;
                _ = reply.closed() => return Ok(()),
                result = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    meshway_control::shared::report_observation_result(
                        control,
                        &output.task,
                        &output.result,
                    ),
                ) => result,
            };
            match result {
                Ok(Ok(_)) => {
                    let _ = reply.send(Ok(()));
                    Ok(())
                }
                Ok(Err(error)) => {
                    tracing::warn!(operation = "report_observation_result", observation = %output.task.id,
                        %error, "Observation result rejected");
                    let _ = reply.send(Err(()));
                    Err(error)
                }
                Err(_) => {
                    let _ = reply.send(Err(()));
                    Ok(())
                }
            }
        }
        #[cfg(target_os = "linux")]
        Request::Traversal(request) => traversal::dispatch(control, request).await,
        Request::DnsAssociation(report) => {
            // Not retried: the source's next query of the name completes again.
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                meshway_control::shared::report_dns_association(control, &report),
            )
            .await
            {
                Ok(Ok(accepted)) => tracing::debug!(
                    name = %report.name,
                    expires_at_ms = accepted.expires_at_ms,
                    "DNS association accepted"
                ),
                Ok(Err(error)) => {
                    tracing::warn!(operation = "report_dns_association", %error, "DNS association rejected")
                }
                Err(_) => tracing::warn!(
                    operation = "report_dns_association",
                    "DNS association report timed out"
                ),
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn corrupt_underlay_journal_is_storage_but_live_conflict_is_retryable_underlay() {
        use super::startup_problem;
        use meshway_protocol::ipc::Problem;
        assert_eq!(
            startup_problem(&meshway_underlay::Error::Journal),
            Problem::Storage
        );
        assert_eq!(
            startup_problem(&meshway_underlay::Error::JournalVersion {
                file: "underlay-egress.json",
                version: 1
            }),
            Problem::Storage
        );
        assert_eq!(
            startup_problem(&meshway_underlay::Error::Conflict),
            Problem::Underlay
        );
        assert_eq!(
            startup_problem(&meshway_underlay::Error::Storage(
                meshway_private_storage::Error::InUse
            )),
            Problem::Underlay
        );
    }
}
