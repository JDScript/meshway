use super::{Command, Completion, Report, Request, Target, packets::Packets};
use meshway_connectivity::{Event, Manager};
use meshway_dataplane::{Packet, secure::SecureNode};
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

pub(super) async fn run(
    mut host: Host,
    mut commands: mpsc::Receiver<Command>,
    mut targets: watch::Receiver<Option<Target>>,
    reports: watch::Sender<Report>,
    gateway: mpsc::Sender<Request>,
    completed: mpsc::Sender<Completion>,
) -> Result<(), Problem> {
    let mut installed: Option<Installed> = None;
    let mut packets = Packets::default();
    let mut buffer = vec![0u8; Packet::MAX_LEN];
    let mut generation = meshway_model::Sequence::MIN;
    let mut pending_completion = None;
    let mut last_completion: Option<meshway_control::ApplicationTicket> = None;
    let mut last_success: Option<meshway_control::ApplicationTicket> = None;
    let mut pause_until: Option<tokio::time::Instant> = None;
    loop {
        if let Some(completion) = pending_completion.take() {
            match completed.try_send(completion) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(completion)) => {
                    pending_completion = Some(completion)
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    close(&mut installed).await;
                    return host.shutdown(true).await.map_err(|_| Problem::Underlay);
                }
            }
        }
        enum Work {
            Command(Option<Command>),
            Target,
            Invalid,
            HardExpired,
            Packet(Result<usize, meshway_underlay::Error>),
            Carrier(Event),
            Tick,
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
        if pending_completion.is_some() {
            let retry = tokio::time::Instant::now() + Duration::from_secs(1);
            wake_at = Some(wake_at.map_or(retry, |at| at.min(retry)));
        }
        let work = tokio::select! { biased;
            command = commands.recv() => Work::Command(command),
            changed = targets.changed(), if pending_completion.is_none() => { if changed.is_err() { Work::Command(None) } else { Work::Target } },
            _ = async { match pause_until { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => Work::HardExpired,
            _ = async {
                if let Some(guard) = &guard && live {
                    tokio::select! {
                        _ = guard.invalidated() => {},
                        _ = host.invalidated() => {},
                    }
                } else { std::future::pending::<()>().await; }
            } => Work::Invalid,
            work = async {
                // Packet pressure must not starve incoming handshakes or lease
                // renewal work. Lifecycle/authority remain the outer priority.
                tokio::select! {
                    result = async {
                        match host.tun() { Some(tun) => tun.recv(&mut buffer).await.map_err(meshway_underlay::Error::Tun), None => std::future::pending().await }
                    }, if live => Work::Packet(result),
                    event = async {
                        match installed.as_mut().and_then(|state| state.manager.as_mut()) {
                            Some(manager) => manager.recv().await,
                            None => std::future::pending().await,
                        }
                    }, if live => Work::Carrier(event),
                    _ = async { match wake_at { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => Work::Tick,
                    else => std::future::pending().await,
                }
            } => work,
        };
        match work {
            Work::Command(Some(Command::Builder(reply))) => {
                let _ = reply.send(host.client_builder());
                continue;
            }
            Work::Command(Some(Command::Sample(reply))) => {
                let report = sample(&installed, reports.borrow().clone());
                reports.send_replace(report.clone());
                let _ = reply.send(report);
                continue;
            }
            Work::Command(Some(Command::Shutdown { keep_sink })) => {
                close(&mut installed).await;
                return host
                    .shutdown(keep_sink)
                    .await
                    .map_err(|_| Problem::Underlay);
            }
            Work::Command(None) => {
                close(&mut installed).await;
                return host.shutdown(true).await.map_err(|_| Problem::Underlay);
            }
            Work::Command(Some(Command::Stop { down, reply })) => {
                close(&mut installed).await;
                installed = None;
                pause_until = None;
                last_success = None;
                packets = Packets::default();
                let result = if down {
                    host.down().await
                } else {
                    host.suspend().await
                }
                .map_err(|_| Problem::Underlay);
                let mut report = Report::stopped();
                if result.is_err() {
                    report.plane = DataPlane::Failed;
                    report.problem = Some(Problem::Underlay);
                } else if !down {
                    report.plane = DataPlane::Suspended;
                }
                reports.send_replace(report);
                let _ = reply.send(result);
                continue;
            }
            Work::Target => {
                let target = targets.borrow_and_update().clone();
                let Some(target) = target else {
                    continue;
                };
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
                let Ok(next_generation) = generation.checked_next() else {
                    close(&mut installed).await;
                    let _ = host.shutdown(true).await;
                    return Err(Problem::Worker);
                };
                generation = next_generation;
                report.generation = generation;
                reports.send_replace(report.clone());
                let result =
                    install(&mut host, &mut installed, &mut packets, target, !refreshing).await;
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
                        last_success = report.ticket.clone();
                        report.plane = if installed.is_some() {
                            DataPlane::Ready
                        } else {
                            DataPlane::Stopped
                        }
                    }
                    Err((problem, _)) => {
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
            | Work::Carrier(Event::Closed)
            | Work::Packet(Err(_))
            | Work::HardExpired => {
                let transition = !matches!(work, Work::HardExpired | Work::Packet(Err(_)))
                    && guard
                        .as_ref()
                        .is_some_and(|guard| guard.ensure_current().is_err());
                let deadline = guard.as_ref().and_then(handoff_deadline);
                pause_until = if transition { deadline } else { None };
                close(&mut installed).await;
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
                        Work::Packet(_) => Problem::Tun,
                        Work::Carrier(_) => Problem::Transport,
                        _ => Problem::Underlay,
                    });
                }
                report.completion = None;
                report.facts = None;
                reports.send_replace(report);
                continue;
            }
            Work::Packet(Ok(size)) => match Packet::parse(buffer[..size].to_vec()) {
                Ok(packet) => packets.enqueue(packet),
                Err(meshway_dataplane::PacketError::Unsupported) => {
                    packets.drop_packet("unsupported_ip_or_local_control")
                }
                Err(_) => packets.drop_packet("malformed_ip"),
            },
            Work::Carrier(Event::Frame { link, frame }) => {
                if let Some(state) = &installed {
                    packets.incoming(link.carrier, frame.payload, &state.target);
                }
            }
            Work::Carrier(Event::Changed) | Work::Tick => {}
        }
        if let Some(state) = &mut installed
            && state.live
            && let Some(manager) = &state.manager
        {
            if let Ok(carriers) = manager.carriers() {
                let _ = state.secure.set_carriers(carriers);
            }
            let guard = state
                .target
                .identity
                .as_ref()
                .expect("installed identity")
                .snapshot();
            tokio::select! { biased;
                _ = host.invalidated() => {},
                _ = guard.invalidated() => {},
                _ = packets.drive(&mut state.secure, manager, &host, &state.target, &gateway) => {},
            }
        }
        let report = sample(&installed, reports.borrow().clone());
        reports.send_replace(report);
    }
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

fn handoff_deadline(guard: &meshway_control::ExecutionSnapshot) -> Option<tokio::time::Instant> {
    let wall_now = u64::try_from(chrono::Utc::now().timestamp_millis()).ok()?;
    let remaining = guard.expires_at_ms().checked_sub(wall_now)?;
    let deadline = tokio::time::Instant::from_std(guard.expires_at())
        .min(tokio::time::Instant::now() + Duration::from_millis(remaining));
    (deadline > tokio::time::Instant::now()).then_some(deadline)
}

/// An explicit measurement by the resource owner. Application observation time
/// remains immutable; sampling current sockets is distinct from receipt replay.
fn sample(installed: &Option<Installed>, mut report: Report) -> Report {
    report.facts = None;
    report.dependencies.clear();
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
                        *report.dependencies.entry(peer).or_default() += 1;
                    }
                }
            }
        }
    }
    report
}

async fn install(
    host: &mut Host,
    installed: &mut Option<Installed>,
    packets: &mut Packets,
    target: Target,
    allow_replacement: bool,
) -> Result<(), (Problem, ApplyFailure)> {
    let Some(identity) = &target.identity else {
        close(installed).await;
        *installed = None;
        *packets = Packets::default();
        return host
            .suspend()
            .await
            .map_err(|_| (Problem::Underlay, ApplyFailure::Underlay));
    };
    let cfg = identity
        .snapshot()
        .configuration()
        .map_err(|_| (Problem::Apply, ApplyFailure::Validation))?;
    let validation = super::requirements::validate(cfg);
    packets.clear_incoming();
    close(installed).await;
    let retained = installed
        .as_mut()
        .is_some_and(|state| state.secure.reconcile(identity.clone()).is_ok());
    tracing::debug!(retained, publication = %target.ticket.target().execution_publication, "Reconciling Node packet execution");
    if !retained && !allow_replacement {
        // Expiry may win after Control offered the refresh. Do not silently
        // rebuild a lost replay/flow owner under an already completed ticket;
        // recovery must return through a new saved application attempt.
        return Err((Problem::Apply, ApplyFailure::Dataplane));
    }
    if !retained {
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
    validation?;
    let mut manager = Manager::start(
        identity.clone(),
        Arc::new(host.sockets()),
        Default::default(),
    )
    .await
    .map_err(|_| (Problem::Transport, ApplyFailure::Listeners))?;
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
    Ok(())
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
