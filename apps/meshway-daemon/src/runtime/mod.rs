//! Own durable intent and coordinate the IPC service and Server control task.
mod control;
mod intent;
mod local_endpoints;
mod netcheck;
mod network;
mod peers;
mod ping;
mod status;

pub(crate) use intent::{Action, Mutation};
use intent::{Desired, Store};
use status::Report;
pub(crate) use status::View;

use crate::{
    ipc::{self, Socket},
    settings::Settings,
};
use meshway_protocol::ipc::{ErrorCode, Phase};
use std::{io::Write, sync::Arc};
use tokio::sync::{mpsc, watch};

#[derive(Clone)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Executed by the Linux-only worker.
pub(crate) struct PingRequest {
    pub id: meshway_model::RequestId,
    pub target: meshway_protocol::ipc::PingTarget,
    pub count: u16,
    pub events: mpsc::Sender<meshway_protocol::ipc::PingEvent>,
    pub cancellation: tokio_util::sync::CancellationToken,
}

/// An operator's routing profile selection for the control owner to send.
pub(crate) struct Selection {
    /// The profile to select; `None` clears the selection.
    pub profile: Option<String>,
    /// The operator stops waiting then; an unserved request is dropped, never sent late.
    pub deadline: tokio::time::Instant,
    pub reply: tokio::sync::oneshot::Sender<
        Result<meshway_configuration::selection::RoutingSelectionState, ErrorCode>,
    >,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Executed by the Linux-only worker.
pub(crate) struct PingSession {
    pub request: PingRequest,
    pub progress: watch::Sender<ping::Progress>,
}

pub async fn run(
    settings: Settings,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<(), &'static str> {
    tracing::info!(
        process_id = std::process::id(),
        version = env!("CARGO_PKG_VERSION"),
        "Node starting"
    );
    tokio::pin!(shutdown);
    let path = settings.data_dir.clone();
    let mut store = tokio::task::spawn_blocking(move || Store::open(&path))
        .await
        .map_err(|_| "Node startup worker failed.")??;
    let socket = Socket::bind(Arc::clone(&store.directory), store.record.socket).await?;
    store.socket(socket.identity())?;
    let settings = Arc::new(settings);
    let initial = store.record.desired(false);
    let (desired, desired_rx) = watch::channel(initial);
    let (reported, report_rx) = watch::channel(Report::initial(store.record.revision));
    let mut completions = report_rx.clone();
    let (sync_status, sync_rx) = watch::channel(control::SyncStatus::default());
    let (network_reported, network_report) = watch::channel(network::Report::stopped());
    let view = View {
        desired: desired_rx.clone(),
        report: report_rx,
        server: settings.server.clone(),
        network_mode: settings.network_mode,
        network: network_report,
        sync: sync_rx,
    };
    let (commands, mut received) = mpsc::channel::<Mutation>(16);
    let (pings, mut ping_requests) = mpsc::channel::<PingRequest>(8);
    let (netchecks, mut netcheck_requests) =
        mpsc::channel::<tokio::sync::oneshot::Sender<meshway_protocol::ipc::NetcheckReport>>(4);
    let (selections, selection_requests) = mpsc::channel::<Selection>(4);
    let (packet_pings, local_pings) = watch::channel(None);
    let (server_probe, server_probes) = watch::channel(None);
    let (stop_ipc, stop_rx) = watch::channel(false);
    let mut control = tokio::spawn(control::run(
        settings.clone(),
        desired_rx,
        reported,
        store.directory.clone(),
        network_reported,
        packet_pings,
        server_probe,
        sync_status,
        selection_requests,
    ));
    let mut listener = tokio::spawn(ipc::serve(
        socket,
        ipc::Channels {
            commands,
            pings,
            netchecks,
            selections,
        },
        view.clone(),
        stop_rx.clone(),
    ));
    let mut ping_forwarders = tokio::task::JoinSet::new();
    let mut netcheck_runs = tokio::task::JoinSet::new();
    let mut listener_done = false;
    let mut control_done = false;
    let mut failure = None;
    let mut rotations = Vec::new();
    let socket_path = settings.data_dir.join(ipc::SOCKET_FILE);
    tracing::info!(socket = %socket_path.display(), "Local IPC ready");
    if writeln!(
        std::io::stdout().lock(),
        "Local IPC ready: {}",
        socket_path.display()
    )
    .is_err()
    {
        failure = Some("Cannot write Node readiness.");
    } else {
        crate::service::notify("READY=1");
    }
    while failure.is_none() {
        tokio::select! { biased;
            _ = &mut shutdown => break,
            _ = ping_forwarders.join_next(), if !ping_forwarders.is_empty() => {},
            _ = netcheck_runs.join_next(), if !netcheck_runs.is_empty() => {},
            reply = netcheck_requests.recv() => {
                if let Some(reply) = reply {
                    // Bounded like the IPC queue: four in flight at most.
                    if netcheck_runs.len() < 4 {
                        let local = local_pings.borrow().clone();
                        let server = server_probes.borrow().clone();
                        let origin = settings.server.clone();
                        netcheck_runs.spawn(async move {
                            let _ = reply.send(netcheck::run(origin, local, server).await);
                        });
                    }
                }
            },
            result = &mut listener => {
                listener_done = true;
                failure = Some(result.ok().and_then(Result::err).unwrap_or("IPC service stopped unexpectedly."));
            },
            result = &mut control => {
                control_done = true;
                failure = Some(result.ok().and_then(Result::err).unwrap_or("Control worker stopped unexpectedly."));
            },
            command = received.recv() => {
                let Some(command) = command else { break; };
                if apply(&mut store, &desired, &view, command, &mut rotations).await.is_err() {
                    failure = Some("Node intent persistence failed.");
                }
            },
            updated = completions.changed() => {
                if updated.is_ok() {
                    let report = completions.borrow_and_update().clone();
                    if complete(&mut store, &desired, &report, &mut rotations).await.is_err() {
                        failure = Some("Node operation receipt persistence failed.");
                    }
                }
            },
            ping = ping_requests.recv() => {
                if let Some(mut ping) = ping {
                    let code = ping::admission(&view.status()).err().or_else(|| (ping_forwarders.len() >= 8).then_some(meshway_protocol::ipc::PingError::Busy));
                    if let Some(code) = code {
                        let _ = ping.events.try_send(meshway_protocol::ipc::PingEvent::Failed { code });
                        ping.cancellation.cancel();
                    } else {
                        let client = ping.events.clone();
                        let cancellation = ping.cancellation.clone();
                        let (events, incoming) = mpsc::channel(8);
                        let (progress, progress_rx) = watch::channel(ping::Progress::Setup);
                        let intent = ping::Intent::capture(view.desired.clone());
                        ping.events = events;
                        ping_forwarders.spawn(ping::forward(client, incoming, cancellation, stop_rx.clone(), progress_rx, intent));
                        if let Some(gateway) = local_pings.borrow().as_ref() {
                            gateway.send(PingSession { request: ping, progress });
                        } else {
                            let _ = ping.events.try_send(meshway_protocol::ipc::PingEvent::Failed { code: meshway_protocol::ipc::PingError::NotRunning });
                            ping.cancellation.cancel();
                        }
                    }
                }
            },
        }
    }
    crate::service::notify("STOPPING=1");
    // No new mutations; every already queued command still receives an owner
    // result. A broken durable store rejects further work rather than resetting.
    received.close();
    ping_requests.close();
    while let Some(ping) = ping_requests.recv().await {
        let _ = ping
            .events
            .try_send(meshway_protocol::ipc::PingEvent::Failed {
                code: meshway_protocol::ipc::PingError::Stopped,
            });
        ping.cancellation.cancel();
    }
    while let Some(command) = received.recv().await {
        if failure.is_some() {
            let _ = command.reply.send(Err(ErrorCode::Storage));
        } else if apply(&mut store, &desired, &view, command, &mut rotations)
            .await
            .is_err()
        {
            failure = Some("Node intent persistence failed.");
        }
    }
    desired.send_modify(|plan| plan.shutdown = true);
    for (_, reply) in rotations.drain(..) {
        let _ = reply.send(Err(ErrorCode::Stopping));
    }
    stop_ipc.send_replace(true);
    while ping_forwarders.join_next().await.is_some() {}
    netcheck_runs.shutdown().await;
    if !listener_done {
        match listener.await {
            Ok(Ok(())) => {}
            _ => {
                failure.get_or_insert("IPC shutdown failed.");
            }
        }
    }
    if !control_done {
        match control.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                failure.get_or_insert(error);
            }
            Err(_) => {
                failure.get_or_insert("Control shutdown failed.");
            }
        }
    }
    drop(store);
    tracing::info!(failed = failure.is_some(), "Node stopped");
    writeln!(std::io::stdout().lock(), "Node stopped.")
        .map_err(|_| "Cannot write Node shutdown.")?;
    failure.map_or(Ok(()), Err)
}

async fn apply(
    store: &mut Store,
    desired: &watch::Sender<Desired>,
    view: &View,
    command: Mutation,
    rotations: &mut RotationWaiters,
) -> Result<(), ()> {
    if matches!(command.action, Action::RotateKey | Action::Logout) && rotations.len() >= 16 {
        let _ = command.reply.send(Err(ErrorCode::Busy));
        return Ok(());
    }
    let has_key = command.enrollment_key.is_some();
    let blocked = matches!(view.status().phase, Phase::Blocked);
    // A pending leave ends only by its completion, another logout, or an
    // explicit new enrollment; nothing else may rejoin with this identity.
    let leaving = view
        .report
        .borrow()
        .control
        .as_ref()
        .is_some_and(|snapshot| snapshot.status().leaving.is_some());
    let mutation = store.mutation(
        command.action,
        command.id,
        command.expected,
        blocked,
        command.enrollment_key,
    );
    if leaving
        && mutation.as_ref().is_ok_and(|record| record.is_some())
        && match command.action {
            Action::Up => !has_key,
            Action::RotateKey | Action::Reenroll => true,
            Action::Down | Action::Logout => false,
        }
    {
        let _ = command.reply.send(Err(ErrorCode::Leaving));
        return Ok(());
    }
    if matches!(
        command.action,
        Action::RotateKey | Action::Reenroll | Action::Logout
    ) && mutation.as_ref().is_ok_and(|record| record.is_some())
    {
        let state = view
            .report
            .borrow()
            .control
            .as_ref()
            .map(meshway_control::StatusSnapshot::status);
        let rejected = if state.as_ref().is_some_and(|state| state.reload_required) {
            Some(ErrorCode::Storage)
        } else if state.as_ref().is_none_or(|state| state.binding.is_none()) {
            Some(ErrorCode::NotReady)
        } else {
            None
        };
        if let Some(code) = rejected {
            let _ = command.reply.send(Err(code));
            return Ok(());
        }
    }
    if has_key
        && !leaving
        && mutation.as_ref().is_ok_and(|record| record.is_some())
        && (view.report.borrow().control.is_none() || view.status().identity.is_some())
    {
        let _ = command.reply.send(Err(ErrorCode::NotReady));
        return Ok(());
    }
    let result = match mutation {
        Err(code) => Err(code),
        Ok(None) => Ok(()),
        Ok(Some(record)) => {
            if store.commit(record).await.is_err() {
                let _ = command.reply.send(Err(ErrorCode::Storage));
                return Err(());
            }
            let mut next = store
                .record
                .desired(matches!(command.action, Action::Up | Action::Reenroll));
            next.reenroll = command.action == Action::Reenroll;
            desired.send_replace(next);
            tracing::info!(
                request_id = %command.id,
                revision = store.record.revision.get(),
                desired_up = store.record.desired_up,
                "Node intent committed"
            );
            Ok(())
        }
    };
    if let Err(code) = result {
        tracing::warn!(request_id = %command.id, error = ?code, "Node intent rejected");
    }
    if result.is_ok() && matches!(command.action, Action::RotateKey | Action::Logout) {
        let pending = if command.action == Action::RotateKey {
            &store.record.rotation
        } else {
            &store.record.logout
        };
        if let Some(rotation) = pending
            && rotation.id == command.id
        {
            if let Some(result) = rotation.result {
                let _ = command.reply.send(result);
            } else {
                rotations.push((command.id, command.reply));
            }
            return Ok(());
        }
        let _ = command.reply.send(Err(ErrorCode::Conflict));
        return Ok(());
    }
    let _ = command.reply.send(result);
    Ok(())
}

type RotationWaiters = Vec<(
    meshway_model::RequestId,
    tokio::sync::oneshot::Sender<Result<(), ErrorCode>>,
)>;

async fn complete(
    store: &mut Store,
    desired: &watch::Sender<Desired>,
    report: &Report,
    waiters: &mut RotationWaiters,
) -> Result<(), ()> {
    if let Some(outcome) = report.rotation
        && store
            .record
            .rotation
            .as_ref()
            .is_some_and(|rotation| rotation.id == outcome.id && rotation.result.is_none())
    {
        let mut record = store.record.clone();
        record.rotation.as_mut().expect("pending rotation").result = Some(outcome.result);
        store.commit(record).await.map_err(|_| ())?;
        desired.send_replace(store.record.desired(false));
        let mut pending = Vec::new();
        for (id, reply) in waiters.drain(..) {
            if id == outcome.id {
                let _ = reply.send(outcome.result);
            } else {
                pending.push((id, reply));
            }
        }
        *waiters = pending;
    }
    if let Some(outcome) = report.logout
        && let awaited = store
            .record
            .logout
            .as_ref()
            .is_some_and(|logout| logout.id == outcome.id && logout.result.is_none())
        // A retried leave can also complete after its request was answered.
        && (awaited
            || (outcome.result.is_ok()
                && (store.record.desired_up || store.record.enrollment_key.is_some())))
    {
        let mut record = store.record.clone();
        if awaited {
            record.logout.as_mut().expect("pending logout").result = Some(outcome.result);
        }
        if outcome.result.is_ok() {
            // The identity is gone: the Node stays down until a new enrollment.
            record.desired_up = false;
            record.enrollment_key = None;
            record.revision = record.revision.checked_next().map_err(|_| ())?;
        }
        store.commit(record).await.map_err(|_| ())?;
        desired.send_replace(store.record.desired(false));
        let mut pending = Vec::new();
        for (id, reply) in waiters.drain(..) {
            if id == outcome.id {
                let _ = reply.send(outcome.result);
            } else {
                pending.push((id, reply));
            }
        }
        *waiters = pending;
    }
    if report.revision == store.record.revision
        && report
            .control
            .as_ref()
            .is_some_and(|snapshot| snapshot.status().binding.is_some())
        && store.record.enrollment_key.is_some()
    {
        let mut record = store.record.clone();
        record.enrollment_key = None;
        store.commit(record).await.map_err(|_| ())?;
        desired.send_modify(|plan| plan.enrollment_key = None);
    }
    Ok(())
}
