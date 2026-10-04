use crate::{
    ipc::{self, Mutation, Socket},
    settings::Settings,
    status::{Desired, Report, View},
    store::{Action, Store},
    worker,
};
use meshway_ipc::{ErrorCode, Phase};
use std::{io::Write, sync::Arc};
use tokio::sync::{mpsc, watch};

pub async fn run(
    settings: Settings,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<(), &'static str> {
    tokio::pin!(shutdown);
    let path = settings.data_dir.clone();
    let mut store = tokio::task::spawn_blocking(move || Store::open(&path))
        .await
        .map_err(|_| "Node startup worker failed.")??;
    let socket = Socket::bind(&mut store).await?;
    let settings = Arc::new(settings);
    let initial = Desired {
        revision: store.record.revision,
        up: store.record.desired_up,
        shutdown: false,
        retry_enrollment: false,
    };
    let (desired, desired_rx) = watch::channel(initial);
    let (reported, report_rx) = watch::channel(Report::initial(store.record.revision));
    let view = View {
        desired: desired_rx.clone(),
        report: report_rx,
        server: settings.server.clone(),
    };
    let (commands, mut received) = mpsc::channel::<Mutation>(16);
    let (stop_ipc, stop_rx) = watch::channel(false);
    let mut control = tokio::spawn(worker::run(settings.clone(), desired_rx, reported));
    let mut listener = tokio::spawn(ipc::serve(socket, commands, view.clone(), stop_rx));
    let mut listener_done = false;
    let mut control_done = false;
    let mut failure = None;
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
            result = &mut listener => { listener_done = true; failure = Some(result.ok().and_then(Result::err).unwrap_or("IPC service stopped unexpectedly.")); },
            _ = &mut control => { control_done = true; failure = Some("Control worker stopped unexpectedly."); },
            command = received.recv() => {
                let Some(command) = command else { break; };
                if apply(&mut store,&desired,&view,command).await.is_err() { failure = Some("Node intent persistence failed."); }
            },
        }
    }
    // No new mutations; every already queued command still receives an owner
    // result. A broken durable store rejects further work rather than resetting.
    received.close();
    while let Some(command) = received.recv().await {
        if failure.is_some() {
            let _ = command.reply.send(Err(ErrorCode::Storage));
        } else if apply(&mut store, &desired, &view, command).await.is_err() {
            failure = Some("Node intent persistence failed.");
        }
    }
    desired.send_modify(|plan| plan.shutdown = true);
    stop_ipc.send_replace(true);
    if !listener_done {
        match listener.await {
            Ok(Ok(())) => {}
            _ => {
                failure.get_or_insert("IPC shutdown failed.");
            }
        }
    }
    if !control_done && control.await.is_err() {
        failure.get_or_insert("Control shutdown failed.");
    }
    drop(store);
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
            Ok(())
        }
    };
    let _ = command.reply.send(result);
    Ok(())
}
