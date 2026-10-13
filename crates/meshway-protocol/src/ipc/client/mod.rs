//! Authenticated Unix client shared by local control consumers.
use crate::ipc::{
    CodecError, Command, ErrorCode, NetcheckReport, PingEvent, PingTarget, Request, Response,
    ResultBody, Status, VERSION,
};
use meshway_model::{ObservationId, RequestId, Sequence};
use std::{
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::PathBuf,
    time::Duration,
};
use tokio::net::UnixStream;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const PING_IDLE_TIMEOUT: Duration = Duration::from_secs(15);
/// Above the daemon's own bound on one selection request (25 s).
const SELECTION_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Cannot create local request ID.")]
    RequestId,
    #[error("Cannot connect to meshwayd; check --socket and local permissions.")]
    Connect(#[source] std::io::Error),
    #[error("Cannot inspect local socket.")]
    Inspect(#[source] std::io::Error),
    #[error("Cannot verify daemon identity.")]
    Peer(#[source] std::io::Error),
    #[error("Local socket identity or permissions differ.")]
    Identity,
    #[error("Local request failed.")]
    Write(#[source] CodecError),
    #[error("Invalid or incomplete daemon response.")]
    Read(#[source] CodecError),
    #[error("Daemon protocol or request identity differs.")]
    Response,
    #[error(transparent)]
    Remote(#[from] ErrorCode),
    #[error("Daemon request timed out; inspect status before retrying.")]
    Timeout,
}

pub struct Client {
    socket: PathBuf,
}

impl Client {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
        }
    }

    /// Submit once. A dropped waiter or a timeout does not cancel daemon intent;
    /// callers must inspect current status before deciding whether to retry.
    pub async fn call(&self, command: Command) -> Result<Status, ClientError> {
        let timeout = if matches!(command, Command::RotateKey { .. } | Command::Logout { .. }) {
            Duration::from_secs(70)
        } else {
            REQUEST_TIMEOUT
        };
        let request = request(command)?;
        tokio::time::timeout(timeout, self.exchange(&request))
            .await
            .map_err(|_| ClientError::Timeout)?
    }

    /// Start exactly one measurement subscription. Dropping the stream closes
    /// its connection, cancelling this subscriber without changing Node intent.
    pub async fn ping(&self, target: PingTarget, count: u16) -> Result<PingStream, ClientError> {
        if !(1..=1000).contains(&count) {
            return Err(ClientError::Response);
        }
        let request = request(Command::Ping { target, count })?;
        tokio::time::timeout(REQUEST_TIMEOUT, async {
            let stream = self.connect_send(&request).await?;
            let mut ping = PingStream {
                stream: Some(stream),
                request_id: request.request_id,
                sequence: Sequence::MIN,
                observation_id: None,
                external: false,
                pending: None,
            };
            ping.pending = ping.next().await?;
            Ok(ping)
        })
        .await
        .map_err(|_| ClientError::Timeout)?
    }

    /// Run one bounded network diagnostic; it changes no Node intent.
    pub async fn netcheck(&self) -> Result<NetcheckReport, ClientError> {
        let request = request(Command::Netcheck)?;
        tokio::time::timeout(REQUEST_TIMEOUT, async {
            let mut stream = self.connect_send(&request).await?;
            let response = read_response(&mut stream, request.request_id).await?;
            match response.result {
                ResultBody::Netcheck { report } => Ok(*report),
                ResultBody::Error { code } => Err(ClientError::Remote(code)),
                _ => Err(ClientError::Response),
            }
        })
        .await
        .map_err(|_| ClientError::Timeout)?
    }

    /// Ask the daemon to have the Server select this Node's routing profile.
    /// The answer is the Server's committed selection; the Node applies it
    /// with its next netmap. A timeout leaves the outcome unknown: inspect
    /// status before retrying.
    pub async fn select_profile(
        &self,
        profile: String,
    ) -> Result<meshway_configuration::selection::RoutingSelectionState, ClientError> {
        let request = request(Command::SelectProfile { profile })?;
        tokio::time::timeout(SELECTION_TIMEOUT, async {
            let mut stream = self.connect_send(&request).await?;
            let response = read_response(&mut stream, request.request_id).await?;
            match response.result {
                ResultBody::Selection { selection } => Ok(*selection),
                ResultBody::Error { code } => Err(ClientError::Remote(code)),
                _ => Err(ClientError::Response),
            }
        })
        .await
        .map_err(|_| ClientError::Timeout)?
    }

    async fn exchange(&self, request: &Request) -> Result<Status, ClientError> {
        let mut stream = self.connect_send(request).await?;
        let response = read_response(&mut stream, request.request_id).await?;
        match response.result {
            ResultBody::Status { status } => Ok(*status),
            ResultBody::Error { code } => Err(ClientError::Remote(code)),
            ResultBody::Ping { .. }
            | ResultBody::Netcheck { .. }
            | ResultBody::Selection { .. } => Err(ClientError::Response),
        }
    }

    async fn connect_send(&self, request: &Request) -> Result<UnixStream, ClientError> {
        let mut stream = UnixStream::connect(&self.socket)
            .await
            .map_err(ClientError::Connect)?;
        // Verify the connected peer against the socket owner, not a UID claimed
        // in the response. The private socket path is chosen by the operator.
        let metadata = std::fs::symlink_metadata(&self.socket).map_err(ClientError::Inspect)?;
        let peer = stream.peer_cred().map_err(ClientError::Peer)?;
        if !metadata.file_type().is_socket()
            || metadata.mode() & 0o7777 != 0o600
            || peer.uid() != metadata.uid()
        {
            return Err(ClientError::Identity);
        }
        crate::ipc::write(&mut stream, request)
            .await
            .map_err(ClientError::Write)?;
        Ok(stream)
    }
}

fn request(command: Command) -> Result<Request, ClientError> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| ClientError::RequestId)?;
    let request_id = RequestId::new(uuid::Builder::from_random_bytes(bytes).into_uuid())
        .map_err(|_| ClientError::RequestId)?;
    Ok(Request {
        version: VERSION,
        request_id,
        command,
    })
}

async fn read_response(
    stream: &mut UnixStream,
    request_id: RequestId,
) -> Result<Response, ClientError> {
    let response: Response = crate::ipc::read(stream).await.map_err(ClientError::Read)?;
    if response.request_id != request_id {
        return Err(ClientError::Response);
    }
    if response.version != VERSION {
        return Err(
            if matches!(
                response.result,
                ResultBody::Error {
                    code: ErrorCode::Version
                }
            ) {
                ClientError::Remote(ErrorCode::Version)
            } else {
                ClientError::Response
            },
        );
    }
    Ok(response)
}

/// One subscription, with bounded per-frame idle waits and no overall deadline.
pub struct PingStream {
    stream: Option<UnixStream>,
    request_id: RequestId,
    sequence: Sequence,
    observation_id: Option<ObservationId>,
    /// The decision sends echo requests outside the Mesh.
    external: bool,
    pending: Option<PingEvent>,
}

impl PingStream {
    /// Return actual typed events. EOF before a terminal event is an error.
    /// Cancelling an in-flight `next` closes the connection; it cannot be resumed
    /// at an ambiguous frame boundary or retried as a second measurement.
    pub async fn next(&mut self) -> Result<Option<PingEvent>, ClientError> {
        if let Some(event) = self.pending.take() {
            return Ok(Some(event));
        }
        let Some(mut stream) = self.stream.take() else {
            return Ok(None);
        };
        let response = tokio::time::timeout(
            PING_IDLE_TIMEOUT,
            read_response(&mut stream, self.request_id),
        )
        .await
        .map_err(|_| ClientError::Timeout)??;
        let (sequence, event) = match response.result {
            ResultBody::Ping { sequence, event } => (sequence, event),
            ResultBody::Error { code } if self.sequence == Sequence::MIN => {
                return Err(ClientError::Remote(code));
            }
            _ => return Err(ClientError::Response),
        };
        if sequence != self.sequence {
            return Err(ClientError::Response);
        }
        match &event {
            // The decision comes first, once; a Mesh Node target is then
            // accepted as a measurement, any other target is echoed.
            PingEvent::Decision { decision } if self.sequence == Sequence::MIN => {
                use crate::ipc::PingAction;
                self.external = matches!(
                    decision.action,
                    Some(PingAction::Exit { .. } | PingAction::Direct | PingAction::Unmanaged)
                ) && decision.node.is_none();
            }
            PingEvent::Accepted { observation_id, .. }
                if self.observation_id.is_none() && !self.external =>
            {
                self.observation_id = Some(*observation_id);
            }
            PingEvent::Failed { .. } => {}
            PingEvent::Preparing { .. } => {}
            PingEvent::Completed { .. } if self.observation_id.is_some() || self.external => {}
            PingEvent::Sample { observation_id, .. }
                if self.observation_id == Some(*observation_id) => {}
            PingEvent::Echo { .. } if self.external => {}
            _ => return Err(ClientError::Response),
        }
        if !matches!(
            event,
            PingEvent::Completed { .. } | PingEvent::Failed { .. }
        ) {
            self.sequence = self
                .sequence
                .checked_next()
                .map_err(|_| ClientError::Response)?;
            self.stream = Some(stream);
        }
        Ok(Some(event))
    }
}

#[cfg(test)]
mod tests;
