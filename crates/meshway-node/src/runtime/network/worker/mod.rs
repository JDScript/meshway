//! The unique network owner. It drives the Node's single [`Engine`]: every
//! netmap Control offers is one `reconfig`, the Engine's own timer ends an
//! expired lease, and packet, adjacency and DNS work borrow the components
//! between reconfigurations. An ended lease terminates every flow, circuit
//! and exit projection and keeps only the negative capture and DNS boundaries.
use super::{
    Command, ObservationPlan, Offer, Report, Request, dns::Attachment, plane::Plane,
    reporting::Publisher,
};
use crate::host::Host;
use meshway_connectivity::{AdjacencyInterest, Event};
use meshway_dataplane::{DEFAULT_HEADROOM, DEFAULT_TAILROOM, Packet, PacketPool};
use meshway_engine::Engine;
use meshway_protocol::execution::ApplyFailure;
use meshway_protocol::ipc::{DataPlane, Problem};
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};

#[cfg(target_os = "macos")]
mod dns_packets;
mod lifecycle;
mod packets;
mod peers;
mod sample;
mod timing;
mod withdrawal;
use packets::{packet_pool, receive_packets};
use peers::PeerChannels;
use sample::{RoamEvidence, sample_host};
use timing::{apply_failed, apply_recovery, apply_retry_at, lease_end, sleep_until};
use withdrawal::{Cause, Withdrawn, finish_withdrawal, shutdown, withdraw, withdrawing};

/// Exit replies taken in one turn, like one Mesh TUN read turn.
const EXIT_REPLY_BATCH: usize = 32;

pub(super) struct Inputs {
    pub host: Box<dyn Host>,
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
    let mut path = network_changes.borrow_and_update().path;
    let mut overlaps = host.pool_overlap();
    let mut repairs = host.repairs();
    repairs.borrow_and_update();
    // An external address inside a Mesh pool: nothing installs while it
    // lasts, and its departure installs the offered netmap again.
    let mut overlap: Option<meshway_underlay::PoolOverlap> = overlaps.borrow_and_update().clone();
    // Every fail-closed owner's health, consumed as edges (kernel ownership,
    // the DNS owner, queued reinstalls of the same netmap).
    let mut healths = lifecycle::Healths::new(host.ownership());
    // A path change re-probes the exit prerequisites after reporting `probing`.
    let mut exit_probe_at: Option<tokio::time::Instant> = None;
    let mut roams = RoamEvidence::default();
    let local_diagnostics = host.diagnostics();
    let mut engine = Engine::new(Plane::new(host, dns, peers, traversal)?);
    let workers = std::env::var("MESHWAY_DATAPLANE_WORKERS")
        .ok()
        .map(|value| value.parse::<usize>().map_err(|_| Problem::Worker))
        .transpose()?
        .unwrap_or_else(meshway_dataplane::secure::pipeline::Pool::default_workers);
    let mut pool =
        meshway_dataplane::secure::pipeline::Pool::new(workers).map_err(|_| Problem::Worker)?;
    let mut diagnostics = super::diagnostics::Diagnostics::default();
    diagnostics.externals.attach(local_diagnostics);
    let buffers = packet_pool(mtu)?;
    let mut dns_requests = engine.components_mut().dns.take_queries();
    let mut dns_reports = engine.components_mut().dns.take_reports();
    let mut exit_dns = engine.components().dns.exit_dns();
    let mut dns_queries = super::dns::mesh::Queries::default();
    #[cfg(target_os = "macos")]
    let mut dns_output: Option<meshway_dns::packet::Output> = None;
    // The worker's own recovery of a failed installation of the same netmap.
    let mut retry = apply_recovery();
    let mut dns_failed = false;
    // A failure withdrew installed content and nothing installed since.
    let mut withdrawn: Option<Withdrawn> = None;
    // Whether the last apply saw a configured listener unbound; its Manager
    // owner retries the bind, and a change of this fact applies the content
    // again so the outcome reports it (a queued reinstall).
    let mut listener_gap = false;
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
                withdrawn.get_or_insert(Withdrawn::now(Problem::Underlay));
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
            #[cfg(target_os = "macos")]
            DnsPacket(Option<meshway_dns::packet::Output>),
            #[cfg(target_os = "macos")]
            DnsPacketSent(Result<(), meshway_tun::Error>),
            DnsQuery(Box<super::dns::mesh::Query>),
            DnsReport(Box<meshway_protocol::DnsAssociationReport>),
            ObservationPlan,
            DiagnosticSent,
            Command(Option<Command>),
            Offer,
            Retry,
            /// The exit role's prerequisites came to hold after its content
            /// applied without it: apply that content again.
            ExitRecovered,
            /// A listener became unavailable or bound again.
            ListenersChanged,
            Expired,
            HostFailed,
            /// An owner's health changed, a re-proof is due or a reinstall
            /// was queued: [`lifecycle::decide`] says what follows.
            Lifecycle(lifecycle::Edge),
            /// An unavailable end of kernel ownership is due for its re-proof.
            HostRecovery,
            /// The Host proved its ownership again in a new term.
            HostRecovered,
            DnsFailure,
            /// The failed DNS owner re-proved its system installation.
            DnsRecovered,
            Packet(Result<Vec<Result<Packet, meshway_dataplane::PacketError>>, crate::host::Error>),
            Carrier(Vec<Event>),
            CarrierClosed,
            Tick,
            SampleDue,
            NetworkChanged,
            PoolOverlap,
            Repair,
            ExitChanged,
            ExitFailed,
            /// The DNS owner's exit resolver fact changed.
            ExitDns,
            ExitReply(Result<Vec<Vec<u8>>, crate::host::Error>),
        }
        let live = engine.desired().is_some() && engine.components().secure().is_some();
        let expiry = engine.next_expiry();
        let engine_degraded = engine.degraded();
        let plane = engine.components_mut();
        let can_read = live && plane.packets.can_read(&pool);
        #[cfg(target_os = "macos")]
        let packet_dns = plane.host.dns_packets();
        #[cfg(target_os = "macos")]
        let read_tun = can_read || packet_dns.is_some();
        #[cfg(not(target_os = "macos"))]
        let read_tun = can_read;
        let can_take_replies = live && plane.packets.can_take_exit_replies(EXIT_REPLY_BATCH);
        let has_outputs = plane.packets.has_outputs();
        let mut wake_at = live.then(|| plane.packets.wake_at(&pool)).flatten();
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
        if let Some(at) = exit_probe_at {
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
        // A failed DNS preparation waits for the DNS owner's own recovery
        // too, which judged it (proven: read again every 60 s).
        let retry_at = match (apply_retry_at(&retry), plane.dns.prepare_at()) {
            (Some(apply), Some(prepare)) => Some(apply.max(prepare)),
            (apply, _) => apply,
        }
        .filter(|_| (plane.installed.is_none() || engine_degraded) && overlap.is_none());
        let recovery_at = plane.host.recovery_at().map(tokio::time::Instant::from_std);
        healths.follow_dns(plane.dns.guard());
        let repair_at = plane.host.repair_at().map(tokio::time::Instant::from_std);
        let Plane {
            host,
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
            changed = overlaps.changed() => if changed.is_ok() { Work::PoolOverlap } else { Work::Command(None) },
            changed = repairs.changed() => if changed.is_ok() { Work::Repair } else { Work::Command(None) },
            _ = sleep_until(repair_at) => Work::Repair,
            edge = healths.next(recovery_at) => Work::Lifecycle(edge),
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
                    work = async {
                        #[cfg(target_os = "macos")]
                        { tokio::select! {
                            output = dns_packets::receive(&packet_dns), if dns_output.is_none() => Work::DnsPacket(output),
                            result = dns_packets::send(host.as_ref(), dns_output.as_ref()), if dns_output.is_some() => Work::DnsPacketSent(result),
                        } }
                        #[cfg(not(target_os = "macos"))]
                        { std::future::pending::<Work>().await }
                    } => work,
                    _ = async { match &mut health { Some(health) => { let _ = health.changed().await; }, None => std::future::pending().await } } => Work::ExitChanged,
                    reply = async { match exit.as_mut() { Some(exit) => exit.recv_batch(EXIT_REPLY_BATCH).await, None => std::future::pending().await } }, if can_take_replies => Work::ExitReply(reply),
                    _ = packets.sent(host.as_ref()), if has_outputs => Work::Tick,
                    _ = pool.ready(), if live && pool.has_pending() => Work::Tick,
                    result = async {
                        match host.tun() { Some(tun) => receive_packets(tun, &buffers).await.map_err(crate::host::Error::Tun), None => std::future::pending().await }
                    }, if read_tun => Work::Packet(result),
                    event = async {
                        match manager {
                            Some(manager) => manager.recv_batch_with_data(can_read).await,
                            None => std::future::pending().await,
                        }
                    }, if live => if event.iter().any(|event| matches!(event, Event::Closed)) { Work::CarrierClosed } else { Work::Carrier(event) },
                    _ = sleep_until(wake_at) => {
                        // Diagnostics: a timer that fires late means this
                        // worker was not polled although its work was due.
                        if let Some(late) = wake_at.map(|at| at.elapsed()).filter(|late| *late >= Duration::from_millis(100)) {
                            tracing::debug!(late_ms = u64::try_from(late.as_millis()).unwrap_or(u64::MAX), "Network worker woke late");
                        }
                        Work::Tick
                    },
                    else => std::future::pending().await,
                }
            } => work,
        };
        #[cfg(target_os = "macos")]
        let work = match work {
            Work::DnsPacketSent(Err(error)) => {
                dns_output = None;
                Work::Packet(Err(crate::host::Error::Tun(error)))
            }
            work => work,
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
        // An owner's edge first becomes the one action `decide` gives for
        // the current snapshot; that action is handled below like any work.
        let work = match work {
            Work::Lifecycle(lifecycle::Edge::Reinstall(trigger)) => match trigger {
                "exitRecovered" => Work::ExitRecovered,
                "listenersChanged" => Work::ListenersChanged,
                _ => Work::Offer,
            },
            Work::Lifecycle(edge) => {
                let plane = engine.components();
                let host = healths.host();
                let snapshot = lifecycle::Snapshot {
                    host_ended: !host.is_healthy(),
                    host_due: recovery_at.is_some_and(|at| tokio::time::Instant::now() >= at),
                    dns: healths.dns(plane.dns.acknowledged()),
                    dns_failed,
                    installed: plane.installed.is_some(),
                };
                match lifecycle::decide(&snapshot) {
                    lifecycle::Action::ReproveHost => Work::HostRecovery,
                    lifecycle::Action::WithdrawDns => Work::DnsFailure,
                    lifecycle::Action::ReinstallDns => Work::DnsRecovered,
                    lifecycle::Action::WithdrawHost => Work::HostFailed,
                    lifecycle::Action::Nothing => {
                        // A change of kernel ownership the plane need not act
                        // on (an unavailable end that became a conflict while
                        // withdrawn): its report follows the owner.
                        if edge == lifecycle::Edge::Host && !host.is_healthy() {
                            let mut report = reports.borrow().clone();
                            let detail = plane.host.ownership_ended().map(|end| end.to_string());
                            if report.problem == Some(Problem::Underlay)
                                && report.problem_detail != detail
                            {
                                report.problem_detail = detail;
                                report.apply = retry.health().clone();
                                report.observed_at = chrono::Utc::now();
                                reports.send_replace(report);
                            }
                        }
                        continue;
                    }
                }
            }
            work => work,
        };
        let work = match work {
            Work::HostRecovery => {
                let plane = engine.components_mut();
                match plane.host.recover().await {
                    Ok(()) => {
                        // The DNS owner's anchor and sockets belonged to the
                        // ended term; it prepares again on the new one.
                        if let Err(error) = plane.dns.detach().await {
                            tracing::warn!(
                                ?error,
                                "DNS owner did not stop with the ended ownership term"
                            );
                        }
                        Work::HostRecovered
                    }
                    Err(_) => {
                        let mut report = reports.borrow().clone();
                        report.problem_detail =
                            plane.host.ownership_ended().map(|end| end.to_string());
                        report.observed_at = chrono::Utc::now();
                        report.apply = retry.health().clone();
                        reports.send_replace(report);
                        continue;
                    }
                }
            }
            Work::ExitChanged => {
                let failed = engine
                    .components_mut()
                    .installed
                    .as_mut()
                    .and_then(|state| state.exit.as_mut())
                    .is_some_and(|exit| {
                        exit.health.has_changed().is_err() || {
                            let health = exit.health.borrow_and_update();
                            health.failed || health.ended
                        }
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
            })
            || exit_probe_at.is_some_and(|at| tokio::time::Instant::now() >= at);
        if stale && engine.desired().is_some() {
            exit_probe_at = None;
            let capability = engine.components_mut().exit_capability().await;
            if engine.components().exit_pending()
                && capability
                    .as_ref()
                    .is_some_and(meshway_model::exit::ExitCapability::installable)
            {
                healths.reinstall("exitRecovered");
            }
            let mut report = reports.borrow().clone();
            report.exit_capability = capability;
            reports.send_replace(report);
        }
        let structural = matches!(
            &work,
            Work::Offer
                | Work::Retry
                | Work::DnsRecovered
                | Work::HostRecovered
                | Work::ExitRecovered
                | Work::ListenersChanged
        ) || matches!(&work, Work::Carrier(events) if events.iter().any(|event| matches!(event, Event::Changed)));
        let _turn = SlowTurn::new(match &work {
            Work::Command(_) => "command",
            Work::Expired => "expired",
            Work::Packet(_) => "packet",
            Work::Carrier(_) => "carrier",
            Work::Tick => "tick",
            Work::SampleDue => "sample",
            Work::ExitReply(_) => "exitReply",
            Work::DnsQuery(_) | Work::DnsReport(_) => "dns",
            Work::Offer => "offer",
            Work::Lifecycle(_) => "lifecycle",
            _ => "other",
        });
        match work {
            Work::DnsQuery(query) => dns_queries.enqueue(*query),
            Work::DnsReport(report) => {
                if gateway.try_send(Request::DnsAssociation(report)).is_err() {
                    tracing::debug!("Control queue full; DNS association report dropped");
                }
                continue;
            }
            Work::NetworkChanged => {
                let change = *network_changes.borrow_and_update();
                if change.path != path {
                    path = change.path;
                    // The Manager reacts on its own sockets and channels;
                    // identity, content and leases stay as they are.
                    if let Some(manager) = engine
                        .components_mut()
                        .installed
                        .as_mut()
                        .and_then(|state| state.manager.as_mut())
                    {
                        match manager.path_changed().await {
                            Ok(roamed) => tracing::debug!(
                                operation = "roam",
                                path_epoch = change.path,
                                datagram_channels = roamed.datagram_channels,
                                streams_retired = roamed.streams_retired,
                                observations_withdrawn = roamed.observations_withdrawn,
                                listeners_unbound = roamed.listeners_unbound,
                                "Underlay path changed"
                            ),
                            Err(error) => {
                                tracing::debug!(%error, "Underlay path change not applied to connectivity")
                            }
                        }
                    }
                    // The exit's prerequisites (its outer interface, reverse
                    // path filter) may differ on the new path: the role
                    // reports probing until the probe ran again.
                    if engine.desired().is_some() && reports.borrow().exit_capability.is_some() {
                        let mut report = reports.borrow().clone();
                        if let Some(capability) = &mut report.exit_capability {
                            capability.set_status(meshway_model::exit::ExitStatus::Probing);
                            capability.observed_at = chrono::Utc::now();
                        }
                        reports.send_replace(report);
                        exit_probe_at = Some(tokio::time::Instant::now() + Duration::from_secs(1));
                    }
                }
                let report =
                    sample_host(engine.components(), &diagnostics, reports.borrow().clone());
                reports.send_replace(report);
                continue;
            }
            Work::PoolOverlap => {
                let current = overlaps.borrow_and_update().clone();
                match current {
                    Some(found) => {
                        if overlap.is_none() {
                            tracing::warn!(interface = %found.interface, address = %found.address,
                                "Mesh pool overlap; the packet plane fails closed until the address leaves");
                            diagnostics.invalidate();
                            diagnostics.stop().await;
                            dns_queries.stop().await;
                            if engine.desired().is_some() {
                                withdrawn.get_or_insert(Withdrawn::now(Problem::Underlay));
                            }
                            let _ = engine.reconfig(None).await;
                        }
                        let mut report = reports.borrow().clone();
                        report.plane = DataPlane::Failed;
                        report.apply = retry.health().clone();
                        report.problem = Some(Problem::Underlay);
                        report.problem_detail = Some(super::pool_overlap_detail(&found));
                        report.completion = Some(Err(ApplyFailure::Underlay));
                        report.content = None;
                        report.facts = None;
                        report.exit_flows = None;
                        report.observed_at = chrono::Utc::now();
                        reports.send_replace(report);
                        overlap = Some(found);
                    }
                    None => {
                        if overlap.take().is_some() {
                            tracing::info!("Mesh pool overlap ended; installing the netmap again");
                            retry = apply_recovery();
                            healths.reinstall("offer");
                        }
                    }
                }
                continue;
            }
            Work::Repair => {
                repairs.borrow_and_update();
                let host = &mut engine.components_mut().host;
                if let Err(error) = host.repair().await {
                    if host.repair_at().is_some() {
                        tracing::debug!(%error, "Owned capture objects unreadable; the repair stays owed");
                    } else {
                        tracing::warn!(%error, "Reinstalling owned capture objects failed; kernel ownership ends");
                    }
                }
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
                retry = apply_recovery();
                healths.clear_reinstall();
                withdrawn = None;
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
            | Work::HostRecovered
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
                // This install takes every queued reinstall with it.
                healths.clear_reinstall();
                if !matches!(work, Work::Retry) {
                    retry = apply_recovery();
                }
                // The reinstall below logs once it installs, whichever input
                // led to it (see `Withdrawn::reinstalled`).
                let trigger = match work {
                    Work::Offer => "offer",
                    Work::Retry => "retry",
                    Work::DnsRecovered => "dnsRecovered",
                    Work::HostRecovered => "hostRecovered",
                    Work::ExitRecovered => "exitRecovered",
                    _ => "listenersChanged",
                };
                if matches!(work, Work::DnsRecovered) {
                    dns_failed = false;
                    engine.components_mut().dns.acknowledge();
                }
                if matches!(work, Work::HostRecovered) {
                    tracing::info!("Kernel ownership recovered; reinstalling the netmap");
                    dns_failed = false;
                    // Nothing of the ended term is installed: whatever the
                    // Engine still holds applies again from the start.
                    if engine.desired().is_some() {
                        diagnostics.invalidate();
                        diagnostics.stop().await;
                        withdraw(&mut engine, &reports, true).await;
                        withdrawn.get_or_insert(Withdrawn::now(Problem::Underlay));
                    }
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
                        withdrawn.get_or_insert(Withdrawn::of(Cause::Withdrawn));
                    }
                    continue;
                };
                // A new ownership term publishes the overlap it read before
                // the worker sees that change.
                if overlap.is_some() || overlaps.borrow().is_some() {
                    // Kept offered; installed once the overlapping address leaves.
                    continue;
                }
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
                            if engine.components().exit_pending()
                                && report
                                    .exit_capability
                                    .as_ref()
                                    .is_some_and(meshway_model::exit::ExitCapability::installable)
                            {
                                healths.reinstall("exitRecovered");
                            }
                        }
                        reports.send_replace(report);
                    }
                    // A first instance rebinds Native UDP listeners as
                    // coordination sockets; one may now be unavailable.
                    if rebound && engine.components().listeners_unavailable() != listener_gap {
                        healths.reinstall("listenersChanged");
                    }
                    continue;
                }
                let previous = reports.borrow().clone();
                let mut report = previous.clone();
                // Changed associations keep the installed content's work: its
                // report stays until the new desired state's outcome below.
                // The last outcome's problem stays until this one's outcome:
                // a plane withdrawn for a failure keeps its cause while it
                // prepares again.
                if change == Some(super::transitions::Change::Content) {
                    report.plane = DataPlane::Preparing;
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
                report.dns_guard = plane.dns.report_guard();
                report.exit_dns = exit_dns.borrow_and_update().clone();
                report.observed_at = chrono::Utc::now();
                let installed = matches!(result, Ok(()) | Err(meshway_engine::Error::Degraded));
                if installed && let Some(withdrawal) = withdrawn.take() {
                    withdrawal.reinstalled(trigger, reference.serial.get(), result.is_ok());
                }
                match result {
                    Ok(()) => {
                        dns_failed = false;
                        // A complete apply ends an earlier degraded one's
                        // problem (a listener that bound on retry).
                        report.problem = None;
                        report.problem_detail = None;
                        retry = apply_recovery();
                        report.apply = meshway_model::recovery::Health::Healthy;
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
                        apply_failed(&mut retry, failure);
                        report.apply = retry.health().clone();
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
                        if previous.content.is_some() {
                            withdrawn.get_or_insert(Withdrawn::now(problem));
                        }
                        report.completion = Some(Err(failure));
                        report.problem = Some(problem);
                        report.problem_detail = plane.failure_detail.take();
                        report.plane = DataPlane::Failed;
                        apply_failed(&mut retry, failure);
                        report.apply = retry.health().clone();
                    }
                }
                reports.send_replace(report);
            }
            Work::Expired => {
                // An ended association only narrows the desired state; the
                // lease's diagnostics stop with the lease alone.
                let (ended, failed) =
                    super::transitions::expire(&mut engine, async || diagnostics.stop().await)
                        .await;
                if failed {
                    tracing::warn!("Netmap expiry cleanup failed");
                }
                if ended {
                    tracing::info!("Netmap lease ended; terminating the packet plane");
                    withdrawn.get_or_insert(Withdrawn::of(Cause::LeaseEnded));
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
                withdrawing(
                    &reports,
                    Problem::Underlay,
                    engine
                        .components()
                        .host
                        .ownership_ended()
                        .map(|end| end.to_string()),
                    retry.health().clone(),
                );
                withdrawn.get_or_insert(Withdrawn::now(Problem::Underlay));
                diagnostics.stop().await;
                withdraw(&mut engine, &reports, true).await;
                continue;
            }
            Work::ExitFailed | Work::ExitReply(Err(_)) => {
                // The exit role is a capability this Node provides to other
                // sources, not a component its own traffic requires: its
                // failure ends that role alone. Its flows retire with their
                // projections; the applied content, capture, DNS and
                // adjacencies stay. The capability probe reinstalls the role
                // on the same content once its prerequisites hold.
                let plane = engine.components_mut();
                let cleanup = plane.withdraw_exit().await;
                tracing::warn!(
                    cleanup_failed = cleanup.is_err(),
                    "Exit role failed; withdrawing it alone"
                );
                let capability = plane.exit_capability().await;
                if engine.components().exit_pending()
                    && capability
                        .as_ref()
                        .is_some_and(meshway_model::exit::ExitCapability::installable)
                {
                    healths.reinstall("exitRecovered");
                }
                let mut report = reports.borrow().clone();
                report.exit_capability = capability;
                report.exit_flows = None;
                reports.send_replace(report);
                continue;
            }
            Work::CarrierClosed | Work::Packet(Err(_)) | Work::DnsFailure => {
                let dns_error = matches!(work, Work::DnsFailure);
                // Kernel ownership that ended first is the cause (the DNS
                // anchor ends with it); otherwise the failed component.
                let ended = engine
                    .components()
                    .host
                    .ownership_ended()
                    .map(|end| end.to_string());
                let problem = if ended.is_some() {
                    Problem::Underlay
                } else {
                    match work {
                        Work::DnsFailure => Problem::Dns,
                        Work::Packet(_) => Problem::Tun,
                        _ => Problem::Transport,
                    }
                };
                if dns_error {
                    tracing::warn!("Required DNS component failed; suspending packet plane");
                } else {
                    tracing::warn!(
                        ?problem,
                        "Required packet plane component failed; suspending packet plane"
                    );
                }
                dns_failed |= dns_error;
                if dns_error {
                    engine.components_mut().dns.acknowledge();
                }
                // The DNS check that failed, else why kernel ownership ended:
                // never an empty detail over a known reason.
                let detail = if problem == Problem::Dns {
                    engine.components().dns.detail().or(ended)
                } else {
                    ended
                };
                apply_failed(&mut retry, withdrawal::failure(problem));
                let mut report = withdrawing(&reports, problem, detail, retry.health().clone());
                withdrawn.get_or_insert(Withdrawn::now(problem));
                // The decisions of every external ping go with the content.
                diagnostics.withdraw().await;
                let cleanup = engine.reconfig(None).await;
                // Kernel ownership that ended during the cleanup is the
                // Host's failure now (its re-proof recovers it). A cleanup
                // that failed otherwise (an exit retirement its owner
                // retries) keeps the earliest cause.
                if cleanup.is_err()
                    && problem != Problem::Underlay
                    && let Some(end) = engine.components().host.ownership_ended()
                {
                    report.problem = Some(Problem::Underlay);
                    report.problem_detail = Some(end.to_string());
                    report.completion = Some(Err(ApplyFailure::Underlay));
                    report.apply = retry.health().clone();
                    report.observed_at = chrono::Utc::now();
                    reports.send_replace(report.clone());
                }
                continue;
            }
            Work::Packet(Ok(received)) => {
                let packets = &mut engine.components_mut().packets;
                for packet in received {
                    #[cfg(target_os = "macos")]
                    if let Ok(packet) = &packet
                        && let Some(port) = &packet_dns
                        && port.accepts(packet.as_bytes())
                    {
                        port.submit(packet.as_bytes().to_vec());
                        continue;
                    }
                    match packet {
                        Ok(packet) if can_read => packets.enqueue(packet),
                        Ok(_) => packets.drop_packet("packet_plane_withdrawn"),
                        Err(meshway_dataplane::PacketError::Unsupported) => {
                            packets.drop_packet("unsupported_ip_or_local_control")
                        }
                        Err(_) => packets.drop_packet("malformed_ip"),
                    }
                }
            }
            Work::ExitReply(Ok(replies)) => {
                let Plane {
                    installed, packets, ..
                } = engine.components_mut();
                if let Some(state) = installed
                    && let Some(secure) = &mut state.secure
                {
                    for reply in replies {
                        match Packet::parse(reply) {
                            Ok(packet) => match secure
                                .exit_reply(meshway_dataplane::exit::ExitReplyIngress::new(packet))
                            {
                                Ok(frame) => packets.exit_reply(frame),
                                Err(_) => packets.drop_packet("exit_reply_admission"),
                            },
                            Err(_) => packets.drop_packet("exit_reply_malformed"),
                        }
                    }
                    if let Some(exit) = &mut state.exit {
                        exit.drive(secure);
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
            #[cfg(target_os = "macos")]
            Work::DnsPacket(output) => {
                dns_output = output;
                continue;
            }
            #[cfg(target_os = "macos")]
            Work::DnsPacketSent(_) => {
                dns_output = None;
                continue;
            }
            Work::DiagnosticSent => {}
            Work::ExitChanged | Work::HostRecovery | Work::Lifecycle(_) => {
                unreachable!("mapped above")
            }
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
            // pass a Close on to the surviving side of each relayed one. A
            // new Manager of this Node (its adjacencies restarted) is a new
            // boot to every peer: end this Node's own circuits the same way.
            packets.closes(secure.set_own_boot(manager.boot()));
            if let Ok(boots) = manager.peer_boots() {
                packets.closes(secure.set_peer_boots(boots));
            }
            {
                let known = peers.borrow();
                let now = chrono::Utc::now();
                for family in [
                    meshway_model::exit::ExitFamily::Ipv4,
                    meshway_model::exit::ExitFamily::Ipv6,
                ] {
                    let exits = known
                        .as_ref()
                        .map(|known| known.ready_exits(now, family))
                        .unwrap_or_default();
                    secure.set_ready_exits(family, exits.current, exits.last_known);
                }
                if let Some(current) = channels.changed(known.as_ref(), now) {
                    secure.set_peer_channels(current);
                }
            }
            diagnostics.synchronize(&state.desired, instance);
            tokio::select! { biased;
                // An owner's health changed: the next turn takes it.
                _ = healths.changed() => {},
                _ = async {
                    static PROFILE: meshway_logging::profile::Metric = meshway_logging::profile::Metric::new("host.drive.poll");
                    PROFILE.poll(packets.drive(secure, manager, carriers.as_ref(), host.as_ref(), &state.desired, &mut diagnostics, &mut dns_queries, dns.dispatcher(), &mut pool, state.exit.as_mut())).await;
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
            healths.reinstall("listenersChanged");
        }
        if structural {
            let report = sample_host(engine.components(), &diagnostics, reports.borrow().clone());
            roams.observe(&report);
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

/// A worker turn that took long is logged with its kind when it ends
/// (diagnostics: the packet plane waits for every turn).
struct SlowTurn {
    kind: &'static str,
    started: std::time::Instant,
}

impl SlowTurn {
    fn new(kind: &'static str) -> Self {
        Self {
            kind,
            started: std::time::Instant::now(),
        }
    }
}

impl Drop for SlowTurn {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed();
        if elapsed >= Duration::from_millis(100) {
            tracing::debug!(
                kind = self.kind,
                turn_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                "Network worker turn was slow"
            );
        }
    }
}
