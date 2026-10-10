//! The unique network owner. It drives the Node's single [`Engine`]: every
//! netmap Control offers is one `reconfig`, the Engine's own timer ends an
//! expired lease, and packet, adjacency and DNS work borrow the components
//! between reconfigurations. An ended lease terminates every flow, circuit
//! and exit projection and keeps only the negative capture and DNS boundaries.
use super::{
    Command, ObservationPlan, Offer, Peers, Report, Request, dns::Attachment, plane::Plane,
    reporting::Publisher,
};
use meshway_connectivity::{AdjacencyInterest, Event};
use meshway_dataplane::{DEFAULT_HEADROOM, DEFAULT_TAILROOM, Packet, PacketPool};
use meshway_engine::Engine;
use meshway_ipc::{DataPlane, Problem};
use meshway_protocol::execution::ApplyFailure;
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
    let mut dns_queries = super::dns::mesh::Queries::default();
    let mut retry = Retry::default();
    let mut dns_failed = false;
    let mut sample_at = None;
    loop {
        meshway_logging::profile::report();
        enum Work {
            DnsQuery(Box<super::dns::mesh::Query>),
            ObservationPlan,
            DiagnosticSent,
            Command(Option<Command>),
            Offer,
            Retry,
            Expired,
            HostFailed,
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
        let live = engine.desired().is_some() && engine.components().secure().is_some();
        let expiry = engine.next_expiry();
        let plane = engine.components_mut();
        let can_read = live && plane.packets.can_read(&pool);
        let has_outputs = plane.packets.has_outputs();
        let mut wake_at = live.then(|| plane.packets.wake_at()).flatten();
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
        let retry_at = retry.at.filter(|_| plane.installed.is_none());
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
            _ = sleep_until(retry_at) => Work::Retry,
            _ = dns.failed(), if !dns_failed => Work::DnsFailure,
            _ = host.invalidated(), if installed.is_some() => Work::HostFailed,
            changed = network_changes.changed() => if changed.is_ok() { Work::NetworkChanged } else { Work::Command(None) },
            _ = sleep_until(sample_at) => Work::SampleDue,
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
        let work = if matches!(work, Work::CarrierClosed | Work::Packet(Err(_)))
            && engine
                .next_expiry()
                .is_some_and(|at| tokio::time::Instant::now() >= at)
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
        let structural = matches!(&work, Work::Offer | Work::Retry)
            || matches!(&work, Work::Carrier(events) if events.iter().any(|event| matches!(event, Event::Changed)));
        match work {
            Work::DnsQuery(query) => dns_queries.enqueue(*query),
            Work::NetworkChanged => {
                network_changes.borrow_and_update();
                let report =
                    sample_host(engine.components(), &diagnostics, reports.borrow().clone());
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
                let scope = engine.components().diagnostic_scope().filter(|_| live);
                diagnostics.ping(scope, request);
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
            Work::Offer | Work::Retry => {
                if matches!(work, Work::Offer) {
                    retry = Retry::default();
                }
                let offer = offers.borrow_and_update().clone();
                engine.components_mut().set_keys(offer.keys);
                engine.components_mut().set_instance(offer.instance).await;
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
                let same = engine
                    .desired()
                    .is_some_and(|desired| desired.reference() == reference);
                engine.components_mut().stage(netmap.lease);
                if same {
                    // Same content: Engine extends its own timer only.
                    if engine.reconfig(Some(&netmap)).await.is_err() {
                        tracing::warn!(
                            netmap_serial = reference.serial.get(),
                            "Netmap lease not accepted"
                        );
                    } else {
                        let mut report = reports.borrow().clone();
                        report.expires_at = lease_end(&netmap.lease);
                        reports.send_replace(report);
                    }
                    continue;
                }
                let previous = reports.borrow().clone();
                let mut report = previous.clone();
                report.plane = DataPlane::Preparing;
                report.problem = None;
                report.netmap = Some(reference);
                report.completion = None;
                report.facts = None;
                report.content = None;
                report.generation = report
                    .generation
                    .checked_next()
                    .unwrap_or(report.generation);
                reports.send_replace(report.clone());
                let plane = engine.components_mut();
                plane.report = report;
                plane.failure = None;
                let result = engine.reconfig(Some(&netmap)).await;
                let retained = engine.desired().is_some();
                let plane = engine.components_mut();
                let mut report = std::mem::replace(&mut plane.report, Report::stopped());
                report.dns_guard = plane.dns.guard();
                report.observed_at = chrono::Utc::now();
                match result {
                    Ok(()) => {
                        dns_failed = false;
                        report.completion = Some(Ok(()));
                        report.content = Some(netmap.content.clone());
                        report.expires_at = lease_end(&netmap.lease);
                        report.plane = if plane.secure().is_some() {
                            DataPlane::Ready
                        } else {
                            DataPlane::Stopped
                        };
                    }
                    Err(error) if plane.failure.is_none() && retained => {
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
                        report.plane = DataPlane::Failed;
                        retry.failed(failure);
                    }
                }
                reports.send_replace(report);
            }
            Work::Expired => {
                diagnostics.stop().await;
                if engine.expire().await.is_err() {
                    tracing::warn!("Netmap expiry cleanup failed");
                }
                if engine.desired().is_none() {
                    tracing::info!("Netmap lease ended; terminating the packet plane");
                    finish_withdrawal(&mut engine, &reports, false);
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
            Work::Tick | Work::DiagnosticSent => {}
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
            secure.set_ready_exits(peers.ready_exits());
            // DNS routes still bind the staged snapshot until DNS applies content.
            let snapshot = state.identity.snapshot().clone();
            diagnostics.synchronize(&state.desired, instance);
            tokio::select! { biased;
                _ = host.invalidated() => {},
                _ = dns.failed(), if !dns_failed => {},
                _ = async {
                    static PROFILE: meshway_logging::profile::Metric = meshway_logging::profile::Metric::new("host.drive.poll");
                    PROFILE.poll(packets.drive(secure, manager, carriers.as_ref(), host, &state.desired, &mut diagnostics, &mut dns_queries, dns.dispatcher(), &mut pool, state.exit.as_mut())).await;
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
        .and_then(|s| s.exit.as_ref())
        .map(|exit| exit.health.borrow().flows);
    report.dependencies.clear();
    report.observation_dependencies.clear();
    report.facts_observed_at = chrono::Utc::now();
    if let Some(state) = installed
        && let (Some(secure), Some(manager)) = (&state.secure, &state.manager)
    {
        report.facts = manager.snapshot().ok();
        if let Ok(circuits) = secure.ready_circuits() {
            let node = state.desired.netmap().node_id;
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
