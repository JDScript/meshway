//! Local assembly only: Control verifies and persists the signed netmap,
//! Engine owns its lease and the single reconfiguration, Host owns kernel
//! state, Manager owns adjacency, and SecureNode owns IP flows and circuit
//! crypto. Packets, ping and DNS are decided locally on the desired netmap.
mod diagnostics;
mod dns;
pub(crate) use dns::DnsFact;
mod facts;
mod packets;
mod plane;
mod reporting;
mod requirements;
mod transitions;
mod traversal;
mod worker;

use chrono::{DateTime, Utc};
use meshway_engine::DataPlaneKeys;
use meshway_model::NetmapRef;
use meshway_private_storage::Directory;
use meshway_protocol::execution::ApplyFailure;
use meshway_protocol::ipc::{DataPlane, Problem};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, watch};

/// Why the Host did not open, judged as at run time (G9): a foreign kernel
/// object or an unrecoverable journal is proven (`blocked`, read again every
/// 60 s); a failed read, a held lock and a bootstrap resolver that did not
/// open are retried (as `relay_unavailable` is in a running term); invalid
/// options and an unsupported platform are refusals.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn startup_failure(error: &meshway_underlay::Error) -> crate::host::OpenError {
    use crate::host::OpenError;
    use meshway_underlay::{Error, Judgement};
    match error {
        Error::Storage(meshway_private_storage::Error::InUse) => {
            OpenError::unknown(Problem::Underlay)
        }
        Error::Journal | Error::JournalVersion { .. } => OpenError::proven(Problem::Storage),
        Error::Storage(_) => OpenError::unknown(Problem::Storage),
        Error::Unsupported => Problem::UnsupportedMode.into(),
        Error::Options | Error::Origin => Problem::Configuration.into(),
        Error::Resolver => OpenError::unknown(Problem::Underlay),
        error => match error.judgement() {
            Judgement::Proven => OpenError::proven(Problem::Underlay),
            Judgement::Unknown | Judgement::Refused => OpenError::unknown(Problem::Underlay),
        },
    }
}

/// The detail of a Mesh pool overlap, led by its stable reason.
pub(super) fn pool_overlap_detail(overlap: &meshway_underlay::PoolOverlap) -> String {
    format!(
        "meshPoolOverlap: {} address {} is inside a Mesh address pool",
        overlap.interface, overlap.address
    )
}

pub(super) fn capabilities(
    host: &crate::host::Capabilities,
) -> std::collections::BTreeMap<String, meshway_model::state::Capability> {
    let mut capabilities = meshway_connectivity::capabilities();
    for family in &host.exit_families {
        let (feature, version) = family.feature();
        capabilities.insert(
            feature.into(),
            meshway_model::state::Capability::Feature { version },
        );
    }
    if host.path_observation {
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
    /// The DNS owner's state, live: its fault, the check it failed and its
    /// recovery.
    pub dns_guard: Option<watch::Receiver<DnsFact>>,
    /// Netmap whose installation this report describes.
    pub netmap: Option<NetmapRef>,
    pub completion: Option<Result<(), ApplyFailure>>,
    /// The worker's own recovery of a failed installation of this netmap
    /// (retrying a transient failure; blocked on a refusal until another
    /// offer). Status reads it with every other owner's health.
    pub apply: meshway_model::recovery::Health,
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
    /// The installed Manager's listeners whose bind is being retried,
    /// live: each retry owner's attempts and health (read at each Status).
    pub listeners: Option<watch::Receiver<Vec<meshway_connectivity::ListenerUnavailable>>>,
    /// The Control relay's listener state, live; set once at Host start
    /// like `kernel_changes`.
    pub control_relay: Option<watch::Receiver<Option<meshway_underlay::ControlRelayState>>>,
    /// Kernel ownership's health, live across terms; set once at Host start
    /// like `kernel_changes`.
    pub ownership: Option<watch::Receiver<meshway_model::recovery::Health>>,
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
            netmap: None,
            completion: None,
            apply: Default::default(),
            observed_at: Utc::now(),
            facts_observed_at: Utc::now(),
            facts: None,
            exit_capability: None,
            exit_cleanups: None,
            kernel_changes: None,
            listeners: None,
            control_relay: None,
            ownership: None,
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
            .is_none_or(|guard| guard.has_changed().is_ok() && guard.borrow().fault.is_none())
            && self.content.is_some()
    }
    /// The DNS owner's fault while content is installed, with the check it
    /// failed: the plane withdraws for it (or does already), so it is the
    /// cause Status shows before the worker's own report.
    pub fn dns_fault(&self) -> Option<Option<String>> {
        let guard = self.dns_guard.as_ref()?;
        if self.content.is_none() || (guard.has_changed().is_ok() && guard.borrow().fault.is_none())
        {
            return None;
        }
        Some(guard.borrow().detail.as_ref().map(ToString::to_string))
    }
    /// The listeners whose bind is being retried: the live retry owners'
    /// while their Manager runs, otherwise the last sample's.
    pub fn unavailable_listeners(&self) -> Vec<meshway_connectivity::ListenerUnavailable> {
        match self
            .listeners
            .as_ref()
            .filter(|live| live.has_changed().is_ok())
        {
            Some(live) => live.borrow().clone(),
            None => self
                .facts
                .as_ref()
                .map(|facts| facts.unavailable.clone())
                .unwrap_or_default(),
        }
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
    ObservationResult {
        output: Box<meshway_dataplane::diagnostics::Output>,
        reply: oneshot::Sender<Result<(), ()>>,
    },
    Traversal(traversal::Request),
    /// Complete one trusted DNS association task (exit resolver).
    DnsAssociation(Box<meshway_protocol::DnsAssociationReport>),
}

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
    pub request_interrupt: Option<Arc<dyn meshway_control_client::RequestInterrupt>>,
    pub capabilities: crate::host::Capabilities,
    platform: Arc<dyn crate::host::Platform>,
    observation_plans: watch::Sender<Option<ObservationPlan>>,
    commands: mpsc::Sender<Command>,
    offers: Arc<watch::Sender<Offer>>,
    pub report: watch::Receiver<Report>,
    pub requests: mpsc::Receiver<Request>,
    worker: tokio::task::JoinHandle<Result<(), Problem>>,
    reporting: reporting::Schedule,
    /// Control's last known peer facts, published by Control alone.
    peers: watch::Sender<Option<meshway_control::KnownPeers>>,
    traversal_acknowledged: watch::Sender<traversal::Acknowledged>,
    /// The Underlay's classified network changes; its relay closes Control
    /// connections on each path change.
    network_changes: watch::Receiver<meshway_underlay::NetworkChange>,
}

impl Handle {
    pub async fn open(
        directory: Arc<Directory>,
        origin: &str,
        reports: watch::Sender<Report>,
        options: meshway_underlay::Options,
        dns_settings: meshway_dns::SystemDnsSettings,
        platform: Arc<dyn crate::host::Platform>,
    ) -> Result<Self, crate::host::OpenError> {
        {
            let mtu = options.mtu;
            let host = platform
                .open(directory, origin, options, dns_settings.clone())
                .await?;
            let network_changes = host.network_changes();
            let request_interrupt = host.request_interrupt();
            let kernel_changes = host.kernel_changes();
            let exit_cleanups = host.exit_cleanups();
            let control_relay = host.control_relay();
            let ownership = host.ownership();
            reports.send_modify(|report| {
                report.kernel_changes = kernel_changes;
                report.exit_cleanups = exit_cleanups;
                report.control_relay = control_relay;
                report.ownership = ownership;
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
            let dns = dns::Attachment::new(dns_settings);
            let (peers, worker_peers) = watch::channel(None);
            let worker = platform.spawn(Box::new(move || {
                Box::pin(worker::run(worker::Inputs {
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
            }));
            Ok(Self {
                request_interrupt,
                capabilities: platform.capabilities(),
                platform,
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
        self.network_changes.borrow().path
    }

    /// A counter of [`Self::path_changes`] that outlives this borrow.
    pub fn path_counter(&self) -> impl Fn() -> u64 + Send + Sync + 'static {
        let changes = self.network_changes.clone();
        move || changes.borrow().path
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
    /// RC-1 M9 (G9; red on 61c8654f, m9/red-old-code.log): at start as at
    /// run time, a foreign kernel object and an unrecoverable journal are
    /// proven (`blocked`, read again every 60 s), a held lock, a failed read
    /// and a bootstrap resolver that did not open are retried, invalid
    /// options are refused.
    #[test]
    fn startup_failures_are_judged_as_at_run_time() {
        use super::startup_failure;
        use crate::host::OpenError;
        use meshway_protocol::ipc::Problem;
        use meshway_underlay::Error;
        assert_eq!(
            startup_failure(&Error::Journal),
            OpenError::proven(Problem::Storage)
        );
        assert_eq!(
            startup_failure(&Error::JournalVersion {
                file: "underlay-egress.json",
                version: 1
            }),
            OpenError::proven(Problem::Storage)
        );
        assert_eq!(
            startup_failure(&Error::Conflict),
            OpenError::proven(Problem::Underlay)
        );
        for retried in [
            Error::Storage(meshway_private_storage::Error::InUse),
            Error::Netlink,
            Error::Drain,
            Error::Resolver,
        ] {
            assert_eq!(
                startup_failure(&retried),
                OpenError::unknown(Problem::Underlay),
                "{retried}"
            );
        }
        assert_eq!(
            startup_failure(&Error::Options),
            Problem::Configuration.into()
        );
    }
}
