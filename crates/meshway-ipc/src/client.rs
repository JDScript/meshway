//! One-shot Unix client shared by local control consumers.
use crate::{CodecError, Command, ErrorCode, Request, Response, ResultBody, Status, VERSION};
use meshway_model::RequestId;
use std::{
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::PathBuf,
    time::Duration,
};
use tokio::net::UnixStream;

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
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|_| ClientError::RequestId)?;
        let request_id = RequestId::new(uuid::Builder::from_random_bytes(bytes).into_uuid())
            .map_err(|_| ClientError::RequestId)?;
        let request = Request {
            version: VERSION,
            request_id,
            command,
        };
        tokio::time::timeout(Duration::from_secs(12), self.exchange(&request))
            .await
            .map_err(|_| ClientError::Timeout)?
    }

    async fn exchange(&self, request: &Request) -> Result<Status, ClientError> {
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
        crate::write(&mut stream, request)
            .await
            .map_err(ClientError::Write)?;
        let response: Response = crate::read(&mut stream).await.map_err(ClientError::Read)?;
        if response.version != VERSION || response.request_id != request.request_id {
            return Err(ClientError::Response);
        }
        match response.result {
            ResultBody::Status { status } => Ok(*status),
            ResultBody::Error { code } => Err(ClientError::Remote(code)),
        }
    }
}
