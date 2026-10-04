//! Authorize local peers and forward mutations to the runtime's single owner.
mod socket;

use crate::runtime::{Action, Mutation, View};
use meshway_ipc::{Command, ErrorCode, Request, Response, ResultBody, VERSION};
pub(crate) use socket::{Socket, SocketIdentity};
use std::time::Duration;
use tokio::{
    net::UnixStream,
    sync::{mpsc, oneshot, watch},
    task::JoinSet,
};

fn authorized(stream: &UnixStream) -> bool {
    stream
        .peer_cred()
        .is_ok_and(|peer| peer.uid() == 0 || peer.uid() == rustix::process::geteuid().as_raw())
}

async fn handle(mut stream: UnixStream, commands: mpsc::Sender<Mutation>, view: View) {
    if !authorized(&stream) {
        return;
    }
    let Ok(request) = meshway_ipc::read::<Request>(&mut stream).await else {
        return;
    };
    if !authorized(&stream) {
        return;
    }
    let result = if request.version != VERSION {
        Err(ErrorCode::Version)
    } else {
        let action = match request.command {
            Command::Status => None,
            Command::Up { expected_revision } => Some((Action::Up, expected_revision)),
            Command::Down { expected_revision } => Some((Action::Down, expected_revision)),
        };
        if let Some((action, expected)) = action {
            let (reply, result) = oneshot::channel();
            match commands.try_send(Mutation {
                action,
                expected,
                id: request.request_id,
                reply,
            }) {
                Ok(()) => result.await.unwrap_or(Err(ErrorCode::Stopping)),
                Err(mpsc::error::TrySendError::Full(_)) => Err(ErrorCode::Busy),
                Err(mpsc::error::TrySendError::Closed(_)) => Err(ErrorCode::Stopping),
            }
        } else {
            Ok(())
        }
    };
    let response = Response {
        version: VERSION,
        request_id: request.request_id,
        result: match result {
            Ok(()) => ResultBody::Status {
                status: Box::new(view.status()),
            },
            Err(code) => ResultBody::Error { code },
        },
    };
    let _ = meshway_ipc::write(&mut stream, &response).await;
}

pub(crate) async fn serve(
    socket: Socket,
    commands: mpsc::Sender<Mutation>,
    view: View,
    mut stop: watch::Receiver<bool>,
) -> Result<(), &'static str> {
    let mut clients = JoinSet::new();
    let outcome = loop {
        tokio::select! { biased;
            _ = stop.wait_for(|stopping| *stopping) => break Ok(()),
            Some(_) = clients.join_next(), if !clients.is_empty() => {},
            accepted = socket.listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(value) => value,
                    Err(_) => break Err("IPC accept failed."),
                };
                if clients.len() >= 32 {
                    continue;
                }
                let commands = commands.clone();
                let view = view.clone();
                clients.spawn(async move {
                    let _ = tokio::time::timeout(Duration::from_secs(10), handle(stream, commands, view)).await;
                });
            },
        }
    };
    clients.shutdown().await;
    outcome
}
