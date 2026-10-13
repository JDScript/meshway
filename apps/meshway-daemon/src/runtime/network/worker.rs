//! The unique network owner. It drives the Node's single [`Engine`]: every
//! netmap Control offers is one `reconfig`, the Engine's own timer ends an
//! expired lease, and packet, adjacency and DNS work borrow the components
//! between reconfigurations. An ended lease terminates every flow, circuit
//! and exit projection and keeps only the negative capture and DNS boundaries.
use super::{
    Command, ObservationPlan, Offer, Report, Request, dns::Attachment, plane::Plane,
    reporting::Publisher,
};
use meshway_connectivity::{AdjacencyInterest, Event};
use meshway_dataplane::{DEFAULT_HEADROOM, DEFAULT_TAILROOM, Packet, PacketPool};
use meshway_engine::Engine;
use meshway_protocol::execution::ApplyFailure;
use meshway_protocol::ipc::{DataPlane, Problem};
use meshway_underlay::Host;
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};

pub(super) struct Inputs {
    pub host: Host,
    pub commands: mpsc::Receiver<Command>,
    pub offers: watch::Receiver<Offer>,
    pub reports: Publisher,
    pub gateway: mpsc::Sender<Request>,
    pub dns: Attachment,
    pub traversal: Arc<dyn meshway_connectivity::traversal::TraversalGateway>,
    pub observation_plans: watch::Receiver<Option<ObservationPlan>>,
    pub peers: watch::Receiver<Option<meshway_control::KnownPeers>>,
    pub mtu: usize,
}

/// Covers the existing 128 queued packets, 32 sends and one 32-packet read
/// turn without allocating overflow buffers; every buffer holds one TUN MTU
/// packet. The pool belongs to this owner.
fn packet_pool(mtu: usize) -> Result<PacketPool, Problem> {
    PacketPool::new(256, mtu, DEFAULT_HEADROOM, DEFAULT_TAILROOM).map_err(|_| Problem::Tun)
}

/// Bounded retry of a transient installation failure of the same netmap.
#[derive(Default)]
struct Retry {
    failures: u8,
    at: Option<tokio::time::Instant>,
}
impl Retry {
    fn failed(&mut self, failure: ApplyFailure) {
        self.failures = self.failures.saturating_add(1);
        let transient = !matches!(failure, ApplyFailure::Validation | ApplyFailure::Dataplane);
        self.at = (transient && self.failures < 3).then(|| {
            tokio::time::Instant::now()
                + Duration::from_secs(if self.failures == 1 { 5 } else { 15 })
        });
    }
}

fn lease_end(lease: &meshway_model::NetmapLease) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::from_timestamp_millis(i64::try_from(lease.expires_at_ms).ok()?)
}

async fn sleep_until(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

pub(super) async fn run(inputs: Inputs) -> Result<(), Problem> {
    let Inputs {
        host,
        mut commands,
        mut offers,
        reports,
        gateway,
        dns,
        traversal,
        mut observation_plans,
        peers,
        mtu,
    } = inputs;
    let mut network_changes = host.network_changes();
    let mut engine = Engine::new(Plane::new(host, dns, peers, traversal)?);
    let workers = std::env::var("MESHWAY_DATAPLANE_WORKERS")
        .ok()
        .map(|value| value.parse::<usize>().map_err(|_| Problem::Worker))
        .transpose()?
        .unwrap_or_else(meshway_dataplane::secure::pipeline::Pool::default_workers);
    let mut pool =
        meshway_dataplane::secure::pipeline::Pool::new(workers).map_err(|_| Problem::Worker)?;
    let mut diagnostics = super::diagnostics::Diagnostics::default();
    let buffers = packet_pool(mtu)?;
    let mut dns_requests = engine.components_mut().dns.take_queries();
    let mut dns_reports = engine.components_mut().dns.take_reports();
    let mut exit_dns = engine.components().dns.exit_dns();
    let mut dns_queries = super::dns::mesh::Queries::default();
    let mut retry = Retry::default();
    let mut dns_failed = false;
    // The exit role's prerequisites came to hold after its content applied
    // without it: apply that content again so the role installs.
    let mut exit_recovered = false;
    // Whether the last apply saw a configured listener unbound; its Manager
    // owner retries the bind, and a change of this fact applies the content
    // again so the outcome reports it.
    let mut listener_gap = false;
    let mut listeners_changed = false;
    let mut sample_at = None;
    let mut channels = PeerChannels::default();
    let mut clamped_at: Option<tokio::time::Instant> = None;
    loop {
        meshway_logging::profile::report();
        // Addresses an ended association left captured for its live exit
        // flows are released once those flows end (checked on every wakeup;
        // a live plane wakes at least every second).
        if engine
            .desired()
            .is_some_and(|desired| !desired.retained().is_empty())
        {
            let before = engine
                .desired()
                .and_then(|desired| desired.capture_summary());
            if engine.release_retained().await.is_err() {
                tracing::warn!("Releasing the capture of ended exit flows failed");
            }
            if engine.desired().is_none() {
                diagnostics.stop().await;
                finish_withdrawal(&mut engine, &reports, true);
                continue;
            }
            let after = engine
                .desired()
                .and_then(|desired| desired.capture_summary());
            if after != before {
                let mut report = reports.borrow().clone();
                report.capture = after;
                reports.send_replace(report);
            }
        }
        enum Work {
            DnsQuery(Box<super::dns::mesh::Query>),
            DnsReport(Box<meshway_protocol::DnsAssociationReport>),
            ObservationPlan,
            DiagnosticSent,
            Command(Option<Command>),
            Offer,
            Retry,
            ExitRecovered,
            /// A listener became unavailable or bound again.
            ListenersChanged,
            Expired,
            HostFailed,
            DnsFailure,
            /// The failed DNS owner re-proved its system installation.
            DnsRecovered,
            Packet(
                Result<
                    Vec<Result<Packet, meshway_dataplane::PacketError>>,
                    meshway_underlay::Error,
                >,
            ),
            Carrier(Vec<Event>),
            CarrierClosed,
            Tick,
            SampleDue,
            NetworkChanged,
            ExitChanged,
            ExitFailed,
            /// The DNS owner's exit resolver fact changed.
            ExitDns,
            ExitReply(Result<meshway_underlay::ExitReply, meshway_underlay::Error>),
        }
        let live = engine.desired().is_some() && engine.components().secure().is_some();
        let expiry = engine.next_expiry();
        let engine_degraded = engine.degraded();
        let plane = engine.components_mut();
        let can_read = live && plane.packets.can_read(&pool);
        let has_outputs = plane.packets.has_outputs();
        let mut wake_at = live.then(|| plane.packets.wake_at()).flatten();
        if let Some(at) = live
            .then(|| {
                plane
                    .secure()
                    .and_then(meshway_dataplane::secure::SecureNode::next_keepalive)
            })
            .flatten()
        {
            let at = tokio::time::Instant::from_std(at);
            wake_at = Some(wake_at.map_or(at, |old| old.min(at)));
        }
        if let Some(at) = diagnostics.wake_at() {
            wake_at = Some(wake_at.map_or(at, |old| old.min(at)));
        }
        if let Some(at) = dns_queries.wake_at() {
            wake_at = Some(wake_at.map_or(at, |old| old.min(at)));
        }
        if plane
            .installed
            .as_ref()
            .is_some_and(|state| state.exit.is_some())
        {
            let at = tokio::time::Instant::now() + Duration::from_millis(250);
            wake_at = Some(wake_at.map_or(at, |old| old.min(at)));
        }
        if live {
            // Ready-exit facts expire; refresh the selection input regularly.
            let at = tokio::time::Instant::now() + Duration::from_secs(1);
            wake_at = Some(wake_at.map_or(at, |old| old.min(at)));
        }
        let retry_at = retry
            .at
            .filter(|_| plane.installed.is_none() || engine_degraded);
        let Plane {
            host,
            dns,
            packets,
            installed,
            ..
        } = plane;
        let work = tokio::select! { biased;
            command = commands.recv() => Work::Command(command),
            _ = sleep_until(expiry) => Work::Expired,
            _ = diagnostics.sent(), if diagnostics.has_sends() => Work::DiagnosticSent,
            changed = observation_plans.changed() => if changed.is_ok() { Work::ObservationPlan } else { Work::Command(None) },
            changed = offers.changed() => if changed.is_err() { Work::Command(None) } else { Work::Offer },
            query = dns_requests.recv() => match query { Some(query) => Work::DnsQuery(Box::new(query)), None => Work::Command(None) },
            Some(report) = dns_reports.recv() => Work::DnsReport(Box::new(report)),
            _ = sleep_until(retry_at) => Work::Retry,
            () = std::future::ready(()), if exit_recovered => Work::ExitRecovered,
            () = std::future::ready(()), if listeners_changed => Work::ListenersChanged,
            _ = dns.failed(), if !dns_failed => Work::DnsFailure,
            _ = dns.recovered(), if dns_failed => Work::DnsRecovered,
            _ = host.ended(), if installed.is_some() => Work::HostFailed,
            changed = network_changes.changed() => if changed.is_ok() { Work::NetworkChanged } else { Work::Command(None) },
            _ = sleep_until(sample_at) => Work::SampleDue,
            Ok(()) = exit_dns.changed() => Work::ExitDns,
            work = async {
                let (manager, mut exit) = match installed.as_mut() {
                    Some(state) => (state.manager.as_mut(), state.exit.as_mut()),
                    None => (None, None),
                };
                let mut health = exit.as_ref().map(|exit| exit.health.clone());
                // Packet pressure must not starve incoming handshakes or lease
                // work. Reconfiguration remains the outer priority.
                tokio::select! {
                    _ = async { match &mut health { Some(health) => { let _ = health.changed().await; }, None => std::future::pending().await } } => Work::ExitChanged,
                    reply = async { match exit.as_mut() { Some(exit) => exit.reader.recv().await, None => std::future::pending().await } }, if live => Work::ExitReply(reply),
                    _ = packets.sent(host), if has_outputs => Work::Tick,
                    _ = pool.ready(), if live && pool.has_pending() => Work::Tick,
                    result = async {
                        match host.tun() { Some(tun) => receive_packets(tun, &buffers).await.map_err(meshway_underlay::Error::Tun), None => std::future::pending().await }
                    }, if can_read => Work::Packet(result),
                    event = async {
                        match manager {
                            Some(manager) => manager.recv_batch_with_data(can_read).await,
                            None => std::future::pending().await,
                        }
                    }, if live => if event.iter().any(|event| matches!(event, Event::Closed)) { Work::CarrierClosed } else { Work::Carrier(event) },
                    _ = sleep_until(wake_at) => Work::Tick,
                    else => std::future::pending().await,
                }
            } => work,
        };
        // Resource owners still observe the same lease end. Their EOF can win
        // the select against the Engine timer; it is the normal expiry, not a
        // transport/TUN installation failure.
        let work = if (matches!(work, Work::CarrierClosed | Work::Packet(Err(_)))
            && engine
                .next_expiry()
                .is_some_and(|at| tokio::time::Instant::now() >= at))
            // A suspended host's monotonic timer missed the wall-clock end.
            || (matches!(work, Work::Tick) && engine.lease_ended())
        {
            Work::Expired
        } else {
            work
        };
        let work = match work {
            Work::ExitChanged => {
                let failed = engine
                    .components_mut()
                    .installed
                    .as_mut()
                    .and_then(|state| state.exit.as_mut())
                    .is_some_and(|exit| {
                        exit.health.has_changed().is_err() || exit.health.borrow_and_update().failed
                    });
                if failed { Work::ExitFailed } else { Work::Tick }
            }
            work => work,
        };
        // The capability fact is a timed observation: refresh it well within
        // the State report lifetime the Server enforces. Checked on every
        // event, not only idle ticks: continuous exit traffic never idles, and
        // any sample taken below must not carry an expired observation.
        let stale = reports
            .borrow()
            .exit_capability
            .as_ref()
            .is_some_and(|fact| {
                chrono::Utc::now() - fact.observed_at
                    >= chrono::Duration::seconds(meshway_protocol::STATE_REPORT_TTL_SECONDS / 3)
            });
        if stale && engine.desired().is_some() {
            let capability = engine.components_mut().exit_capability().await;
            exit_recovered = engine.components().exit_pending()
                && capability
                    .as_ref()
                    .is_some_and(|fact| fact.status == meshway_model::exit::ExitStatus::Ready);
            let mut report = reports.borrow().clone();
            report.exit_capability = capability;
            reports.send_replace(report);
        }
        let structural = matches!(
            &work,
            Work::Offer
                | Work::Retry
                | Work::DnsRecovered
                | Work::ExitRecovered
                | Work::ListenersChanged
        ) || matches!(&work, Work::Carrier(events) if events.iter().any(|event| matches!(event, Event::Changed)));
        match work {
            Work::DnsQuery(query) => dns_queries.enqueue(*query),
            Work::DnsReport(report) => {
                if gateway.try_send(Request::DnsAssociation(report)).is_err() {
                    tracing::debug!("Control queue full; DNS association report dropped");
                }
                continue;
            }
            Work::NetworkChanged => {
                network_changes.borrow_and_update();
                let report =
                    sample_host(engine.components(), &diagnostics, reports.borrow().clone());
                reports.send_replace(report);
                continue;
            }
            Work::ExitDns => {
                let mut report = reports.borrow().clone();
                report.exit_dns = exit_dns.borrow_and_update().clone();
                reports.send_replace(report);
                continue;
            }
            Work::SampleDue => {
                let report =
                    sample_host(engine.components(), &diagnostics, reports.borrow().clone());
                reports.send_replace(report);
                sample_at = None;
                continue;
            }
            Work::ObservationPlan => {
                diagnostics.plan(observation_plans.borrow_and_update().clone());
            }
            Work::Command(Some(Command::Ping(request))) => {
                let instance = engine
                    .components()
                    .diagnostic_scope()
                    .and_then(|(_, instance)| instance);
                let scope = engine
                    .components_mut()
                    .installed
                    .as_mut()
                    .filter(|_| live)
                    .and_then(|state| Some((&state.desired, instance, state.secure.as_mut()?)));
                diagnostics.ping(scope, request);
            }
            Work::Command(Some(Command::Netcheck(reply))) => {
                let probe = engine
                    .components()
                    .installed
                    .as_ref()
                    .and_then(|state| state.manager.as_ref())
                    .and_then(|manager| manager.stun_probe().ok());
                let _ = reply.send(probe);
                continue;
            }
            Work::Command(Some(Command::Builder(reply))) => {
                let _ = reply.send(
                    engine
                        .components()
                        .host
                        .client_builder()
                        .map_err(|_| Problem::Underlay),
                );
                continue;
            }
            Work::Command(Some(Command::Sample(reply))) => {
                let report =
                    sample_host(engine.components(), &diagnostics, reports.borrow().clone());
                let _ = reply.send(reports.sample(report));
                sample_at = None;
                continue;
            }
            Work::Command(Some(Command::Shutdown { keep_sink })) => {
                return shutdown(engine, &mut dns_queries, &mut diagnostics, keep_sink).await;
            }
            Work::Command(None) => {
                return shutdown(engine, &mut dns_queries, &mut diagnostics, true).await;
            }
            Work::Command(Some(Command::Stop { down, reply })) => {
                dns_queries.stop().await;
                diagnostics.stop().await;
                let closing = engine.reconfig(None).await.map_err(|_| Problem::Underlay);
                retry = Retry::default();
                let plane = engine.components_mut();
                let dns_result = if down {
                    plane
                        .dns
                        .down(&mut plane.host)
                        .await
                        .map_err(|_| Problem::Dns)
                } else {
                    Ok(())
                };
                let host_result = if down && dns_result.is_ok() {
                    dns_failed = false;
                    plane.host.down().await
                } else {
                    plane.host.suspend().await
                }
                .map_err(|_| Problem::Underlay);
                let result = dns_result.and(closing).and(host_result);
                let mut report = Report::stopped();
                if result.is_err() {
                    report.plane = DataPlane::Failed;
                    report.problem = result.err();
                } else if !down {
                    report.plane = DataPlane::Suspended;
                }
                reports.send_replace(report);
                let _ = reply.send(result);
                continue;
            }
            Work::Offer
            | Work::Retry
            | Work::DnsRecovered
            | Work::ExitRecovered
            | Work::ListenersChanged => {
                // An offer and a recovered component are new inputs; a retry
                // continues the bounded backoff of the same failure. A newer
                // offer pending takes precedence over reapplying the current one.
                let reapply = matches!(work, Work::ExitRecovered | Work::ListenersChanged)
                    && offers.has_changed().is_ok_and(|changed| !changed);
                if reapply && matches!(work, Work::ExitRecovered) {
                    tracing::info!("Exit prerequisites hold; installing the exit role");
                }
                if reapply && matches!(work, Work::ListenersChanged) {
                    tracing::info!("Listener availability changed; applying the netmap again");
                }
                exit_recovered = false;
                listeners_changed = false;
                if !matches!(work, Work::Retry) {
                    retry = Retry::default();
                }
                if matches!(work, Work::DnsRecovered) {
                    tracing::info!("Required DNS component recovered; reinstalling the netmap");
                    dns_failed = false;
                }
                let offer = offers.borrow_and_update().clone();
                if let Some(floor) = offer.clock_floor_ms {
                    engine.raise_clock_floor(floor);
                }
                engine.components_mut().set_keys(offer.keys);
                let rebound = engine.components_mut().set_instance(offer.instance).await;
                let Some(netmap) = offer.netmap else {
                    // Control withdrew the lease: end the installed netmap.
                    if engine.desired().is_some() || engine.components().installed.is_some() {
                        diagnostics.invalidate();
                        diagnostics.stop().await;
                        withdraw(&mut engine, &reports, false).await;
                    }
                    continue;
                };
                let reference = netmap.content.reference();
                // Same healthy content and associations: Engine extends its
                // own timer only. Changed associations, other content and a
                // degraded installation apply and report below.
                // Reapplying keeps the installed content's work and report.
                let change = (!reapply).then(|| {
                    super::transitions::change(engine.desired(), engine.degraded(), &netmap)
                });
                if change == Some(super::transitions::Change::Renewal) {
                    if engine.reconfig(Some(&netmap)).await.is_err() {
                        tracing::warn!(
                            netmap_serial = reference.serial.get(),
                            "Netmap lease not accepted"
                        );
                    } else {
                        let mut report = reports.borrow().clone();
                        report.expires_at = lease_end(&netmap.lease);
                        if rebound {
                            report.exit_capability =
                                engine.components_mut().exit_capability().await;
                            // Content applied before the instance arrived
                            // could not observe the exit prerequisites.
                            exit_recovered = engine.components().exit_pending()
                                && report.exit_capability.as_ref().is_some_and(|fact| {
                                    fact.status == meshway_model::exit::ExitStatus::Ready
                                });
                        }
                        reports.send_replace(report);
                    }
                    // A first instance rebinds Native UDP listeners as
                    // coordination sockets; one may now be unavailable.
                    listeners_changed |=
                        rebound && engine.components().listeners_unavailable() != listener_gap;
                    continue;
                }
                let previous = reports.borrow().clone();
                let mut report = previous.clone();
                // Changed associations keep the installed content's work: its
                // report stays until the new desired state's outcome below.
                if change == Some(super::transitions::Change::Content) {
                    report.plane = DataPlane::Preparing;
                    report.problem = None;
                    report.problem_detail = None;
                    report.netmap = Some(reference);
                    report.completion = None;
                    report.facts = None;
                    report.content = None;
                    report.generation = report
                        .generation
                        .checked_next()
                        .unwrap_or(report.generation);
                    reports.send_replace(report.clone());
                }
                let plane = engine.components_mut();
                plane.report = report;
                plane.failure = None;
                plane.failure_detail = None;
                let result = if reapply {
                    engine.reapply().await
                } else {
                    engine.reconfig(Some(&netmap)).await
                };
                let kept = engine.desired().is_some();
                listener_gap = engine.components().listeners_unavailable();
                let capture = engine
                    .desired()
                    .and_then(|desired| desired.capture_summary());
                let plane = engine.components_mut();
                let mut report = std::mem::replace(&mut plane.report, Report::stopped());
                report.dns_guard = plane.dns.guard();
                report.exit_dns = exit_dns.borrow_and_update().clone();
                report.observed_at = chrono::Utc::now();
                match result {
                    Ok(()) => {
                        dns_failed = false;
                        // A complete apply ends an earlier degraded one's
                        // problem (a listener that bound on retry).
                        report.problem = None;
                        report.problem_detail = None;
                        report.completion = Some(Ok(()));
                        report.content = Some(netmap.content.clone());
                        report.capture = capture;
                        report.expires_at = lease_end(&netmap.lease);
                        report.plane = if plane.secure().is_some() {
                            DataPlane::Ready
                        } else {
                            DataPlane::Stopped
                        };
                    }
                    Err(meshway_engine::Error::Degraded) => {
                        let (problem, failure) = plane
                            .failure
                            .take()
                            .unwrap_or((Problem::Transport, ApplyFailure::Listeners));
                        tracing::warn!(
                            ?problem,
                            ?failure,
                            netmap_serial = reference.serial.get(),
                            "Node netmap installed without some capability; retrying"
                        );
                        // Installed and safe, but not a complete application.
                        report.completion = Some(Err(failure));
                        report.problem = Some(problem);
                        report.problem_detail = plane.failure_detail.take();
                        report.content = Some(netmap.content.clone());
                        report.capture = capture;
                        report.expires_at = lease_end(&netmap.lease);
                        report.plane = if plane.secure().is_some() {
                            DataPlane::Ready
                        } else {
                            DataPlane::Failed
                        };
                        retry.failed(failure);
                    }
                    Err(error) if plane.failure.is_none() && kept => {
                        // Rejected input (regressing reference or stale lease):
                        // Engine kept its current desired content unchanged.
                        tracing::warn!(
                            ?error,
                            netmap_serial = reference.serial.get(),
                            "Netmap offer rejected by Engine"
                        );
                        reports.send_replace(previous);
                        continue;
                    }
                    Err(error) => {
                        let (problem, failure) = plane
                            .failure
                            .take()
                            .unwrap_or((Problem::Apply, ApplyFailure::Validation));
                        tracing::warn!(
                            ?problem,
                            ?failure,
                            ?error,
                            netmap_serial = reference.serial.get(),
                            "Node netmap installation did not complete"
                        );
                        diagnostics.invalidate();
                        report.completion = Some(Err(failure));
                        report.problem = Some(problem);
                        report.problem_detail = plane.failure_detail.take();
                        report.plane = DataPlane::Failed;
                        retry.failed(failure);
                    }
                }
                reports.send_replace(report);
            }
            Work::Expired => {
                // An ended association only narrows the desired state; the
                // lease's diagnostics stop with the lease alone.
                let (withdrawn, failed) =
                    super::transitions::expire(&mut engine, async || diagnostics.stop().await)
                        .await;
                if failed {
                    tracing::warn!("Netmap expiry cleanup failed");
                }
                if withdrawn {
                    tracing::info!("Netmap lease ended; terminating the packet plane");
                    finish_withdrawal(&mut engine, &reports, false);
                } else {
                    let mut report = reports.borrow().clone();
                    report.capture = engine
                        .desired()
                        .and_then(|desired| desired.capture_summary());
                    reports.send_replace(report);
                }
                continue;
            }
            Work::HostFailed => {
                diagnostics.stop().await;
                withdraw(&mut engine, &reports, true).await;
                continue;
            }
            Work::ExitFailed
            | Work::ExitReply(Err(_))
            | Work::CarrierClosed
            | Work::Packet(Err(_))
            | Work::DnsFailure => {
                diagnostics.invalidate();
                let dns_error = matches!(work, Work::DnsFailure);
                if dns_error {
                    tracing::warn!("Required DNS component failed; suspending packet plane");
                }
                dns_failed |= dns_error;
                let cleanup = engine.reconfig(None).await;
                let mut report = reports.borrow().clone();
                report.plane = DataPlane::Failed;
                report.content = None;
                report.problem_detail = None;
                report.problem = Some(if cleanup.is_err() {
                    Problem::Underlay
                } else {
                    match work {
                        Work::DnsFailure => Problem::Dns,
                        Work::Packet(_) => Problem::Tun,
                        Work::CarrierClosed => Problem::Transport,
                        _ => Problem::Underlay,
                    }
                });
                report.completion = Some(Err(match report.problem {
                    Some(Problem::Dns) => ApplyFailure::Dns,
                    Some(Problem::Tun) => ApplyFailure::Tun,
                    Some(Problem::Transport) => ApplyFailure::Listeners,
                    _ => ApplyFailure::Underlay,
                }));
                report.observed_at = chrono::Utc::now();
                report.facts = None;
                report.exit_flows = None;
                if let Some(capability) = &mut report.exit_capability {
                    capability.installed_families.clear();
                }
                retry.failed(
                    report
                        .completion
                        .and_then(Result::err)
                        .unwrap_or(ApplyFailure::Underlay),
                );
                reports.send_replace(report);
                continue;
            }
            Work::Packet(Ok(received)) => {
                let packets = &mut engine.components_mut().packets;
                for packet in received {
                    match packet {
                        Ok(packet) => packets.enqueue(packet),
                        Err(meshway_dataplane::PacketError::Unsupported) => {
                            packets.drop_packet("unsupported_ip_or_local_control")
                        }
                        Err(_) => packets.drop_packet("malformed_ip"),
                    }
                }
            }
            Work::ExitReply(Ok(reply)) => {
                let Plane {
                    installed, packets, ..
                } = engine.components_mut();
                if let Some(state) = installed
                    && let Some(secure) = &mut state.secure
                {
                    match Packet::parse(reply.packet) {
                        Ok(packet) => match secure
                            .exit_reply(meshway_dataplane::exit::ExitReplyIngress::new(packet))
                        {
                            Ok(frame) => {
                                if let Some(exit) = &mut state.exit {
                                    exit.drive(secure);
                                }
                                packets.exit_reply(frame);
                            }
                            Err(_) => packets.drop_packet("exit_reply_admission"),
                        },
                        Err(_) => packets.drop_packet("exit_reply_malformed"),
                    }
                }
            }
            Work::Carrier(events) => {
                let Plane {
                    installed, packets, ..
                } = engine.components_mut();
                for event in events {
                    let Event::Frame {
                        link,
                        frame,
                        received_at,
                    } = event
                    else {
                        continue;
                    };
                    if let Some(state) = installed.as_ref()
                        && let Some(secure) = &state.secure
                    {
                        packets.incoming(
                            *link,
                            frame.payload.into(),
                            received_at,
                            &state.desired,
                            secure,
                        );
                    }
                }
            }
            Work::Tick => {}
            Work::DiagnosticSent => {}
            Work::ExitChanged => unreachable!("mapped above"),
        }
        let live = engine.desired().is_some();
        let instance = engine
            .components()
            .diagnostic_scope()
            .and_then(|(_, instance)| instance);
        let Plane {
            host,
            dns,
            packets,
            installed,
            peers,
            ..
        } = engine.components_mut();
        if live
            && let Some(state) = installed
            && let (Some(secure), Some(manager)) = (&mut state.secure, &state.manager)
        {
            // One scheduling view per turn. It grants no I/O authority: every
            // output still captures and checks the Manager's actual channel,
            // while SecureNode checks its own packet/circuit admission.
            let carriers = manager.carriers().ok();
            if let Some(carriers) = &carriers {
                let _ = secure.set_carriers(carriers.clone());
            }
            // Measured inner MTU of each current channel: oversized local
            // packets are answered with fragmentation needed on the TUN,
            // and the TCP MSS of SYNs is clamped to it (refreshed each second).
            if let Ok(mtu) = manager.path_mtu() {
                secure.set_path_limits(
                    mtu.into_iter()
                        .map(|(peer, limit)| (peer, limit.inner))
                        .collect(),
                );
            }
            if clamped_at.is_none_or(|at| at.elapsed() >= Duration::from_secs(1)) {
                clamped_at = Some(tokio::time::Instant::now());
                if let Err(error) = host.clamp_mss(secure.clamps()) {
                    tracing::debug!(%error, "TCP MSS clamp not updated");
                }
            }
            // A restarted adjacent peer lost its circuits: end them here and
            // pass a Close on to the surviving side of each relayed one.
            if let Ok(boots) = manager.peer_boots() {
                packets.closes(secure.set_peer_boots(boots));
            }
            {
                let known = peers.borrow();
                let now = chrono::Utc::now();
                secure.set_ready_exits(
                    known
                        .as_ref()
                        .map(|known| known.ready_exits(now))
                        .unwrap_or_default(),
                );
                if let Some(current) = channels.changed(known.as_ref(), now) {
                    secure.set_peer_channels(current);
                }
            }
            diagnostics.synchronize(&state.desired, instance);
            tokio::select! { biased;
                _ = host.ended() => {},
                _ = dns.failed(), if !dns_failed => {},
                _ = async {
                    static PROFILE: meshway_logging::profile::Metric = meshway_logging::profile::Metric::new("host.drive.poll");
                    PROFILE.poll(packets.drive(secure, manager, carriers.as_ref(), host, &state.desired, &mut diagnostics, &mut dns_queries, dns.dispatcher(), &mut pool, state.exit.as_mut())).await;
                    dns_queries.drive(secure, &state.desired, manager).await;
                    diagnostics.drive(secure, manager, carriers.as_ref(), &gateway);
                } => {},
            }
            let interest = manager.interest();
            for wanted in secure.take_adjacency_interests() {
                interest.interest(wanted.peer);
            }
        } else {
            dns_queries.stop().await;
        }
        if structural
            && engine.desired().is_some()
            && engine.components().listeners_unavailable() != listener_gap
        {
            listeners_changed = true;
        }
        if structural {
            let report = sample_host(engine.components(), &diagnostics, reports.borrow().clone());
            reports.send_replace(report);
            sample_at = None;
        } else {
            // Packet/diagnostic work may change circuit dependencies. Coalesce
            // their observation, not packet processing: the first dirty event
            // sets a deadline that later traffic cannot postpone.
            sample_at.get_or_insert_with(|| {
                tokio::time::Instant::now() + super::reporting::CHANGE_INTERVAL
            });
        }
    }
}

/// Recomputes the peers' reported channels only when the facts or their
/// freshness change, not on every packet turn.
#[derive(Default)]
struct PeerChannels {
    facts: Option<
        std::sync::Arc<
            std::collections::BTreeMap<meshway_model::NodeId, meshway_protocol::PeerFacts>,
        >,
    >,
    until: Option<chrono::DateTime<chrono::Utc>>,
}
impl PeerChannels {
    fn changed(
        &mut self,
        known: Option<&meshway_control::KnownPeers>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Option<
        std::collections::BTreeMap<
            meshway_model::NodeId,
            std::collections::BTreeSet<meshway_model::NodeId>,
        >,
    > {
        let facts = known.map(|known| &known.peers);
        let same = match (&self.facts, facts) {
            (Some(old), Some(new)) => std::sync::Arc::ptr_eq(old, new),
            (None, None) => true,
            _ => false,
        };
        if same && self.until.is_none_or(|until| now < until) {
            return None;
        }
        self.facts = facts.cloned();
        self.until = facts.and_then(|facts| {
            facts
                .values()
                .map(|facts| facts.expires_at)
                .filter(|at| *at > now)
                .min()
        });
        Some(known.map(|known| known.channels(now)).unwrap_or_default())
    }
}

async fn shutdown(
    mut engine: Engine<Plane>,
    dns_queries: &mut super::dns::mesh::Queries,
    diagnostics: &mut super::diagnostics::Diagnostics,
    keep_sink: bool,
) -> Result<(), Problem> {
    dns_queries.stop().await;
    diagnostics.stop().await;
    let closing = engine.reconfig(None).await.map_err(|_| Problem::Underlay);
    let mut plane = engine.into_components();
    let dns_result = plane
        .dns
        .shutdown(&mut plane.host, keep_sink)
        .await
        .map_err(|_| Problem::Dns);
    let host_result = plane
        .host
        .shutdown(keep_sink)
        .await
        .map_err(|_| Problem::Underlay);
    dns_result.and(closing).and(host_result)
}

/// Control withdrew the lease or the Host lost its physical owner: apply
/// empty, which terminates every flow, circuit and exit projection and keeps
/// the negative capture and DNS boundary.
async fn withdraw(engine: &mut Engine<Plane>, reports: &Publisher, host_failed: bool) {
    tracing::info!(
        host_failed,
        "Netmap withdrawn; terminating the packet plane"
    );
    let cleanup = engine.reconfig(None).await;
    finish_withdrawal(engine, reports, host_failed || cleanup.is_err());
}

fn finish_withdrawal(engine: &mut Engine<Plane>, reports: &Publisher, failed: bool) {
    let failed = failed || engine.components().failure.is_some();
    engine.components_mut().failure = None;
    let mut report = reports.borrow().clone();
    report.plane = if failed {
        DataPlane::Failed
    } else {
        DataPlane::Suspended
    };
    report.problem = failed.then_some(Problem::Underlay);
    report.problem_detail = None;
    report.completion = None;
    report.content = None;
    report.expires_at = None;
    report.facts = None;
    report.exit_flows = None;
    if let Some(capability) = &mut report.exit_capability {
        capability.installed_families.clear();
    }
    reports.send_replace(report);
}

/// A cancellation-safe TUN read has no read-ahead owner. After the first packet,
/// drain only immediately-ready reads, bounded to 32; lifecycle work then regains
/// priority. Packet admission still happens individually in the unique engine.
async fn receive_packets(
    tun: &meshway_tun::Tun,
    buffers: &PacketPool,
) -> Result<Vec<Result<Packet, meshway_dataplane::PacketError>>, meshway_tun::Error> {
    tun.recv_batch_map(|bytes| {
        let mut buffer = buffers
            .acquire(bytes.len())
            .map_err(|_| meshway_tun::Error::Io(std::io::Error::other("packet pool exhausted")))?;
        buffer.copy_from_slice(bytes);
        Ok(Packet::parse(buffer))
    })
    .await?
    .into_iter()
    .collect()
}

/// An explicit measurement by the resource owner. Application observation time
/// remains immutable; sampling current sockets is distinct from receipt replay.
fn sample_host(
    plane: &Plane,
    diagnostics: &super::diagnostics::Diagnostics,
    mut report: Report,
) -> Report {
    report.interface_epoch = *plane.host.network_changes().borrow();
    report.owned_interfaces = plane.host.owned_interfaces();
    report.known_peers = Some(plane.peers.clone());
    sample(&plane.installed, diagnostics, report)
}

fn sample(
    installed: &Option<super::plane::Installed>,
    diagnostics: &super::diagnostics::Diagnostics,
    mut report: Report,
) -> Report {
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("host.sample");
    let _profile = PROFILE.enter();
    let secure = installed.as_ref().and_then(|state| state.secure.as_ref());
    meshway_logging::profile::gauge(
        "secure.circuit_count",
        secure.map(|secure| secure.circuit_count() as u64),
    );
    meshway_logging::profile::gauge(
        "flow.count",
        secure.map(|secure| secure.flow_count() as u64),
    );
    meshway_logging::profile::gauge("secure.ready_circuits", None);
    report.facts = None;
    report.exit_flows = installed
        .as_ref()
        .and_then(|s| s.exit.as_ref())
        .map(|exit| exit.health.borrow().flows);
    report.dependencies.clear();
    report.observation_dependencies.clear();
    report.circuits.clear();
    report.routes.clear();
    report.relays.clear();
    report.rtts = diagnostics.rtts().clone();
    report.facts_observed_at = chrono::Utc::now();
    if let Some(state) = installed
        && let (Some(secure), Some(manager)) = (&state.secure, &state.manager)
    {
        report.facts = manager.snapshot().ok();
        if let Ok(circuits) = secure.ready_circuits() {
            let node = state.desired.netmap().node_id;
            meshway_logging::profile::gauge("secure.ready_circuits", Some(circuits.len() as u64));
            for circuit in circuits {
                let far = match (circuit.path.first(), circuit.path.last()) {
                    (Some(first), Some(last)) if *first == node => Some(*last),
                    (Some(first), Some(last)) if *last == node => Some(*first),
                    _ => None,
                };
                if let Some(far) = far {
                    let mut path = circuit.path.clone();
                    if path.first() != Some(&node) {
                        path.reverse();
                    }
                    let shorter = report
                        .routes
                        .get(&far)
                        .is_none_or(|known| path.len() < known.len());
                    if shorter {
                        report.routes.insert(far, path);
                    }
                }
                if circuit.grant.purpose == meshway_dataplane::Purpose::Business {
                    use meshway_dataplane::secure::CircuitRole;
                    use meshway_model::state::ReporterRole;
                    let role = match circuit.role {
                        CircuitRole::Origin => ReporterRole::Origin,
                        CircuitRole::Relay => ReporterRole::Relay,
                        CircuitRole::Terminal => match circuit.grant.target {
                            meshway_dataplane::Target::Exit { .. } => ReporterRole::Exit,
                            meshway_dataplane::Target::Mesh { .. } => ReporterRole::Target,
                        },
                    };
                    report
                        .circuits
                        .insert(circuit.id, (circuit.path.clone(), role));
                }
                for pair in circuit.path.windows(2) {
                    let peer = if pair[0] == node {
                        Some(pair[1])
                    } else if pair[1] == node {
                        Some(pair[0])
                    } else {
                        None
                    };
                    let Some(peer) = peer else {
                        continue;
                    };
                    match circuit.grant.purpose {
                        meshway_dataplane::Purpose::Dns => {}
                        meshway_dataplane::Purpose::Business => {
                            *report.dependencies.entry(peer).or_default() += 1
                        }
                        meshway_dataplane::Purpose::Diagnostic => {
                            let meshway_dataplane::Target::Mesh { node } = &circuit.grant.target
                            else {
                                continue;
                            };
                            let ids = report.observation_dependencies.entry(peer).or_default();
                            for id in diagnostics.observations_toward(*node) {
                                if !ids.contains(&id) {
                                    ids.push(id);
                                }
                            }
                        }
                    }
                }
            }
            // A peer reached through several circuits shows the one a new
            // flow joins, in SecureNode's path order, not the first in ID order.
            for (far, path) in &mut report.routes {
                if let Some(joined) = secure.joined_route(*far) {
                    *path = joined;
                }
            }
            // Bounded by ID order, the same set on every sample.
            while report.circuits.len() > meshway_protocol::STATE_MAX_PATHS {
                report.circuits.pop_last();
            }
        }
        // Only needed peers without a channel or circuit; bounded by the
        // adjacency needs the Manager tracks.
        if let Some(facts) = &report.facts {
            for (peer, status) in &facts.adjacencies {
                if *status == meshway_model::state::AdjacencyStatus::Unreachable
                    && !report.routes.contains_key(peer)
                    && let Some(path) = secure.relay_route(*peer)
                {
                    report.relays.insert(*peer, path);
                }
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::packet_pool;

    #[test]
    fn packet_pool_holds_one_full_mtu_packet() {
        for mtu in [1280, meshway_dataplane::secure::DEFAULT_INNER_MTU, 1500] {
            let pool = packet_pool(mtu).unwrap();
            assert!(pool.acquire(mtu).is_ok(), "mtu {mtu}");
            assert!(pool.acquire(mtu + 1).is_err(), "mtu {mtu}");
        }
    }
}
