use super::{Command, Completion, Report, Request, Target, packets::Packets};
use futures_util::FutureExt as _;
use meshway_connectivity::{Event, Manager};
use meshway_dataplane::{
    DEFAULT_HEADROOM, DEFAULT_TAILROOM, Packet, PacketPool, secure::SecureNode,
};
use meshway_ipc::{DataPlane, Problem};
use meshway_protocol::execution::ApplyFailure;
use meshway_underlay::Host;
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};

struct Installed {
    target: Target,
    secure: SecureNode,
    manager: Option<Manager>,
    live: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    mut host: Host,
    mut commands: mpsc::Receiver<Command>,
    mut targets: watch::Receiver<Option<Target>>,
    reports: super::reporting::Publisher,
    gateway: mpsc::Sender<Request>,
    completed: mpsc::Sender<Completion>,
    dns: (
        super::dns::Attachment,
        watch::Sender<Option<super::dns::intents::Plan>>,
    ),
    traversal: Arc<dyn meshway_connectivity::traversal::TraversalGateway>,
    mut diagnostic_paused: watch::Receiver<
        std::collections::BTreeSet<meshway_model::ObservationId>,
    >,
    mut observation_plans: watch::Receiver<Option<super::ObservationPlan>>,
) -> Result<(), Problem> {
    let (mut dns, dns_plans) = dns;
    let mut installed: Option<Installed> = None;
    let mut packets = Packets::default();
    let mut diagnostics = super::diagnostics::Diagnostics::default();
    // Covers the existing 128 queued packets, 32 sends and one 32-packet read
    // turn without allocating overflow buffers. The pool belongs to this owner.
    let buffers = PacketPool::new(256, Packet::MAX_LEN, DEFAULT_HEADROOM, DEFAULT_TAILROOM)
        .map_err(|_| Problem::Tun)?;
    let mut generation = meshway_model::Sequence::MIN;
    let mut pending_completion = None;
    let mut last_completion: Option<meshway_control::ApplicationTicket> = None;
    let mut last_success: Option<meshway_control::ApplicationTicket> = None;
    let mut pause_until: Option<tokio::time::Instant> = None;
    let mut preparing: Option<(Target, tokio::time::Instant)> = None;
    let mut dns_failed = false;
    let mut sample_at = None;
    loop {
        meshway_logging::profile::report();
        if let Some(completion) = pending_completion.take() {
            match completed.try_send(completion) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(completion)) => {
                    pending_completion = Some(completion)
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    diagnostics.stop().await;
                    let _ = dns.shutdown(&mut host, true).await;
                    close(&mut installed).await;
                    return host.shutdown(true).await.map_err(|_| Problem::Underlay);
                }
            }
        }
        enum Work {
            ObservationPlan,
            DiagnosticSent,
            DiagnosticsPaused,
            Command(Option<Command>),
            Target,
            Invalid,
            HardExpired,
            DnsTimeout,
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
        }
        let live = installed.as_ref().is_some_and(|state| state.live);
        let guard = installed
            .as_ref()
            .and_then(|state| state.target.identity.as_ref())
            .map(|identity| identity.snapshot().clone());
        let mut wake_at = installed
            .as_ref()
            .filter(|state| state.live)
            .and_then(|state| packets.wake_at(&state.target));
        if let Some(at) = diagnostics.wake_at() {
            wake_at = Some(wake_at.map_or(at, |old| old.min(at)));
        }
        if pending_completion.is_some() {
            let retry = tokio::time::Instant::now() + Duration::from_secs(1);
            wake_at = Some(wake_at.map_or(retry, |at| at.min(retry)));
        }
        let work = tokio::select! { biased;
            command = commands.recv() => Work::Command(command),
            _ = diagnostics.sent(), if diagnostics.has_sends() => Work::DiagnosticSent,
            _ = diagnostic_paused.changed() => Work::DiagnosticsPaused,
            changed = observation_plans.changed() => if changed.is_ok() { Work::ObservationPlan } else { Work::Command(None) },
            changed = targets.changed(), if pending_completion.is_none() => { if changed.is_err() { Work::Command(None) } else { Work::Target } },
            _ = async { match &preparing { Some((_, at)) => tokio::time::sleep_until(*at).await, None => std::future::pending().await } } => Work::DnsTimeout,
            _ = dns.failed(), if !dns_failed => Work::DnsFailure,
            _ = async { match pause_until { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => Work::HardExpired,
            _ = async {
                if let Some(guard) = &guard && live {
                    tokio::select! {
                        _ = guard.invalidated() => {},
                        _ = host.invalidated() => {},
                    }
                } else { std::future::pending::<()>().await; }
            } => Work::Invalid,
            _ = async { match sample_at { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => Work::SampleDue,
            work = async {
                // Packet pressure must not starve incoming handshakes or lease
                // renewal work. Lifecycle/authority remain the outer priority.
                tokio::select! {
                    result = async {
                         match host.tun() { Some(tun) => receive_packets(tun, &buffers).await.map_err(meshway_underlay::Error::Tun), None => std::future::pending().await }
                    }, if live => Work::Packet(result),
                    event = async {
                        match installed.as_mut().and_then(|state| state.manager.as_mut()) {
                            Some(manager) => manager.recv_batch().await,
                            None => std::future::pending().await,
                        }
                    }, if live => if event.iter().any(|event| matches!(event, Event::Closed)) { Work::CarrierClosed } else { Work::Carrier(event) },
                    _ = async { match wake_at { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => Work::Tick,
                    else => std::future::pending().await,
                }
            } => work,
        };
        let structural = matches!(&work, Work::Target | Work::DnsTimeout)
            || matches!(&work, Work::Carrier(events) if events.iter().any(|event| matches!(event, Event::Changed)));
        match work {
            Work::SampleDue => {
                let report = sample(&installed, reports.borrow().clone());
                reports.send_replace(report);
                sample_at = None;
                continue;
            }
            Work::Command(Some(Command::DetachPing(subscription))) => {
                diagnostics.detach(subscription)
            }
            Work::ObservationPlan => {
                diagnostics.plan(observation_plans.borrow_and_update().clone())
            }
            Work::Command(Some(Command::AttachPing {
                subscription,
                observation,
                request,
                lease,
                progress,
            })) => diagnostics.attach(subscription, observation, request, lease, progress),
            Work::DiagnosticsPaused => {
                diagnostics.set_paused(diagnostic_paused.borrow_and_update().clone())
            }
            Work::Command(Some(Command::Builder(reply))) => {
                let _ = reply.send(host.client_builder().map_err(|_| Problem::Underlay));
                continue;
            }
            Work::Command(Some(Command::Sample(reply))) => {
                let report = sample(&installed, reports.borrow().clone());
                let _ = reply.send(reports.sample(report));
                sample_at = None;
                continue;
            }
            Work::Command(Some(Command::Shutdown { keep_sink })) => {
                diagnostics.stop().await;
                let dns_result = dns
                    .shutdown(&mut host, keep_sink)
                    .await
                    .map_err(|_| Problem::Dns);
                close(&mut installed).await;
                let host_result = host
                    .shutdown(keep_sink)
                    .await
                    .map_err(|_| Problem::Underlay);
                return dns_result.and(host_result);
            }
            Work::Command(None) => {
                diagnostics.stop().await;
                let dns_result = dns
                    .shutdown(&mut host, true)
                    .await
                    .map_err(|_| Problem::Dns);
                close(&mut installed).await;
                return dns_result.and(host.shutdown(true).await.map_err(|_| Problem::Underlay));
            }
            Work::Command(Some(Command::Stop { down, reply })) => {
                diagnostics.stop().await;
                dns.suspend();
                preparing = None;
                dns_plans.send_replace(None);
                close(&mut installed).await;
                installed = None;
                pause_until = None;
                last_success = None;
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
                let result = dns_result.and(host_result);
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
            Work::Target | Work::DnsTimeout => {
                let timed_out = matches!(work, Work::DnsTimeout);
                let target = if timed_out {
                    preparing.as_ref().map(|(target, _)| target.clone())
                } else {
                    targets.borrow_and_update().clone()
                };
                let Some(target) = target else {
                    diagnostics.invalidate();
                    continue;
                };
                let same_scope = target
                    .identity
                    .as_ref()
                    .zip(
                        installed
                            .as_ref()
                            .and_then(|state| state.target.identity.as_ref()),
                    )
                    .is_some_and(|(next, old)| {
                        next.snapshot().same_execution(old.snapshot())
                            && old.snapshot().ensure_current().is_ok()
                    });
                if !same_scope {
                    diagnostics.invalidate();
                }
                let prepare_deadline = preparing
                    .as_ref()
                    .filter(|(_, _)| {
                        dns_plans
                            .borrow()
                            .as_ref()
                            .zip(target.identity.as_ref())
                            .is_some_and(|(plan, identity)| {
                                identity
                                    .snapshot()
                                    .configuration()
                                    .is_ok_and(|cfg| plan.matches(target.ticket.instance(), cfg))
                            })
                    })
                    .map(|(_, at)| *at)
                    .unwrap_or_else(|| {
                        tokio::time::Instant::now() + super::dns::intents::PREPARE_TIMEOUT
                    });
                pause_until = None;
                let refreshing = last_success
                    .as_ref()
                    .is_some_and(|old| same_ticket(old, &target.ticket));
                let mut report = if refreshing {
                    reports.borrow().clone()
                } else {
                    Report::stopped()
                };
                report.plane = DataPlane::Preparing;
                report.problem = None;
                report.completion = None;
                report.facts = None;
                report.ticket = Some(target.ticket.clone());
                report.guard = target
                    .identity
                    .as_ref()
                    .map(|identity| identity.snapshot().clone());
                // This installation counter allocates generations for new FDs.
                // Retained sockets report their own unchanged SocketFact value.
                let next_generation = generation.checked_next();
                let Ok(next_generation) = next_generation else {
                    diagnostics.stop().await;
                    let _ = dns.shutdown(&mut host, true).await;
                    close(&mut installed).await;
                    let _ = host.shutdown(true).await;
                    return Err(Problem::Worker);
                };
                generation = next_generation;
                report.generation = generation;
                reports.send_replace(report.clone());
                let result = install(
                    &mut host,
                    &mut installed,
                    &mut packets,
                    target.clone(),
                    !refreshing,
                    (&mut dns, &dns_plans),
                    timed_out,
                    traversal.clone(),
                    generation,
                )
                .await;
                report.dns_guard = dns.guard();
                if result == Ok(false) {
                    reports.send_replace(report);
                    preparing = Some((target, prepare_deadline));
                    close(&mut installed).await;
                    let _ = host.suspend().await;
                    continue;
                }
                preparing = None;
                let result = result.map(|_| ());
                if !refreshing {
                    report.observed_at = chrono::Utc::now();
                }
                report.completion = if refreshing && result.is_err() {
                    None
                } else {
                    Some(result.map_err(|(_, failure)| failure))
                };
                if let Some(ticket) = &report.ticket
                    && !refreshing
                    && last_completion.as_ref().is_none_or(|old| {
                        old.target() != ticket.target()
                            || old.instance() != ticket.instance()
                            || old.attempt() != ticket.attempt()
                    })
                {
                    pending_completion = Some(Completion {
                        ticket: ticket.clone(),
                        result: result.map_err(|(_, failure)| failure),
                        observed_at: report.observed_at,
                    });
                    last_completion = Some(ticket.clone());
                }
                match result {
                    Ok(()) => {
                        dns_failed = false;
                        last_success = report.ticket.clone();
                        report.plane = if installed.is_some() {
                            DataPlane::Ready
                        } else {
                            DataPlane::Stopped
                        }
                    }
                    Err((problem, failure)) => {
                        tracing::warn!(?problem, ?failure, refreshing,
                            publication = %target.ticket.target().execution_publication,
                            target_current = target.identity.as_ref().is_some_and(|identity| identity.snapshot().ensure_current().is_ok()),
                            "Node installation did not complete");
                        dns.suspend();
                        close(&mut installed).await;
                        if host.suspend().await.is_err() {
                            report.problem = Some(Problem::Underlay);
                        } else {
                            report.problem = Some(problem);
                        }
                        report.plane = DataPlane::Failed;
                    }
                }
                reports.send_replace(report);
            }
            Work::Invalid
            | Work::CarrierClosed
            | Work::Packet(Err(_))
            | Work::HardExpired
            | Work::DnsFailure => {
                diagnostics.invalidate();
                dns.suspend();
                let dns_error = matches!(work, Work::DnsFailure);
                if dns_error {
                    tracing::warn!(publication = ?guard.as_ref().map(|guard| guard.reference().execution_publication),
                        execution_current = guard.as_ref().is_some_and(|guard| guard.ensure_current().is_ok()),
                        "Required DNS component failed; suspending packet plane");
                }
                dns_failed |= dns_error;
                if dns_error && let Some((target, _)) = preparing.take() {
                    let observed_at = chrono::Utc::now();
                    pending_completion = Some(Completion {
                        ticket: target.ticket.clone(),
                        result: Err(ApplyFailure::Dns),
                        observed_at,
                    });
                    last_completion = Some(target.ticket);
                }
                let transition = !dns_error
                    && !matches!(work, Work::HardExpired | Work::Packet(Err(_)))
                    && guard
                        .as_ref()
                        .is_some_and(|guard| guard.ensure_current().is_err());
                let deadline = guard.as_ref().and_then(handoff_deadline);
                pause_until = if transition { deadline } else { None };
                if pause_until.is_some() {
                    if let Some(state) = &mut installed {
                        state.live = false;
                    }
                } else {
                    close(&mut installed).await;
                }
                packets.clear_incoming();
                // A cancelled predecessor is never usable authority. Retain
                // only its device/capture while all packet pumps are stopped,
                // bounded by its original UTC and monotonic expiration.
                let cleanup = if pause_until.is_some() {
                    Ok(())
                } else {
                    installed = None;
                    packets = Packets::default();
                    host.suspend().await
                };
                let mut report = reports.borrow().clone();
                report.plane = if cleanup.is_ok() {
                    DataPlane::Suspended
                } else {
                    DataPlane::Failed
                };
                report.problem = cleanup.err().map(|_| Problem::Underlay);
                if !transition && !matches!(work, Work::HardExpired) {
                    report.plane = DataPlane::Failed;
                    report.problem = Some(match work {
                        Work::DnsFailure => Problem::Dns,
                        Work::Packet(_) => Problem::Tun,
                        Work::CarrierClosed => Problem::Transport,
                        _ => Problem::Underlay,
                    });
                }
                report.completion = None;
                report.facts = None;
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
                    if let Some(state) = &installed {
                        packets.incoming(
                            *link,
                            frame.payload,
                            received_at,
                            &state.target,
                            &state.secure,
                        );
                    }
                }
            }
            Work::Tick | Work::DiagnosticSent => {}
        }
        if let Some(state) = &mut installed
            && state.live
            && let Some(manager) = &state.manager
        {
            // One scheduling view per turn. It grants no I/O authority: every
            // output still captures and checks the Manager's actual channel,
            // while SecureNode checks its own packet/circuit permit.
            let carriers = manager.carriers().ok();
            if let Some(carriers) = &carriers {
                let _ = state.secure.set_carriers(carriers.clone());
            }
            let guard = state
                .target
                .identity
                .as_ref()
                .expect("installed identity")
                .snapshot();
            diagnostics.synchronize(guard);
            tokio::select! { biased;
                _ = host.invalidated() => {},
                _ = guard.invalidated() => {},
                _ = dns.failed(), if !dns_failed => {},
                _ = async {
                    static PROFILE: meshway_logging::profile::Metric = meshway_logging::profile::Metric::new("host.drive.poll");
                    PROFILE.poll(packets.drive(&mut state.secure, manager, carriers.as_ref(), &host, &state.target, &gateway, &mut diagnostics)).await;
                    diagnostics.drive(&mut state.secure, manager, carriers.as_ref(), &gateway);
                } => {},
            }
        }
        if structural {
            let report = sample(&installed, reports.borrow().clone());
            reports.send_replace(report);
            sample_at = None;
        } else {
            // Packet/diagnostic work may change circuit dependencies. Coalesce
            // their observation, not packet processing: the first dirty event
            // sets a deadline that later traffic cannot postpone. The explicit
            // sample command and lifecycle/carrier changes remain immediate.
            sample_at.get_or_insert_with(|| {
                tokio::time::Instant::now() + super::reporting::CHANGE_INTERVAL
            });
        }
    }
}

/// A cancellation-safe TUN read has no read-ahead owner. After the first packet,
/// drain only immediately-ready reads, bounded to 32; lifecycle work then regains
/// priority. Packet admission still happens individually in the unique engine.
async fn receive_packets(
    tun: &meshway_tun::Tun,
    buffers: &PacketPool,
) -> Result<Vec<Result<Packet, meshway_dataplane::PacketError>>, meshway_tun::Error> {
    let acquire = || {
        buffers
            .acquire(tun.mtu())
            .map_err(|_| meshway_tun::Error::Io(std::io::Error::other("packet pool exhausted")))
    };
    let mut buffer = acquire()?;
    let first = tun.recv(buffer.payload_mut()).await?;
    let mut packets = Vec::with_capacity(32);
    buffer.truncate(first);
    packets.push(Packet::parse(buffer));
    for _ in 1..32 {
        let mut buffer = acquire()?;
        let Some(result) = tun.recv(buffer.payload_mut()).now_or_never() else {
            break;
        };
        let size = result?;
        buffer.truncate(size);
        packets.push(Packet::parse(buffer));
    }
    Ok(packets)
}

async fn close(installed: &mut Option<Installed>) {
    if let Some(state) = installed {
        state.live = false;
        if let Some(mut manager) = state.manager.take() {
            let _ = manager.shutdown().await;
        }
        // Retain the unique secure owner until a direct successor can be offered.
        // Never maintain() against an invalidated predecessor before reconcile.
    }
}

fn same_ticket(
    a: &meshway_control::ApplicationTicket,
    b: &meshway_control::ApplicationTicket,
) -> bool {
    a.target() == b.target() && a.instance() == b.instance() && a.attempt() == b.attempt()
}

fn handoff_deadline(guard: &meshway_authority::ExecutionSnapshot) -> Option<tokio::time::Instant> {
    guard.retention_deadline().ok().map(Into::into)
}

/// An explicit measurement by the resource owner. Application observation time
/// remains immutable; sampling current sockets is distinct from receipt replay.
fn sample(installed: &Option<Installed>, mut report: Report) -> Report {
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("host.sample");
    let _profile = PROFILE.enter();
    meshway_logging::profile::gauge(
        "secure.retained_circuits",
        installed
            .as_ref()
            .map(|state| state.secure.retained_circuits() as u64),
    );
    meshway_logging::profile::gauge(
        "flow.retained",
        installed
            .as_ref()
            .map(|state| state.secure.retained_flows() as u64),
    );
    meshway_logging::profile::gauge("secure.ready_circuits", None);
    report.facts = None;
    report.dependencies.clear();
    report.observation_dependencies.clear();
    report.facts_observed_at = chrono::Utc::now();
    if let Some(state) = installed
        && state.live
        && let Some(manager) = &state.manager
    {
        report.facts = manager.snapshot().ok();
        if let Ok(circuits) = state.secure.ready_circuits()
            && let Some(identity) = &state.target.identity
            && let Ok(cfg) = identity.snapshot().configuration()
        {
            meshway_logging::profile::gauge("secure.ready_circuits", Some(circuits.len() as u64));
            for circuit in circuits {
                for pair in circuit.path.windows(2) {
                    let peer = if pair[0] == cfg.node_id {
                        Some(pair[1])
                    } else if pair[1] == cfg.node_id {
                        Some(pair[0])
                    } else {
                        None
                    };
                    if let Some(peer) = peer {
                        match circuit.authorization {
                            meshway_configuration::execution::CircuitAuthorization::Business(_) => {
                                *report.dependencies.entry(peer).or_default() += 1
                            }
                            meshway_configuration::execution::CircuitAuthorization::Diagnostic(
                                id,
                            ) => {
                                let ids = report.observation_dependencies.entry(peer).or_default();
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
    target: Target,
    allow_replacement: bool,
    dns: (
        &mut super::dns::Attachment,
        &watch::Sender<Option<super::dns::intents::Plan>>,
    ),
    timed_out: bool,
    traversal: Arc<dyn meshway_connectivity::traversal::TraversalGateway>,
    generation: meshway_model::Sequence,
) -> Result<bool, (Problem, ApplyFailure)> {
    let (dns, dns_plans) = dns;
    let Some(identity) = &target.identity else {
        dns.suspend();
        dns_plans.send_replace(None);
        close(installed).await;
        *installed = None;
        *packets = Packets::default();
        return host
            .suspend()
            .await
            .map(|_| true)
            .map_err(|_| (Problem::Underlay, ApplyFailure::Underlay));
    };
    let cfg = identity
        .snapshot()
        .configuration()
        .map_err(|_| (Problem::Apply, ApplyFailure::Validation))?;
    let needs_dns = super::requirements::dns_required(&cfg.dns.settings)
        || !matches!(
            cfg.dns.egress,
            meshway_configuration::execution::DnsEgress::NoExit
        );
    let dns_prepared = if needs_dns {
        dns.plan(host, &target).await.map(Some).map_err(|error| {
            tracing::warn!(%error, "DNS preparation failed");
            (Problem::Dns, ApplyFailure::Dns)
        })
    } else {
        dns_plans.send_replace(None);
        dns.down(host)
            .await
            .map(|_| None)
            .map_err(|_| (Problem::Dns, ApplyFailure::Dns))
    };
    packets.clear_incoming();
    let physical = installed
        .as_ref()
        .and_then(|state| state.manager.as_ref())
        .is_some_and(|manager| manager.can_reconcile(identity));
    if physical {
        if let Some(state) = installed.as_mut() {
            state.live = false;
        }
    } else {
        close(installed).await;
    }
    let retained =
        installed
            .as_mut()
            .is_some_and(|state| match state.secure.reconcile(identity.clone()) {
                Ok(()) => true,
                Err(error) => {
                    if !allow_replacement {
                        tracing::warn!(?error,
                        previous = %state.target.ticket.target().execution_publication,
                        next = %target.ticket.target().execution_publication,
                        next_current = identity.snapshot().ensure_current().is_ok(),
                        "Secure owner rejected same-attempt execution refresh");
                    }
                    false
                }
            });
    tracing::debug!(retained, publication = %target.ticket.target().execution_publication, "Reconciling Node packet execution");
    if !retained && !allow_replacement {
        // Expiry may win after Control offered the refresh. Do not silently
        // rebuild a lost replay/flow owner under an already completed ticket;
        // recovery must return through a new saved application attempt.
        return Err((Problem::Apply, ApplyFailure::Dataplane));
    }
    if !retained {
        close(installed).await;
        *installed = None;
        *packets = Packets::default();
    }
    host.install(identity.snapshot().clone())
        .await
        .map_err(|error| {
            tracing::warn!(%error, "Node capture installation failed");
            match error {
                meshway_underlay::Error::Tun(_) => (Problem::Tun, ApplyFailure::Tun),
                _ => (Problem::Underlay, ApplyFailure::Underlay),
            }
        })?;
    if host.tun().is_none() {
        return Err((Problem::Tun, ApplyFailure::Tun));
    }
    // Host installation establishes the owned capture/sink first. A failed
    // required component then suspends it without leaking Mesh destinations
    // back to the host default route; no packet loop runs before every gate.
    let dns_ids = if let Some(plan) = dns_prepared? {
        // Publish descriptors only after negative DNS and Mesh capture exist.
        // Control may now create demands and atomically replace this snapshot.
        dns_plans.send_replace(Some(plan.clone()));
        if timed_out {
            return Err((Problem::Dns, ApplyFailure::Dns));
        }
        if plan.empty() {
            Default::default()
        } else if let Some(selection) = &target.dns
            && selection.plan == plan
        {
            if selection.failed {
                return Err((Problem::Dns, ApplyFailure::Dns));
            }
            selection.ids.clone()
        } else {
            return Ok(false);
        }
    } else {
        Default::default()
    };
    super::requirements::validate(
        cfg,
        &dns_ids,
        &target.retired_dns,
        identity
            .snapshot()
            .now_ms()
            .map_err(|_| (Problem::Apply, ApplyFailure::Validation))?,
    )?;
    let dns_snapshot = needs_dns.then(|| identity.snapshot().clone());
    let preserved = if physical && retained {
        installed.as_mut().and_then(|state| state.manager.take())
    } else {
        None
    };
    let mut manager = if let Some(mut manager) = preserved {
        if manager.reconcile(identity.clone()).await.is_err() {
            let _ = manager.shutdown().await;
            return Err((Problem::Transport, ApplyFailure::Listeners));
        }
        manager
    } else {
        Manager::start_with_traversal(
            identity.clone(),
            Arc::new(host.sockets()),
            Default::default(),
            traversal,
            generation,
        )
        .await
        .map_err(|_| (Problem::Transport, ApplyFailure::Listeners))?
    };
    let carriers = match manager.carriers() {
        Ok(carriers) => carriers,
        Err(_) => {
            let _ = manager.shutdown().await;
            return Err((Problem::Transport, ApplyFailure::Listeners));
        }
    };
    if let Some(state) = installed {
        if state.secure.set_carriers(carriers).is_err() {
            let _ = manager.shutdown().await;
            return Err((Problem::Apply, ApplyFailure::Dataplane));
        }
        state.manager = Some(manager);
        state.target = target;
        state.live = true;
    } else {
        let secure = match SecureNode::new(identity.clone(), Default::default(), carriers) {
            Ok(secure) => secure,
            Err(_) => {
                let _ = manager.shutdown().await;
                return Err((Problem::Apply, ApplyFailure::Dataplane));
            }
        };
        *installed = Some(Installed {
            target,
            secure,
            manager: Some(manager),
            live: true,
        });
    }
    if let Some(snapshot) = dns_snapshot {
        dns.install(snapshot, dns_ids).await.map_err(|error| {
            tracing::warn!(%error, "DNS scope installation failed");
            (Problem::Dns, ApplyFailure::Dns)
        })?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{Report, sample};

    #[test]
    fn explicit_sample_does_not_retimestamp_application_observation() {
        let mut report = Report::stopped();
        report.observed_at = chrono::Utc::now() - chrono::Duration::minutes(10);
        let application_time = report.observed_at;
        let before = chrono::Utc::now();
        let sampled = sample(&None, report);
        assert_eq!(sampled.observed_at, application_time);
        assert!(sampled.facts_observed_at >= before);
        assert!(sampled.facts.is_none());
        assert!(sampled.applied().is_none());
    }
}
