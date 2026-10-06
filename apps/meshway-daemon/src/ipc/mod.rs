//! Authorize local peers and forward mutations to the runtime's single owner.
mod socket;

use crate::runtime::{Action, Mutation, PingRequest, View};
use meshway_ipc::{
    Command, ErrorCode, PingError, PingEvent, Request, Response, ResultBody, VERSION,
};
use meshway_model::Sequence;
pub(crate) use socket::{Socket, SocketIdentity};
use std::time::Duration;
use tokio::{
    io::AsyncReadExt,
    net::UnixStream,
    sync::{mpsc, oneshot, watch},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

fn authorized(stream: &UnixStream) -> bool {
    stream
        .peer_cred()
        .is_ok_and(|peer| peer.uid() == 0 || peer.uid() == rustix::process::geteuid().as_raw())
}

async fn handle(
    mut stream: UnixStream,
    commands: mpsc::Sender<Mutation>,
    pings: mpsc::Sender<PingRequest>,
    view: View,
    stop: watch::Receiver<bool>,
) {
    let deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
    if !authorized(&stream) {
        return;
    }
    let Ok(Ok(request)) =
        tokio::time::timeout_at(deadline, meshway_ipc::read::<Request>(&mut stream)).await
    else {
        return;
    };
    if !authorized(&stream) {
        return;
    }
    if request.version == VERSION && matches!(request.command, Command::Ping { .. }) {
        handle_ping(stream, request, pings, stop).await;
        return;
    }
    let _ = tokio::time::timeout_at(deadline, one_shot(&mut stream, commands, view, request)).await;
}

async fn handle_ping(
    mut stream: UnixStream,
    request: Request,
    pings: mpsc::Sender<PingRequest>,
    stop: watch::Receiver<bool>,
) {
    if let Command::Ping { target, count } = &request.command {
        let cancellation = CancellationToken::new();
        let _cancel = cancellation.clone().drop_guard();
        let (events, received) = mpsc::channel(16);
        let ping = PingRequest {
            id: request.request_id,
            target: target.clone(),
            count: *count,
            events,
            cancellation,
        };
        match pings.try_send(ping) {
            Ok(()) => ping_stream(stream, request, received, stop).await,
            Err(error) => {
                let code = match error {
                    mpsc::error::TrySendError::Full(_) => PingError::Busy,
                    mpsc::error::TrySendError::Closed(_) => PingError::Stopped,
                };
                let response = Response {
                    version: VERSION,
                    request_id: request.request_id,
                    result: ResultBody::Ping {
                        sequence: Sequence::MIN,
                        event: PingEvent::Failed { code },
                    },
                };
                let _ =
                    tokio::time::timeout(WRITE_TIMEOUT, meshway_ipc::write(&mut stream, &response))
                        .await;
            }
        }
    }
}

async fn one_shot(
    stream: &mut UnixStream,
    commands: mpsc::Sender<Mutation>,
    view: View,
    request: Request,
) {
    let result = if request.version != VERSION {
        Err(ErrorCode::Version)
    } else {
        let action = match request.command {
            Command::Status => None,
            Command::Up { expected_revision } => Some((Action::Up, expected_revision)),
            Command::Down { expected_revision } => Some((Action::Down, expected_revision)),
            Command::Ping { .. } => return,
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
    let _ = meshway_ipc::write(stream, &response).await;
}

async fn ping_stream(
    stream: UnixStream,
    request: Request,
    mut events: mpsc::Receiver<PingEvent>,
    mut stop: watch::Receiver<bool>,
) {
    let (mut reader, mut writer) = stream.into_split();
    let mut unexpected = [0; 1];
    let mut sequence = Sequence::MIN;
    loop {
        let event = tokio::select! { biased;
            _ = stop.wait_for(|stopping| *stopping) => return,
            // EOF, read failure and any extra client input all end this subscriber.
            _ = reader.read(&mut unexpected) => return,
            event = events.recv() => event.unwrap_or(PingEvent::Failed { code: PingError::Stopped }),
        };
        let terminal = matches!(
            event,
            PingEvent::Completed { .. } | PingEvent::Failed { .. }
        );
        let response = Response {
            version: VERSION,
            request_id: request.request_id,
            result: ResultBody::Ping { sequence, event },
        };
        let written = tokio::select! { biased;
            _ = stop.wait_for(|stopping| *stopping) => return,
            _ = reader.read(&mut unexpected) => return,
            written = tokio::time::timeout(WRITE_TIMEOUT, meshway_ipc::write(&mut writer, &response)) => written,
        };
        if !matches!(written, Ok(Ok(()))) || terminal {
            return;
        }
        let Ok(next) = sequence.checked_next() else {
            return;
        };
        sequence = next;
    }
}

pub(crate) async fn serve(
    socket: Socket,
    commands: mpsc::Sender<Mutation>,
    pings: mpsc::Sender<PingRequest>,
    view: View,
    mut stop: watch::Receiver<bool>,
) -> Result<(), &'static str> {
    let mut clients = JoinSet::new();
    let outcome = loop {
        tokio::select! { biased;
            _ = async { let _ = stop.wait_for(|stopping| *stopping).await; } => break Ok(()),
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
                let pings = pings.clone();
                let view = view.clone();
                let stop = stop.clone();
                clients.spawn(async move {
                    handle(stream, commands, pings, view, stop).await;
                });
            },
        }
    };
    clients.shutdown().await;
    outcome
}

#[cfg(test)]
mod tests;
