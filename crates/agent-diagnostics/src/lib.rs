//! Private local daemon IPC. This is a bounded, single-request protocol, not HTTP.
#![cfg(unix)]
use std::{
    fs::{self, File, Metadata, Permissions},
    io,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixDatagram, UnixListener, UnixStream},
    task::JoinSet,
    time::{self, Instant},
};

use std::{future::Future, pin::Pin, sync::Arc};

pub trait Provider: Send + Sync {
    fn live_status(&self, peers: bool, offset: u32, limit: u32) -> Result<LiveStatus, ProbeError>;
    fn whois(&self, query: &str) -> Result<WhoisObservation, ProbeError>;
    fn probe_neighbor_observed(
        &self,
        request: ProbeRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProbeObservation, ProbeError>> + Send + '_>>;
    fn dns_status(&self) -> dns::Status {
        dns::Status::unavailable()
    }
}

pub mod dns;

const MAX_FRAME: usize = 4096;
const MAX_RESPONSE: usize = 65_536;
pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_PEERS: u32 = 64;
const MAX_CONNECTIONS: usize = 4;
const CONNECTION_DEADLINE: Duration = Duration::from_secs(12);
const STALE_CHECK_DEADLINE: Duration = Duration::from_millis(250);

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeRequest {
    pub peer: String,
    pub timeout_ms: u64,
}

/// Versioned, closed local IPC. The unversioned legacy probe remains accepted.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema_version: u32,
    pub operation: Operation,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    DnsStatus {},
    Status {
        peers: bool,
        offset: u32,
        limit: u32,
    },
    Whois {
        query: String,
    },
    Ping {
        request: ProbeRequest,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum Incoming {
    Versioned(Request),
    Legacy(ProbeRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicIdentity {
    pub node_id: String,
    pub name: String,
    pub mesh_ipv4: String,
    pub mesh_ipv6: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedChannel {
    pub peer_node_id: String,
    pub initiator_node_id: String,
    pub transport: meshway_protocol::ConnectionTransport,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveState {
    Installed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    CurrentAuthenticatedChannelRegistry,
    CurrentTrustedInventory,
    DirectAuthenticatedNeighborRoundTrip,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveStatus {
    pub schema_version: u32,
    pub state: LiveState,
    pub generation: u64,
    pub installed_revision: u32,
    /// Fresh accepted source revision; a semantic-equivalent renewal does not
    /// replace the runtime originally installed at installed_revision.
    pub source_revision: u32,
    pub lease_remaining_ms: u64,
    pub local_node_id: String,
    pub local_identity: Option<PublicIdentity>,
    /// Network ID is not present in the manager's public metadata.
    pub network_id: Option<String>,
    pub authenticated_channel_count: u32,
    /// Omitted by default; an explicit bounded page when requested.
    pub peers: Option<Vec<AuthenticatedChannel>>,
    pub peer_offset: u32,
    pub next_peer_offset: Option<u32>,
    pub evidence: Evidence,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WhoisObservation {
    pub schema_version: u32,
    pub generation: u64,
    pub installed_revision: u32,
    pub source_revision: u32,
    pub identity: PublicIdentity,
    pub evidence: Evidence,
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "status",
    content = "result",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum LiveResponse {
    DnsStatus(dns::Status),
    Status(LiveStatus),
    Whois(WhoisObservation),
    Ping(ProbeObservation),
    Error(ProbeError),
}

impl ProbeRequest {
    pub fn validate(&self) -> Result<(), ProbeError> {
        if self.peer.trim().is_empty()
            || self.peer.chars().count() > 128
            || !(1000..=10_000).contains(&self.timeout_ms)
        {
            return Err(ProbeError::InvalidRequest);
        }
        Ok(())
    }
}

/// Only public identity and metadata from the exact probed channel.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeObservation {
    pub generation: u64,
    pub installed_revision: u32,
    pub source_revision: u32,
    pub evidence: Evidence,
    pub peer_node_id: String,
    pub peer_name: Option<String>,
    pub local_node_id: String,
    pub initiator_node_id: String,
    pub transport: meshway_protocol::ConnectionTransport,
    pub rtt_ns: u64,
}

/// Historical unversioned wire shape. Keep its exact six fields: old clients
/// deny unknown fields. Generation evidence is available only via versioned IPC.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyProbeObservation {
    pub peer_node_id: String,
    pub peer_name: Option<String>,
    pub local_node_id: String,
    pub initiator_node_id: String,
    pub transport: meshway_protocol::ConnectionTransport,
    pub rtt_ns: u64,
}

impl From<ProbeObservation> for LegacyProbeObservation {
    fn from(observation: ProbeObservation) -> Self {
        Self {
            peer_node_id: observation.peer_node_id,
            peer_name: observation.peer_name,
            local_node_id: observation.local_node_id,
            initiator_node_id: observation.initiator_node_id,
            transport: observation.transport,
            rtt_ns: observation.rtt_ns,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum ProbeError {
    #[error(
        "managed daemon unavailable (missing, stopped, lease expired, or configuration withdrawn); run with --apply-connections"
    )]
    Unavailable,
    #[error(
        "peer is not a configured direct neighbor; use its exact inventory name or full 64-character node ID; no automatic routing via other nodes"
    )]
    UnconfiguredPeer,
    #[error("ambiguous inventory name; use the full 64-character node ID")]
    AmbiguousPeer,
    #[error(
        "authenticated neighbor channel unavailable; connection may be pending or authentication/transport failed"
    )]
    ChannelUnavailable,
    #[error("authenticated neighbor probe failed authentication")]
    AuthenticationFailed,
    #[error("authenticated neighbor probe timed out")]
    Timeout,
    #[error("neighbor probe busy or local request limit reached; retry shortly")]
    Busy,
    #[error("invalid local probe request or response")]
    InvalidRequest,
    #[error("private control socket security check failed")]
    Security,
    #[error("identity not found in the current trusted inventory")]
    UnknownIdentity,
    #[error(
        "daemon local IPC version/operation unsupported or response incompatible; upgrade and restart the managed daemon"
    )]
    UnsupportedVersion,
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "status",
    content = "result",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ProbeResponse {
    Ok(LegacyProbeObservation),
    Error(ProbeError),
}

/// Same state path spelling as the daemon, with a fixed private sidecar suffix.
pub fn socket_path(state: &Path) -> PathBuf {
    let mut path = state.as_os_str().to_owned();
    path.push(".control.sock");
    path.into()
}

fn parent_guard(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let metadata = fs::symlink_metadata(parent)?;
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(io::Error::other(
            "control socket directory must be owned by the current user, non-symlink and private (0700)",
        ));
    }
    Ok(())
}

fn socket_guard(path: &Path) -> io::Result<Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_socket() || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(io::Error::other(
            "refusing unrelated control socket path (must be a current-user Unix socket, never a regular file or symlink)",
        ));
    }
    Ok(metadata)
}

fn peer_guard(stream: &UnixStream) -> Result<(), ProbeError> {
    let credentials = stream.peer_cred().map_err(|_| ProbeError::Security)?;
    if credentials.uid() != rustix::process::geteuid().as_raw() {
        return Err(ProbeError::Security);
    }
    Ok(())
}

struct OwnedSocket {
    path: PathBuf,
    metadata: Metadata,
}

impl Drop for OwnedSocket {
    fn drop(&mut self) {
        if let Ok(current) = socket_guard(&self.path)
            && current.dev() == self.metadata.dev()
            && current.ino() == self.metadata.ino()
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub struct Server {
    listener: UnixListener,
    _owned: OwnedSocket,
}

impl Server {
    /// Called only by an Agent already holding its exclusive state lock. The
    /// client never opens the identity, private key, or state lock.
    pub async fn bind(state: &Path, _held_state_lock: &File) -> io::Result<Self> {
        let path = socket_path(state);
        parent_guard(&path)?;
        #[cfg(target_os = "macos")]
        let max_path = 103;
        #[cfg(not(target_os = "macos"))]
        let max_path = 107;
        if path.as_os_str().as_encoded_bytes().len() > max_path {
            return Err(io::Error::other(
                "control socket path is too long for Unix sockets; use a shorter state directory",
            ));
        }
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                let metadata = socket_guard(&path)?;
                // Tokio creates a nonblocking socket. A full accept backlog can
                // yield WouldBlock or remain pending; neither proves staleness.
                // Bound this startup check independently of request deadlines.
                let refused = matches!(
                    time::timeout(STALE_CHECK_DEADLINE, UnixStream::connect(&path)).await,
                    Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionRefused
                );
                // macOS also returns ConnectionRefused for a live full backlog.
                // A nonblocking datagram connect sends no data and distinguishes
                // a live stream socket (protocol mismatch) from a stale inode
                // (ConnectionRefused), without entering its accept backlog.
                let verified_stale = refused
                    && matches!(
                        UnixDatagram::unbound()?.connect(&path),
                        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused
                    );
                if !verified_stale {
                    return Err(io::Error::other(
                        "control socket is already active or cannot be verified stale",
                    ));
                }
                // Only the held daemon state lock permits stale socket removal.
                drop(OwnedSocket {
                    path: path.clone(),
                    metadata,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(&path)?;
        let owned = OwnedSocket {
            metadata: socket_guard(&path)?,
            path,
        };
        fs::set_permissions(&owned.path, Permissions::from_mode(0o600))?;
        Ok(Self {
            listener,
            _owned: owned,
        })
    }

    pub async fn serve(&self, provider: Arc<dyn Provider>) -> io::Result<()> {
        let mut tasks = JoinSet::new();
        let mut since = Instant::now();
        let mut accepted = 0_u8;
        loop {
            tokio::select! {
                biased;
                _ = tasks.join_next(), if !tasks.is_empty() => {},
                incoming = self.listener.accept() => {
                    let (mut socket, _) = incoming?;
                    if peer_guard(&socket).is_err() { continue; }
                    if since.elapsed() >= Duration::from_secs(1) {
                        since = Instant::now();
                        accepted = 0;
                    }
                    if tasks.len() >= MAX_CONNECTIONS || accepted >= 8 {
                        // No queued task/permit for overload. Wait briefly for
                        // socket write readiness so the caller can see Busy.
                        let _ = time::timeout(Duration::from_millis(50),
                            write_frame(&mut socket, &ProbeResponse::Error(ProbeError::Busy))).await;
                        continue;
                    }
                    accepted += 1;
                    let prober = Some(provider.clone());
                    let dns = provider.clone();
                    tasks.spawn(async move {
                        let _ = time::timeout(CONNECTION_DEADLINE, async {
                            match read_frame::<Incoming>(&mut socket).await {
                                Ok(Incoming::Legacy(request)) => {
                                    let Some(prober) = prober else {
                                        let _ = write_frame(&mut socket, &ProbeResponse::Error(ProbeError::Unavailable)).await;
                                        return;
                                    };
                                    let mut extra = [0; 1];
                                    let response = tokio::select! {
                                        biased;
                                        // EOF cancels pending core work. Additional bytes
                                        // are never interpreted as another operation.
                                        _ = socket.read(&mut extra) => return,
                                        result = prober.probe_neighbor_observed(request) => match result {
                                            Ok(value) => ProbeResponse::Ok(value.into()),
                                            Err(error) => ProbeResponse::Error(error),
                                        },
                                    };
                                    let _ = write_frame(&mut socket, &response).await;
                                }
                                Ok(Incoming::Versioned(request)) => {
                                    let response = if request.schema_version != SCHEMA_VERSION {
                                        LiveResponse::Error(ProbeError::UnsupportedVersion)
                                    } else {
                                        match request.operation {
                                            Operation::DnsStatus {} => LiveResponse::DnsStatus(dns.dns_status()),
                                            Operation::Status { peers, offset, limit } => match prober.as_ref().ok_or(ProbeError::Unavailable).and_then(|prober| prober.live_status(peers, offset, limit)) {
                                                Ok(value) => LiveResponse::Status(value),
                                                Err(error) => LiveResponse::Error(error),
                                            },
                                            Operation::Whois { query } => match prober.as_ref().ok_or(ProbeError::Unavailable).and_then(|prober| prober.whois(&query)) {
                                                Ok(value) => LiveResponse::Whois(value),
                                                Err(error) => LiveResponse::Error(error),
                                            },
                                            Operation::Ping { request } => {
                                                let Some(prober) = prober else {
                                                    let _ = write_response(&mut socket, &LiveResponse::Error(ProbeError::Unavailable)).await;
                                                    return;
                                                };
                                                let mut extra = [0; 1];
                                                tokio::select! {
                                                    biased;
                                                    _ = socket.read(&mut extra) => return,
                                                    result = prober.probe_neighbor_observed(request) => match result {
                                                        Ok(value) => LiveResponse::Ping(value),
                                                        Err(error) => LiveResponse::Error(error),
                                                    },
                                                }
                                            },
                                        }
                                    };
                                    let _ = write_response(&mut socket, &response).await;
                                }
                                Err(error) => { let _ = write_frame(&mut socket, &ProbeResponse::Error(error)).await; }
                            }
                        }).await;
                    });
                }
            }
        }
    }
}

// Four-byte big-endian length followed by exactly one JSON value. Length is
// checked before allocation; fragmented headers/payloads are handled by read_exact.
fn frame(value: &impl Serialize) -> Result<Vec<u8>, ProbeError> {
    bounded_frame(value, MAX_FRAME)
}

fn bounded_frame(value: &impl Serialize, limit: usize) -> Result<Vec<u8>, ProbeError> {
    let body = serde_json::to_vec(value).map_err(|_| ProbeError::InvalidRequest)?;
    if body.is_empty() || body.len() > limit {
        return Err(ProbeError::InvalidRequest);
    }
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend(body);
    Ok(bytes)
}

async fn write_frame(socket: &mut UnixStream, value: &impl Serialize) -> Result<(), ProbeError> {
    socket
        .write_all(&frame(value)?)
        .await
        .map_err(|_| ProbeError::Unavailable)
}

async fn read_frame<T: DeserializeOwned>(socket: &mut UnixStream) -> Result<T, ProbeError> {
    read_bounded_frame(socket, MAX_FRAME).await
}

async fn write_response(socket: &mut UnixStream, value: &LiveResponse) -> Result<(), ProbeError> {
    // An oversized response is an explicit error, never a silently truncated page.
    let bytes = bounded_frame(value, MAX_RESPONSE).or_else(|_| {
        bounded_frame(
            &LiveResponse::Error(ProbeError::InvalidRequest),
            MAX_RESPONSE,
        )
    })?;
    socket
        .write_all(&bytes)
        .await
        .map_err(|_| ProbeError::Unavailable)
}

async fn read_bounded_frame<T: DeserializeOwned>(
    socket: &mut UnixStream,
    limit: usize,
) -> Result<T, ProbeError> {
    let size = socket
        .read_u32()
        .await
        .map_err(|_| ProbeError::Unavailable)? as usize;
    if size == 0 || size > limit {
        return Err(ProbeError::InvalidRequest);
    }
    let mut body = vec![0; size];
    socket
        .read_exact(&mut body)
        .await
        .map_err(|_| ProbeError::Unavailable)?;
    serde_json::from_slice(&body).map_err(|_| ProbeError::InvalidRequest)
}

/// One fresh authenticated neighbor round trip through an already-running daemon.
pub async fn probe(
    state: &Path,
    peer: &str,
    timeout: Duration,
) -> Result<ProbeObservation, ProbeError> {
    let request = ProbeRequest {
        peer: peer.into(),
        timeout_ms: timeout
            .as_millis()
            .try_into()
            .map_err(|_| ProbeError::InvalidRequest)?,
    };
    request.validate()?;
    let response = exchange::<LiveResponse>(
        state,
        &Request {
            schema_version: SCHEMA_VERSION,
            operation: Operation::Ping { request },
        },
        MAX_RESPONSE,
    )
    .await
    .map_err(|error| match error {
        ProbeError::InvalidRequest => ProbeError::UnsupportedVersion,
        error => error,
    })?;
    match response {
        LiveResponse::Ping(value) => Ok(value),
        // Old daemons reject the versioned request with the historical error.
        // Never fabricate current metadata or silently fall back to legacy IPC.
        LiveResponse::Error(ProbeError::InvalidRequest) => Err(ProbeError::UnsupportedVersion),
        LiveResponse::Error(error) => Err(error),
        _ => Err(ProbeError::UnsupportedVersion),
    }
}

pub async fn live_status(
    state: &Path,
    peers: bool,
    offset: u32,
    limit: u32,
) -> Result<LiveStatus, ProbeError> {
    if !(1..=MAX_PEERS).contains(&limit) || (!peers && offset != 0) {
        return Err(ProbeError::InvalidRequest);
    }
    match exchange::<LiveResponse>(
        state,
        &Request {
            schema_version: SCHEMA_VERSION,
            operation: Operation::Status {
                peers,
                offset,
                limit,
            },
        },
        MAX_RESPONSE,
    )
    .await?
    {
        LiveResponse::Status(value)
            if value.schema_version == SCHEMA_VERSION
                && value
                    .peers
                    .as_ref()
                    .is_none_or(|peers| peers.len() <= limit as usize) =>
        {
            Ok(value)
        }
        LiveResponse::Error(error) => Err(error),
        _ => Err(ProbeError::InvalidRequest),
    }
}

pub async fn whois(state: &Path, query: &str) -> Result<WhoisObservation, ProbeError> {
    if query.is_empty() || query.chars().count() > 128 {
        return Err(ProbeError::InvalidRequest);
    }
    match exchange::<LiveResponse>(
        state,
        &Request {
            schema_version: SCHEMA_VERSION,
            operation: Operation::Whois {
                query: query.into(),
            },
        },
        MAX_RESPONSE,
    )
    .await?
    {
        LiveResponse::Whois(value) if value.schema_version == SCHEMA_VERSION => Ok(value),
        LiveResponse::Error(error) => Err(error),
        _ => Err(ProbeError::InvalidRequest),
    }
}

async fn exchange<T: DeserializeOwned>(
    state: &Path,
    request: &impl Serialize,
    response_limit: usize,
) -> Result<T, ProbeError> {
    // Validate the outbound bound before connecting. An invalid request must
    // never become a silent write failure followed by a deadline wait.
    let request = frame(request)?;
    let path = socket_path(state);
    parent_guard(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ProbeError::Unavailable
        } else {
            ProbeError::Security
        }
    })?;
    let metadata = socket_guard(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ProbeError::Unavailable
        } else {
            ProbeError::Security
        }
    })?;
    if metadata.mode() & 0o777 != 0o600 {
        return Err(ProbeError::Security);
    }
    time::timeout(CONNECTION_DEADLINE + Duration::from_secs(1), async {
        let mut socket = UnixStream::connect(&path)
            .await
            .map_err(|_| ProbeError::Unavailable)?;
        peer_guard(&socket)?;
        // An overloaded server may send Busy and close before our write; still
        // read its bounded response instead of disguising overload as absence.
        let _ = socket.write_all(&request).await;
        read_bounded_frame(&mut socket, response_limit).await
    })
    .await
    .map_err(|_| ProbeError::Timeout)?
}

#[cfg(test)]
mod tests;
