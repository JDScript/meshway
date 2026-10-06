//! Own durable intent and coordinate the IPC service and Server control task.
mod control;
mod intent;
mod network;
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
use meshway_ipc::{ErrorCode, Phase};
use std::{io::Write, sync::Arc};
use tokio::sync::{mpsc, watch};

#[derive(Clone)]
pub(crate) struct PingRequest {
    pub id: meshway_model::RequestId,
    pub target: meshway_protocol::NodeObservationTarget,
    pub count: u16,
    pub events: mpsc::Sender<meshway_ipc::PingEvent>,
    pub cancellation: tokio_util::sync::CancellationToken,
}

struct PingSession {
    request: PingRequest,
    lease: watch::Sender<Option<meshway_control::ObservationLease>>,
    progress: watch::Sender<ping::Progress>,
    intent: ping::Intent,
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
    let initial = Desired {
        revision: store.record.revision,
        up: store.record.desired_up,
        shutdown: false,
        retry_enrollment: false,
    };
    let (desired, desired_rx) = watch::channel(initial);
    let (reported, report_rx) = watch::channel(Report::initial(store.record.revision));
    let (network_reported, network_report) = watch::channel(network::Report::stopped());
    let view = View {
        desired: desired_rx.clone(),
        report: report_rx,
        server: settings.server.clone(),
        network_mode: settings.network_mode,
        network: network_report,
    };
    let (commands, mut received) = mpsc::channel::<Mutation>(16);
    let (pings, mut ping_requests) = mpsc::channel::<PingRequest>(8);
    let (control_pings, control_ping_requests) = mpsc::channel::<PingSession>(8);
    let (stop_ipc, stop_rx) = watch::channel(false);
    let mut control = tokio::spawn(control::run(
        settings.clone(),
        desired_rx,
        reported,
        store.directory.clone(),
        network_reported,
        control_ping_requests,
    ));
    let mut listener = tokio::spawn(ipc::serve(
        socket,
        commands,
        pings,
        view.clone(),
        stop_rx.clone(),
    ));
    let mut ping_forwarders = tokio::task::JoinSet::new();
    let mut listener_done = false;
    let mut control_done = false;
    let mut failure = None;
    tracing::info!(socket = %settings.data_dir.join("control.sock").display(), "Local IPC ready");
    if writeln!(
        std::io::stdout().lock(),
        "Local IPC ready: {}",
        settings.data_dir.join("control.sock").display()
    )
    .is_err()
    {
        failure = Some("Cannot write Node readiness.");
    }
    while failure.is_none() {
        tokio::select! { biased;
            _ = &mut shutdown => break,
            _ = ping_forwarders.join_next(), if !ping_forwarders.is_empty() => {},
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
                if apply(&mut store, &desired, &view, command).await.is_err() {
                    failure = Some("Node intent persistence failed.");
                }
            },
            ping = ping_requests.recv() => {
                if let Some(mut ping) = ping {
                    let code = ping::admission(&view.status()).err().or_else(|| (ping_forwarders.len() >= 8).then_some(meshway_ipc::PingError::Busy));
                    if let Some(code) = code {
                        let _ = ping.events.try_send(meshway_ipc::PingEvent::Failed { code });
                        ping.cancellation.cancel();
                    } else {
                        let client = ping.events.clone();
                        let cancellation = ping.cancellation.clone();
                        let (events, incoming) = mpsc::channel(8);
                        let (lease, lease_rx) = watch::channel(None);
                        let (progress, progress_rx) = watch::channel(ping::Progress::Setup);
                        let intent = ping::Intent::capture(view.desired.clone());
                        ping.events = events;
                        ping_forwarders.spawn(ping::forward(client, incoming, cancellation, stop_rx.clone(), lease_rx, progress_rx, intent.clone()));
                        if let Err(error) = control_pings.try_send(PingSession { request: ping, lease, progress, intent }) {
                            let ping = error.into_inner().request;
                            let _ = ping.events.try_send(meshway_ipc::PingEvent::Failed { code: meshway_ipc::PingError::Busy });
                            ping.cancellation.cancel();
                        }
                    }
                }
            },
        }
    }
    // No new mutations; every already queued command still receives an owner
    // result. A broken durable store rejects further work rather than resetting.
    received.close();
    ping_requests.close();
    while let Some(ping) = ping_requests.recv().await {
        let _ = ping.events.try_send(meshway_ipc::PingEvent::Failed {
            code: meshway_ipc::PingError::Stopped,
        });
        ping.cancellation.cancel();
    }
    while let Some(command) = received.recv().await {
        if failure.is_some() {
            let _ = command.reply.send(Err(ErrorCode::Storage));
        } else if apply(&mut store, &desired, &view, command).await.is_err() {
            failure = Some("Node intent persistence failed.");
        }
    }
    desired.send_modify(|plan| plan.shutdown = true);
    stop_ipc.send_replace(true);
    while ping_forwarders.join_next().await.is_some() {}
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
) -> Result<(), ()> {
    let blocked = matches!(view.status().phase, Phase::Blocked);
    let mutation = store.mutation(command.action, command.id, command.expected, blocked);
    let result = match mutation {
        Err(code) => Err(code),
        Ok(None) => Ok(()),
        Ok(Some(record)) => {
            if store.commit(record).await.is_err() {
                let _ = command.reply.send(Err(ErrorCode::Storage));
                return Err(());
            }
            desired.send_replace(Desired {
                revision: store.record.revision,
                up: store.record.desired_up,
                shutdown: false,
                retry_enrollment: command.action == Action::Up,
            });
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
    let _ = command.reply.send(result);
    Ok(())
}
