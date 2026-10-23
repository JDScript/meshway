//! Dispatch already authenticated streams to the runtime's single owner.

use crate::runtime::{Action, Mutation, PingRequest, Selection, View};
use meshway_model::Sequence;
use meshway_protocol::ipc::{
    Command, ErrorCode, NetcheckReport, PingError, PingEvent, Request, Response, ResultBody,
    VERSION,
};
use std::time::Duration;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite},
    sync::{mpsc, oneshot, watch},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// One selection request: Control's 20 s HTTP bound plus lock waits.
const SELECTION_TIMEOUT: Duration = Duration::from_secs(25);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Legacy Unix endpoint identity in the durable intent record. The transport
/// host verifies it; the Node only preserves it across restarts.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocketIdentity {
    /// Record compatibility only: device numbers can change across mounts;
    /// the Unix host checks the directory's current device and this inode.
    pub device: u64,
    pub inode: u64,
}

pub(crate) const SOCKET_FILE: &str = "control.sock";

/// An authenticated, framed IPC byte stream. Authentication belongs to its host.
pub trait Connection: AsyncRead + AsyncWrite + Unpin + Send + 'static {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Connection for T {}

/// Bounded admission to the IPC dispatcher (32 live connections).
#[derive(Clone)]
pub struct Handle {
    connections: mpsc::Sender<Accepted>,
    slots: std::sync::Arc<tokio::sync::Semaphore>,
}
pub(crate) type Accepted = (Box<dyn Connection>, tokio::sync::OwnedSemaphorePermit);
impl Handle {
    pub(crate) fn new(connections: mpsc::Sender<Accepted>) -> Self {
        Self {
            connections,
            slots: std::sync::Arc::new(tokio::sync::Semaphore::new(32)),
        }
    }
    /// Submit a connection only after authenticating its peer. A full or stopped
    /// dispatcher rejects and closes it without creating an unbounded waiter.
    pub fn submit(&self, connection: impl Connection) -> Result<(), ErrorCode> {
        if self.connections.is_closed() {
            return Err(ErrorCode::Stopping);
        }
        let slot = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ErrorCode::Busy)?;
        self.connections
            .try_send((Box::new(connection), slot))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => ErrorCode::Busy,
                mpsc::error::TrySendError::Closed(_) => ErrorCode::Stopping,
            })
    }
}

#[derive(Clone)]
pub(crate) struct Channels {
    pub commands: mpsc::Sender<Mutation>,
    pub pings: mpsc::Sender<PingRequest>,
    pub netchecks: mpsc::Sender<oneshot::Sender<NetcheckReport>>,
    pub selections: mpsc::Sender<Selection>,
}

async fn handle(
    mut stream: impl Connection,
    channels: Channels,
    view: View,
    stop: watch::Receiver<bool>,
) {
    let Channels {
        commands,
        pings,
        netchecks,
        selections,
    } = channels;
    let deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
    let Ok(Ok(request)) = tokio::time::timeout_at(
        deadline,
        meshway_protocol::ipc::read::<Request>(&mut stream),
    )
    .await
    else {
        return;
    };
    if request.version == VERSION && matches!(request.command, Command::Ping { .. }) {
        handle_ping(stream, request, pings, stop).await;
        return;
    }
    if request.version == VERSION && matches!(request.command, Command::Netcheck) {
        handle_netcheck(stream, request, netchecks).await;
        return;
    }
    if request.version == VERSION
        && matches!(
            request.command,
            Command::SelectProfile { .. } | Command::ClearProfile
        )
    {
        handle_selection(stream, request, selections, view).await;
        return;
    }
    let deadline = if matches!(
        request.command,
        Command::RotateKey { .. } | Command::Logout { .. }
    ) {
        tokio::time::Instant::now() + Duration::from_secs(65)
    } else {
        deadline
    };
    let _ = tokio::time::timeout_at(deadline, one_shot(&mut stream, commands, view, request)).await;
}

async fn handle_netcheck(
    mut stream: impl Connection,
    request: Request,
    netchecks: mpsc::Sender<oneshot::Sender<NetcheckReport>>,
) {
    let (reply, report) = oneshot::channel();
    let result = match netchecks.try_send(reply) {
        Ok(()) => match tokio::time::timeout(REQUEST_TIMEOUT, report).await {
            Ok(Ok(report)) => ResultBody::Netcheck {
                report: Box::new(report),
            },
            Ok(Err(_)) => ResultBody::Error {
                code: ErrorCode::Stopping,
            },
            Err(_) => ResultBody::Error {
                code: ErrorCode::Busy,
            },
        },
        Err(mpsc::error::TrySendError::Full(_)) => ResultBody::Error {
            code: ErrorCode::Busy,
        },
        Err(mpsc::error::TrySendError::Closed(_)) => ResultBody::Error {
            code: ErrorCode::Stopping,
        },
    };
    let response = Response {
        version: VERSION,
        request_id: request.request_id,
        result,
    };
    let _ = tokio::time::timeout(
        WRITE_TIMEOUT,
        meshway_protocol::ipc::write(&mut stream, &response),
    )
    .await;
}

/// A selection or its clearing needs an online Node holding a netmap; the Server's answer
/// is relayed as is and nothing of it is stored here.
async fn handle_selection(
    mut stream: impl Connection,
    request: Request,
    selections: mpsc::Sender<Selection>,
    view: View,
) {
    let profile = match request.command {
        Command::SelectProfile { profile } => Some(profile),
        Command::ClearProfile => None,
        _ => return,
    };
    let status = view.status();
    let result = if !status.desired_up
        || status.identity.is_none()
        || status.netmap.is_none()
        || status.instance.is_none()
    {
        Err(ErrorCode::NotReady)
    } else {
        let (reply, answer) = oneshot::channel();
        let deadline = tokio::time::Instant::now() + SELECTION_TIMEOUT;
        match selections.try_send(Selection {
            profile,
            deadline,
            reply,
        }) {
            Ok(()) => match tokio::time::timeout_at(deadline, answer).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err(ErrorCode::Stopping),
                Err(_) => Err(ErrorCode::Busy),
            },
            Err(mpsc::error::TrySendError::Full(_)) => Err(ErrorCode::Busy),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(ErrorCode::Stopping),
        }
    };
    let response = Response {
        version: VERSION,
        request_id: request.request_id,
        result: match result {
            Ok(selection) => ResultBody::Selection {
                selection: Box::new(selection),
            },
            Err(code) => ResultBody::Error { code },
        },
    };
    let _ = tokio::time::timeout(
        WRITE_TIMEOUT,
        meshway_protocol::ipc::write(&mut stream, &response),
    )
    .await;
}

async fn handle_ping(
    mut stream: impl Connection,
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
                let _ = tokio::time::timeout(
                    WRITE_TIMEOUT,
                    meshway_protocol::ipc::write(&mut stream, &response),
                )
                .await;
            }
        }
    }
}

async fn one_shot(
    stream: &mut impl Connection,
    commands: mpsc::Sender<Mutation>,
    view: View,
    request: Request,
) {
    let result = if request.version != VERSION {
        Err(ErrorCode::Version)
    } else {
        let action = match request.command {
            Command::Status => Ok(None),
            Command::Up {
                expected_revision,
                reenroll: true,
                enrollment_key: None,
            } => Ok(Some((Action::Reenroll, expected_revision, None))),
            // Enrollment keys cannot re-enroll an identity (Console approval only).
            Command::Up { reenroll: true, .. } => Err(ErrorCode::Contract),
            Command::Up {
                expected_revision,
                enrollment_key,
                reenroll: false,
            } => Ok(Some((Action::Up, expected_revision, enrollment_key))),
            Command::Down { expected_revision } => {
                Ok(Some((Action::Down, expected_revision, None)))
            }
            Command::RotateKey { expected_revision } => {
                Ok(Some((Action::RotateKey, expected_revision, None)))
            }
            Command::Logout { expected_revision } => {
                Ok(Some((Action::Logout, expected_revision, None)))
            }
            Command::Ping { .. }
            | Command::Netcheck
            | Command::SelectProfile { .. }
            | Command::ClearProfile => return,
        };
        match action {
            Err(code) => Err(code),
            Ok(None) => Ok(()),
            Ok(Some((action, expected, enrollment_key))) => {
                let (reply, result) = oneshot::channel();
                match commands.try_send(Mutation {
                    action,
                    expected,
                    id: request.request_id,
                    reply,
                    enrollment_key,
                }) {
                    Ok(()) => result.await.unwrap_or(Err(ErrorCode::Stopping)),
                    Err(mpsc::error::TrySendError::Full(_)) => Err(ErrorCode::Busy),
                    Err(mpsc::error::TrySendError::Closed(_)) => Err(ErrorCode::Stopping),
                }
            }
        }
    };
    let response = Response {
        version: request.version,
        request_id: request.request_id,
        result: match result {
            Ok(()) => ResultBody::Status {
                status: Box::new(view.status()),
            },
            Err(code) => ResultBody::Error { code },
        },
    };
    let _ = meshway_protocol::ipc::write(stream, &response).await;
}

async fn ping_stream(
    stream: impl Connection,
    request: Request,
    mut events: mpsc::Receiver<PingEvent>,
    mut stop: watch::Receiver<bool>,
) {
    let (mut reader, mut writer) = tokio::io::split(stream);
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
            written = tokio::time::timeout(WRITE_TIMEOUT, meshway_protocol::ipc::write(&mut writer, &response)) => written,
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
    mut connections: mpsc::Receiver<Accepted>,
    channels: Channels,
    view: View,
    mut stop: watch::Receiver<bool>,
) -> Result<(), &'static str> {
    #[cfg(test)]
    if FAIL_SERVE.take() {
        drop(channels);
        return Err("IPC service failed.");
    }
    let mut clients = JoinSet::new();
    let outcome = loop {
        tokio::select! { biased;
            _ = async { let _ = stop.wait_for(|stopping| *stopping).await; } => break Ok(()),
            Some(_) = clients.join_next(), if !clients.is_empty() => {},
            accepted = connections.recv() => {
                let Some((stream, slot)) = accepted else { break Ok(()); };
                if clients.len() >= 32 {
                    continue;
                }
                let channels = channels.clone();
                let view = view.clone();
                let stop = stop.clone();
                clients.spawn(async move {
                    let _slot = slot;
                    handle(stream, channels, view, stop).await;
                });
            },
        }
    };
    clients.shutdown().await;
    outcome
}

#[cfg(test)]
thread_local! {
    /// The next service on this thread fails at once, dropping its request
    /// senders, as a listener that stopped unexpectedly.
    pub(crate) static FAIL_SERVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
mod tests;
