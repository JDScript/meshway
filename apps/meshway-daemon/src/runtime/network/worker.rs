//! The unique network owner. It installs each netmap snapshot Control offers:
//! the same content renews in place, a larger serial updates every resource
//! in place where its exact purpose is unchanged, anything else installs
//! afresh. An ended lease terminates every flow, circuit and exit projection
//! and keeps only the negative capture and DNS boundaries.
use super::{
    Command, ObservationPlan, Peers, Report, Request, dns::Attachment, packets::Packets,
    reporting::Publisher,
};
use meshway_authority::DataPlaneIdentity;
use meshway_configuration::netmap::DnsPlan;
use meshway_connectivity::{AdjacencyInterest, Event, Manager};
use meshway_dataplane::{
    DEFAULT_HEADROOM, DEFAULT_TAILROOM, Packet, PacketPool, secure::SecureNode,
};
use meshway_ipc::{DataPlane, Problem};
use meshway_protocol::execution::ApplyFailure;
use meshway_underlay::Host;
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};

struct Installed {
    identity: DataPlaneIdentity,
    /// Absent for a disabled Node, which owns no packet plane.
    secure: Option<SecureNode>,
    manager: Option<Manager>,
    exit: Option<super::exit::Runtime>,
    live: bool,
}

pub(super) struct Inputs {
    pub host: Host,
    pub commands: mpsc::Receiver<Command>,
    pub targets: watch::Receiver<Option<DataPlaneIdentity>>,
    pub reports: Publisher,
    pub gateway: mpsc::Sender<Request>,
    pub dns: Attachment,
    pub traversal: Arc<dyn meshway_connectivity::traversal::TraversalGateway>,
    pub observation_plans: watch::Receiver<Option<ObservationPlan>>,
    pub peers: Peers,
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
        let transient = !matches!(
            failure,
            ApplyFailure::Dns | ApplyFailure::Validation | ApplyFailure::Dataplane
        );
        self.at = (transient && self.failures < 3).then(|| {
            tokio::time::Instant::now()
                + Duration::from_secs(if self.failures == 1 { 5 } else { 15 })
        });
    }
}

pub(super) async fn run(inputs: Inputs) -> Result<(), Problem> {
    let Inputs {
        mut host,
        mut commands,
        mut targets,
        reports,
        gateway,
        mut dns,
        traversal,
        mut observation_plans,
        peers,
        mtu,
    } = inputs;
    let mut network_changes = host.network_changes();
    let mut installed: Option<Installed> = None;
    let mut packets = Packets::default();
    let workers = std::env::var("MESHWAY_DATAPLANE_WORKERS")
        .ok()
        .map(|value| value.parse::<usize>().map_err(|_| Problem::Worker))
        .transpose()?
        .unwrap_or_else(meshway_dataplane::secure::pipeline::Pool::default_workers);
    let mut pool =
        meshway_dataplane::secure::pipeline::Pool::new(workers).map_err(|_| Problem::Worker)?;
    let mut diagnostics = super::diagnostics::Diagnostics::default();
    let buffers = packet_pool(mtu)?;
    let mut dns_requests = dns.take_queries();
    let mut dns_queries = super::dns::mesh::Queries::default();
    let mut generation = meshway_model::Sequence::MIN;
    let mut retry = Retry::default();
    let mut dns_failed = false;
    let mut sample_at = None;
    // The lease end of an invalidated snapshot awaiting Control's next content.
    let mut expires_at: Option<tokio::time::Instant> = None;
    loop {
        meshway_logging::profile::report();
        enum Work {
            DnsQuery(Box<super::dns::mesh::Query>),
            ObservationPlan,
            DiagnosticSent,
            Command(Option<Command>),
            Target,
            Retry,
            Invalid,
            Expired,
            DnsFailure,
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
            ExitReply(Result<meshway_underlay::ExitReply, meshway_underlay::Error>),
        }
        let live = installed.as_ref().is_some_and(|state| state.live);
        let can_read = live && packets.can_read(&pool);
        let has_outputs = packets.has_outputs();
        let guard = installed
            .as_ref()
            .filter(|state| state.live)
            .map(|state| state.identity.snapshot().clone());
        let mut wake_at = installed
            .as_ref()
            .filter(|state| state.live)
            .and_then(|_| packets.wake_at());
        if let Some(at) = diagnostics.wake_at() {
            wake_at = Some(wake_at.map_or(at, |old| old.min(at)));
        }
        if let Some(at) = dns_queries.wake_at() {
            wake_at = Some(wake_at.map_or(at, |old| old.min(at)));
        }
        if installed.as_ref().is_some_and(|state| state.exit.is_some()) {
            let at = tokio::time::Instant::now() + Duration::from_millis(250);
            wake_at = Some(wake_at.map_or(at, |old| old.min(at)));
        }
        if live {
            // Ready-exit facts expire; refresh the selection input regularly.
            let at = tokio::time::Instant::now() + Duration::from_secs(1);
            wake_at = Some(wake_at.map_or(at, |old| old.min(at)));
        }
        let retry_at = retry.at.filter(|_| installed.is_none());
        let work = tokio::select! { biased;
            command = commands.recv() => Work::Command(command),
            _ = diagnostics.sent(), if diagnostics.has_sends() => Work::DiagnosticSent,
            changed = observation_plans.changed() => if changed.is_ok() { Work::ObservationPlan } else { Work::Command(None) },
            changed = targets.changed() => { if changed.is_err() { Work::Command(None) } else { Work::Target } },
            query = dns_requests.recv() => match query { Some(query) => Work::DnsQuery(Box::new(query)), None => Work::Command(None) },
            _ = async { match retry_at { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => Work::Retry,
            _ = dns.failed(), if !dns_failed => Work::DnsFailure,
            _ = async { match expires_at { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => Work::Expired,
            _ = async {
                if let Some(guard) = &guard {
                    tokio::select! {
                        _ = guard.invalidated() => {},
                        _ = host.invalidated() => {},
                    }
                } else { std::future::pending::<()>().await; }
            } => Work::Invalid,
            changed = network_changes.changed() => if changed.is_ok() { Work::NetworkChanged } else { Work::Command(None) },
            _ = async { match sample_at { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => Work::SampleDue,
            work = async {
                let (manager, mut exit) = match installed.as_mut() {
                    Some(state) => (state.manager.as_mut(), state.exit.as_mut()),
                    None => (None, None),
                };
                let mut health = exit.as_ref().map(|exit| exit.health.clone());
                // Packet pressure must not starve incoming handshakes or lease
                // work. Lifecycle/authority remain the outer priority.
                tokio::select! {
                    _ = async { match &mut health { Some(health) => { let _ = health.changed().await; }, None => std::future::pending().await } } => Work::ExitChanged,
                    reply = async { match exit.as_mut() { Some(exit) => exit.reader.recv().await, None => std::future::pending().await } }, if live => Work::ExitReply(reply),
                    _ = packets.sent(&host), if has_outputs => Work::Tick,
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
                    _ = async { match wake_at { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => Work::Tick,
                    else => std::future::pending().await,
                }
            } => work,
        };
        let work = match work {
            Work::ExitChanged => {
                let failed = installed
                    .as_mut()
                    .and_then(|state| state.exit.as_mut())
                    .is_some_and(|exit| {
                        exit.health.has_changed().is_err() || exit.health.borrow_and_update().failed
                    });
                if failed { Work::ExitFailed } else { Work::Tick }
            }
            work => work,
        };
        let structural = matches!(&work, Work::Target | Work::Retry)
            || matches!(&work, Work::Carrier(events) if events.iter().any(|event| matches!(event, Event::Changed)));
        match work {
            Work::DnsQuery(query) => dns_queries.enqueue(*query),
            Work::NetworkChanged => {
                network_changes.borrow_and_update();
                let report = sample_host(&host, &installed, &diagnostics, reports.borrow().clone());
                reports.send_replace(report);
                continue;
            }
            Work::SampleDue => {
                let report = sample_host(&host, &installed, &diagnostics, reports.borrow().clone());
                reports.send_replace(report);
                sample_at = None;
                continue;
            }
            Work::ObservationPlan => {
                diagnostics.plan(observation_plans.borrow_and_update().clone());
            }
            Work::Command(Some(Command::Ping(request))) => {
                let snapshot = installed
                    .as_ref()
                    .filter(|state| state.live && state.secure.is_some())
                    .map(|state| state.identity.snapshot().clone());
                diagnostics.ping(snapshot.as_ref(), request);
            }
            Work::Command(Some(Command::Builder(reply))) => {
                let _ = reply.send(host.client_builder().map_err(|_| Problem::Underlay));
                continue;
            }
            Work::Command(Some(Command::Sample(reply))) => {
                let report = sample_host(&host, &installed, &diagnostics, reports.borrow().clone());
                let _ = reply.send(reports.sample(report));
                sample_at = None;
                continue;
            }
            Work::Command(Some(Command::Shutdown { keep_sink })) => {
                dns_queries.stop().await;
                diagnostics.stop().await;
                let dns_result = dns
                    .shutdown(&mut host, keep_sink)
                    .await
                    .map_err(|_| Problem::Dns);
                let closing = close(&mut installed, &peers).await;
                let host_result = host
                    .shutdown(keep_sink)
                    .await
                    .map_err(|_| Problem::Underlay);
                return dns_result.and(closing).and(host_result);
            }
            Work::Command(None) => {
                dns_queries.stop().await;
                diagnostics.stop().await;
                let dns_result = dns
                    .shutdown(&mut host, true)
                    .await
                    .map_err(|_| Problem::Dns);
                let closing = close(&mut installed, &peers).await;
                return dns_result
                    .and(closing)
                    .and(host.shutdown(true).await.map_err(|_| Problem::Underlay));
            }
            Work::Command(Some(Command::Stop { down, reply })) => {
                dns_queries.stop().await;
                diagnostics.stop().await;
                dns.suspend();
                let closing = close(&mut installed, &peers).await;
                installed = None;
                expires_at = None;
                retry = Retry::default();
                packets = Packets::default();
                let dns_result = if down {
                    dns.down(&mut host).await.map_err(|_| Problem::Dns)
                } else {
                    Ok(())
                };
                let host_result = if down && dns_result.is_ok() {
                    dns_failed = false;
                    host.down().await
                } else {
                    host.suspend().await
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
            Work::Target | Work::Retry => {
                if matches!(work, Work::Target) {
                    retry = Retry::default();
                }
                let Some(identity) = targets.borrow_and_update().clone() else {
                    // Control holds no authority: end the installed netmap.
                    if installed.is_some() {
                        diagnostics.invalidate();
                        expire(
                            &mut host,
                            &mut installed,
                            &mut packets,
                            &dns,
                            &peers,
                            &reports,
                            false,
                        )
                        .await;
                    }
                    expires_at = None;
                    continue;
                };
                if identity.snapshot().ensure_current().is_err() {
                    continue;
                }
                expires_at = None;
                // Same content: a lease renewal or a new instance binding.
                if let Some(state) = installed.as_mut()
                    && state.identity.snapshot().same_netmap(identity.snapshot())
                {
                    if renew(&mut host, &dns, state, identity.clone())
                        .await
                        .is_err()
                    {
                        tracing::warn!("Same-content netmap renewal failed; reinstalling");
                        let _ = close(&mut installed, &peers).await;
                        installed = None;
                    } else {
                        continue;
                    }
                }
                let reference = identity.snapshot().reference();
                let Ok(next_generation) = generation.checked_next() else {
                    dns_queries.stop().await;
                    diagnostics.stop().await;
                    let _ = dns.shutdown(&mut host, true).await;
                    let _ = close(&mut installed, &peers).await;
                    let _ = host.shutdown(true).await;
                    return Err(Problem::Worker);
                };
                generation = next_generation;
                let continuing = installed.as_ref().is_some_and(|state| {
                    identity
                        .snapshot()
                        .netmap_continuation_from(state.identity.snapshot())
                        .is_ok()
                });
                let mut report = if continuing {
                    reports.borrow().clone()
                } else {
                    diagnostics.invalidate();
                    Report::stopped()
                };
                report.plane = DataPlane::Preparing;
                report.problem = None;
                report.netmap = Some(reference);
                report.completion = None;
                report.facts = None;
                report.guard = Some(identity.snapshot().clone());
                report.generation = generation;
                reports.send_replace(report.clone());
                let result = install(
                    &mut host,
                    &mut installed,
                    &mut packets,
                    identity,
                    continuing,
                    &mut dns,
                    traversal.clone(),
                    generation,
                    &peers,
                    &mut report,
                )
                .await;
                report.dns_guard = dns.guard();
                report.observed_at = chrono::Utc::now();
                report.completion = Some(result.map_err(|(_, failure)| failure));
                match result {
                    Ok(()) => {
                        dns_failed = false;
                        report.plane = if installed.as_ref().is_some_and(|s| s.secure.is_some()) {
                            DataPlane::Ready
                        } else {
                            DataPlane::Stopped
                        };
                    }
                    Err((problem, failure)) => {
                        tracing::warn!(
                            ?problem,
                            ?failure,
                            netmap_serial = reference.serial.get(),
                            "Node netmap installation did not complete"
                        );
                        dns.suspend();
                        diagnostics.invalidate();
                        let closing = close(&mut installed, &peers).await;
                        installed = None;
                        packets = Packets::default();
                        let suspended = host.suspend().await;
                        report.problem = Some(if closing.is_err() || suspended.is_err() {
                            Problem::Underlay
                        } else {
                            problem
                        });
                        report.plane = DataPlane::Failed;
                        retry.failed(failure);
                    }
                }
                reports.send_replace(report);
            }
            Work::Invalid => {
                // New content is on its way (stop, down and revocation also
                // arrive as commands or a cleared target): keep everything
                // inert until it, or the lease end.
                let lease_end = installed
                    .as_ref()
                    .map(|state| state.identity.snapshot().expires_at());
                let expired = lease_end.is_none_or(|at| std::time::Instant::now() >= at);
                if let Some(state) = &mut installed {
                    state.live = false;
                }
                diagnostics.invalidate();
                packets.clear_incoming();
                if expired || host_invalidated(&host).await {
                    expire(
                        &mut host,
                        &mut installed,
                        &mut packets,
                        &dns,
                        &peers,
                        &reports,
                        !expired,
                    )
                    .await;
                    expires_at = None;
                } else {
                    expires_at = lease_end.map(Into::into);
                    let mut report = reports.borrow().clone();
                    report.plane = DataPlane::Suspended;
                    reports.send_replace(report);
                }
                continue;
            }
            Work::Expired => {
                expire(
                    &mut host,
                    &mut installed,
                    &mut packets,
                    &dns,
                    &peers,
                    &reports,
                    false,
                )
                .await;
                expires_at = None;
                continue;
            }
            Work::ExitFailed
            | Work::ExitReply(Err(_))
            | Work::CarrierClosed
            | Work::Packet(Err(_))
            | Work::DnsFailure => {
                diagnostics.invalidate();
                dns.suspend();
                let dns_error = matches!(work, Work::DnsFailure);
                if dns_error {
                    tracing::warn!("Required DNS component failed; suspending packet plane");
                }
                dns_failed |= dns_error;
                let closing = close(&mut installed, &peers).await;
                installed = None;
                packets = Packets::default();
                let cleanup = host.suspend().await;
                let mut report = reports.borrow().clone();
                report.plane = DataPlane::Failed;
                report.problem = Some(if closing.is_err() || cleanup.is_err() {
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
                if let Some(state) = &mut installed
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
                for event in events {
                    let Event::Frame {
                        link,
                        frame,
                        received_at,
                    } = event
                    else {
                        continue;
                    };
                    if let Some(state) = &installed
                        && let Some(secure) = &state.secure
                    {
                        packets.incoming(
                            *link,
                            frame.payload.into(),
                            received_at,
                            state.identity.snapshot(),
                            secure,
                        );
                    }
                }
            }
            Work::Tick | Work::DiagnosticSent => {}
            Work::ExitChanged => unreachable!("mapped above"),
        }
        if let Some(state) = &mut installed
            && state.live
            && let (Some(secure), Some(manager)) = (&mut state.secure, &state.manager)
        {
            // One scheduling view per turn. It grants no I/O authority: every
            // output still captures and checks the Manager's actual channel,
            // while SecureNode checks its own packet/circuit admission.
            let carriers = manager.carriers().ok();
            if let Some(carriers) = &carriers {
                let _ = secure.set_carriers(carriers.clone());
            }
            secure.set_ready_exits(peers.ready_exits());
            let snapshot = state.identity.snapshot().clone();
            diagnostics.synchronize(&snapshot);
            tokio::select! { biased;
                _ = host.invalidated() => {},
                _ = snapshot.invalidated() => {},
                _ = dns.failed(), if !dns_failed => {},
                _ = async {
                    static PROFILE: meshway_logging::profile::Metric = meshway_logging::profile::Metric::new("host.drive.poll");
                    PROFILE.poll(packets.drive(secure, manager, carriers.as_ref(), &host, &snapshot, &mut diagnostics, &mut dns_queries, dns.dispatcher(), &mut pool, state.exit.as_mut())).await;
                    dns_queries.drive(secure, &snapshot, manager).await;
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
        if structural {
            let report = sample_host(&host, &installed, &diagnostics, reports.borrow().clone());
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

/// Whether the Host itself (not merely the snapshot) lost authority.
async fn host_invalidated(host: &Host) -> bool {
    tokio::time::timeout(Duration::ZERO, host.invalidated())
        .await
        .is_ok()
}

/// The lease ended (or the Host lost authority): terminate every flow,
/// circuit and exit projection; keep the negative capture and DNS boundary.
async fn expire(
    host: &mut Host,
    installed: &mut Option<Installed>,
    packets: &mut Packets,
    dns: &Attachment,
    peers: &Peers,
    reports: &Publisher,
    host_failed: bool,
) {
    tracing::info!(
        host_failed,
        "Netmap authority ended; terminating the packet plane"
    );
    dns.suspend();
    let closing = close(installed, peers).await;
    *installed = None;
    *packets = Packets::default();
    let cleanup = host.suspend().await;
    let mut report = reports.borrow().clone();
    report.plane = if closing.is_ok() && cleanup.is_ok() && !host_failed {
        DataPlane::Suspended
    } else {
        DataPlane::Failed
    };
    report.problem = (report.plane == DataPlane::Failed).then_some(Problem::Underlay);
    report.completion = None;
    report.facts = None;
    report.exit_flows = None;
    if let Some(capability) = &mut report.exit_capability {
        capability.installed_families.clear();
    }
    reports.send_replace(report);
}

/// Same netmap content: every owner swaps in the snapshot carrying the
/// renewed lease or the new instance binding. Nothing is reinstalled.
async fn renew(
    host: &mut Host,
    dns: &Attachment,
    state: &mut Installed,
    identity: DataPlaneIdentity,
) -> Result<(), Problem> {
    host.install(identity.snapshot().clone())
        .await
        .map_err(|_| Problem::Underlay)?;
    if let Some(secure) = &mut state.secure {
        secure
            .update(identity.clone())
            .map_err(|_| Problem::Apply)?;
    }
    if let Some(manager) = &mut state.manager {
        manager
            .reconcile(identity.clone())
            .await
            .map_err(|_| Problem::Transport)?;
    }
    if dns.guard().is_some() {
        // An equivalent scope keeps the query owner; failures surface via guard.
        dns.install(identity.snapshot().clone())
            .await
            .map_err(|_| Problem::Dns)?;
    }
    state.identity = identity;
    state.live = true;
    Ok(())
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

async fn close(installed: &mut Option<Installed>, peers: &Peers) -> Result<(), Problem> {
    let mut failed = false;
    if let Some(state) = installed {
        state.live = false;
        if let Some(exit) = state.exit.take() {
            let result = match &mut state.secure {
                Some(secure) => exit.stop(secure).await,
                None => Err(Problem::Underlay),
            };
            if let Err(problem) = result {
                tracing::warn!(?problem, "Exit retirement failed");
                failed = true;
            }
        }
        if let Some(mut manager) = state.manager.take() {
            peers.detach();
            let _ = manager.shutdown().await;
        }
        state.secure = None;
    }
    if failed {
        Err(Problem::Underlay)
    } else {
        Ok(())
    }
}

/// An explicit measurement by the resource owner. Application observation time
/// remains immutable; sampling current sockets is distinct from receipt replay.
fn sample_host(
    host: &Host,
    installed: &Option<Installed>,
    diagnostics: &super::diagnostics::Diagnostics,
    mut report: Report,
) -> Report {
    report.interface_epoch = *host.network_changes().borrow();
    report.owned_interfaces = host.owned_interfaces();
    sample(installed, diagnostics, report)
}

fn sample(
    installed: &Option<Installed>,
    diagnostics: &super::diagnostics::Diagnostics,
    mut report: Report,
) -> Report {
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("host.sample");
    let _profile = PROFILE.enter();
    let secure = installed.as_ref().and_then(|state| state.secure.as_ref());
    meshway_logging::profile::gauge(
        "secure.retained_circuits",
        secure.map(|secure| secure.retained_circuits() as u64),
    );
    meshway_logging::profile::gauge(
        "flow.retained",
        secure.map(|secure| secure.retained_flows() as u64),
    );
    meshway_logging::profile::gauge("secure.ready_circuits", None);
    report.facts = None;
    report.exit_flows = installed
        .as_ref()
        .filter(|s| s.live)
        .and_then(|s| s.exit.as_ref())
        .map(|exit| exit.health.borrow().flows);
    report.dependencies.clear();
    report.observation_dependencies.clear();
    report.facts_observed_at = chrono::Utc::now();
    if let Some(state) = installed
        && state.live
        && let (Some(secure), Some(manager)) = (&state.secure, &state.manager)
    {
        report.facts = manager.snapshot().ok();
        if let Ok(circuits) = secure.ready_circuits()
            && let Ok(netmap) = state.identity.snapshot().netmap()
        {
            let node = netmap.netmap.node_id;
            meshway_logging::profile::gauge("secure.ready_circuits", Some(circuits.len() as u64));
            for circuit in circuits {
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
        }
    }
    report
}

#[allow(clippy::too_many_arguments)]
async fn install(
    host: &mut Host,
    installed: &mut Option<Installed>,
    packets: &mut Packets,
    identity: DataPlaneIdentity,
    continuing: bool,
    dns: &mut Attachment,
    traversal: Arc<dyn meshway_connectivity::traversal::TraversalGateway>,
    generation: meshway_model::Sequence,
    peers: &Peers,
    report: &mut Report,
) -> Result<(), (Problem, ApplyFailure)> {
    let snapshot = identity.snapshot().clone();
    let netmap = snapshot
        .netmap()
        .map_err(|_| (Problem::Apply, ApplyFailure::Validation))?
        .netmap
        .clone();
    let reference = snapshot.reference();
    if !continuing {
        close(installed, peers)
            .await
            .map_err(|p| (p, ApplyFailure::Underlay))?;
        *installed = None;
        *packets = Packets::default();
    }
    if let Some(state) = installed.as_mut() {
        state.live = false;
    }
    packets.clear_incoming();
    // Capture, TUN, retained descriptors and the exit role's identity are the
    // Host's own in-place difference between the two contents.
    host.install(snapshot.clone()).await.map_err(|error| {
        tracing::warn!(%error, "Node capture installation failed");
        match error {
            meshway_underlay::Error::Tun(_) => (Problem::Tun, ApplyFailure::Tun),
            _ => (Problem::Underlay, ApplyFailure::Underlay),
        }
    })?;
    let settings = &netmap.projection.configuration.dns;
    let exit_dns = !matches!(netmap.dns.egress, DnsPlan::NoExit);
    let needs_dns = super::requirements::dns_required(settings) || exit_dns;
    report.exit_dns_unavailable = false;
    // Required negative DNS first; query authority follows the installed
    // SecureNode and the authenticated DNS-purpose exchange adapter below.
    if needs_dns {
        match dns.plan(host, &netmap).await {
            Ok(()) => {}
            Err(error) => {
                tracing::warn!(%error, "Required DNS installation failed");
                return Err((Problem::Dns, ApplyFailure::Dns));
            }
        }
    } else {
        dns.down(host)
            .await
            .map_err(|_| (Problem::Dns, ApplyFailure::Dns))?;
    }
    if !netmap.self_.enabled {
        // A disabled Node owns no packet plane; the Host keeps its negative boundary.
        close(installed, peers)
            .await
            .map_err(|p| (p, ApplyFailure::Underlay))?;
        report.exit_capability = None;
        *installed = Some(Installed {
            identity,
            secure: None,
            manager: None,
            exit: None,
            live: false,
        });
        return Ok(());
    }
    // Exit prerequisites, observed on behalf of the live instance only.
    report.exit_capability = match (&netmap.exit_role, snapshot.instance()) {
        (Some(role), Some(instance)) => Some(
            host.probe_exit(instance.instance_id, reference, role.generation)
                .await,
        ),
        _ => None,
    };
    let mut new_role = None;
    if let Some(role) = &netmap.exit_role {
        let unchanged = installed
            .as_ref()
            .and_then(|state| state.exit.as_ref())
            .is_some_and(|exit| exit.plan == *role);
        if !unchanged {
            if report
                .exit_capability
                .as_ref()
                .is_some_and(|c| c.status != meshway_model::exit::ExitStatus::Ready)
            {
                return Err((Problem::Underlay, ApplyFailure::Underlay));
            }
            if let Some(state) = installed.as_mut()
                && let Some(old) = state.exit.take()
                && let Some(secure) = &mut state.secure
            {
                old.stop(secure)
                    .await
                    .map_err(|p| (p, ApplyFailure::Underlay))?;
            }
            let installed_role = host.install_netmap_exit_role(role).await.map_err(|error| {
                tracing::warn!(%error, "Exit role installation failed");
                if let Some(capability) = &mut report.exit_capability {
                    capability.status = meshway_model::exit::ExitStatus::Unavailable {
                        reason: meshway_model::exit::ExitUnavailableReason::OwnershipConflict,
                    };
                }
                (Problem::Underlay, ApplyFailure::Underlay)
            })?;
            new_role = Some((installed_role, role.clone()));
        }
    } else if let Some(state) = installed.as_mut()
        && let Some(old) = state.exit.take()
        && let Some(secure) = &mut state.secure
    {
        old.stop(secure)
            .await
            .map_err(|p| (p, ApplyFailure::Underlay))?;
    }
    if host.tun().is_none() {
        return Err((Problem::Tun, ApplyFailure::Tun));
    }
    // Adjacency: unchanged resources and channels survive a continuing serial.
    let previous = installed.as_mut().and_then(|state| state.manager.take());
    let manager = match previous {
        Some(mut manager) if manager.can_reconcile(&identity) => {
            if manager.reconcile(identity.clone()).await.is_err() {
                let _ = manager.shutdown().await;
                peers.detach();
                return Err((Problem::Transport, ApplyFailure::Listeners));
            }
            manager
        }
        previous => {
            if let Some(mut manager) = previous {
                peers.detach();
                let _ = manager.shutdown().await;
            }
            Manager::start_with_traversal(
                identity.clone(),
                Arc::new(host.sockets()),
                Default::default(),
                traversal,
                generation,
            )
            .await
            .map_err(|_| (Problem::Transport, ApplyFailure::Listeners))?
        }
    };
    peers.attach(manager.facts_sink(), reference);
    let carriers = match manager.carriers() {
        Ok(carriers) => carriers,
        Err(_) => {
            let mut manager = manager;
            let _ = manager.shutdown().await;
            peers.detach();
            return Err((Problem::Transport, ApplyFailure::Listeners));
        }
    };
    // Flows and circuits: an in-place content comparison when continuing.
    let previous = installed.as_mut().and_then(|state| state.secure.take());
    let secure = match previous {
        Some(mut secure) => match secure.update(identity.clone()) {
            Ok(update) => {
                tracing::debug!(
                    ?update,
                    netmap_serial = reference.serial.get(),
                    "SecureNode updated in place"
                );
                if secure.set_carriers(carriers).is_err() {
                    return Err((Problem::Apply, ApplyFailure::Dataplane));
                }
                secure
            }
            Err(_) => SecureNode::new(identity.clone(), Default::default(), carriers)
                .map_err(|_| (Problem::Apply, ApplyFailure::Dataplane))?,
        },
        None => SecureNode::new(identity.clone(), Default::default(), carriers)
            .map_err(|_| (Problem::Apply, ApplyFailure::Dataplane))?,
    };
    let exit = installed.as_mut().and_then(|state| state.exit.take());
    let state = installed.insert(Installed {
        identity,
        secure: Some(secure),
        manager: Some(manager),
        exit,
        live: true,
    });
    if needs_dns {
        dns.install(snapshot.clone()).await.map_err(|error| {
            tracing::warn!(%error, "DNS scope installation failed");
            (Problem::Dns, ApplyFailure::Dns)
        })?;
    }
    if let Some((role, plan)) = new_role {
        state.exit =
            Some(super::exit::Runtime::start(role, plan).map_err(|p| (p, ApplyFailure::Underlay))?);
    }
    let secure = state.secure.as_mut().expect("installed above");
    if let Some(exit) = &mut state.exit {
        exit.synchronize(secure)
            .await
            .map_err(|p| (p, ApplyFailure::Underlay))?;
        if let Some(capability) = &mut report.exit_capability {
            capability.installed_families = vec![meshway_model::exit::ExitFamily::Ipv4];
        }
    }
    snapshot
        .ensure_current()
        .map_err(|_| (Problem::Apply, ApplyFailure::Validation))?;
    Ok(())
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
