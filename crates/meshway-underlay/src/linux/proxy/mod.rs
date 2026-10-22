//! Fixed-origin opaque byte relay. TLS is performed by reqwest over the UDS.
use meshway_private_storage::Directory;
use serde::{Deserialize, Serialize};
use std::{
    io,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    net::{UnixListener, UnixStream},
    sync::Semaphore,
};

use super::{dns::Resolver, protection::Shared, sockets::connect_tcp};
use crate::{Error, Result};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    /// Written for the record's format only: a filesystem may be renumbered
    /// when the same state volume is mounted again (Docker engine or host
    /// restart). The socket must be on the state directory's current device.
    device: u64,
    inode: u64,
}

pub(super) struct Proxy {
    path: PathBuf,
    socket: Arc<Socket>,
    /// Stops the listener owner with this Proxy.
    stop: tokio_util::sync::CancellationToken,
}

/// Where the relay's listener owner publishes whether it serves every new
/// connection (`None`) or not; the Host keeps it across terms.
pub(super) type State = tokio::sync::watch::Sender<Option<crate::ControlRelayState>>;

impl Proxy {
    pub async fn open(
        directory: Arc<Directory>,
        origin: &str,
        shared: Arc<Shared>,
        resolver: Resolver,
        limit: usize,
        timeout: Duration,
        state: State,
    ) -> Result<Self> {
        let url = reqwest::Url::parse(origin).map_err(|_| Error::Origin)?;
        let host = url
            .host_str()
            .ok_or(Error::Origin)?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        let port = url.port_or_known_default().ok_or(Error::Origin)?;
        let path = directory.path().join(crate::files::BOOTSTRAP_SOCKET);
        let uid = std::fs::metadata(directory.path())?.uid();
        let socket = Arc::new(Socket {
            directory,
            path: path.clone(),
            uid,
            state: Default::default(),
        });
        // No accept/network work starts before the record is durable. A
        // socket that cannot listen yet is the owner's to retry, at process
        // start as in a re-proved term (see [`serve`]).
        let first = socket.bind().await;
        let stop = tokio_util::sync::CancellationToken::new();
        let tasks = shared.tasks.clone();
        let connections = Connections {
            tasks: tasks.clone(),
            permits: Arc::new(Semaphore::new(limit)),
            uid,
            refused: 0,
            state,
            relay: move |stream| {
                let (host, shared, resolver) = (host.clone(), shared.clone(), resolver.clone());
                async move {
                    if let Err(error) = relay(stream, &host, port, shared, resolver, timeout).await
                    {
                        tracing::warn!(error = %error, "Protected control relay failed");
                    }
                }
            },
        };
        let owner = serve(socket.clone(), first, connections);
        let stopped = stop.clone();
        tasks.spawn(async move {
            tokio::select! { biased;
                _ = stopped.cancelled() => {},
                () = owner => {},
            }
        })?;
        Ok(Self { path, socket, stop })
    }

    pub fn client_builder(&self) -> Result<reqwest::ClientBuilder> {
        let tls = meshway_transport_stream::tls::platform_client_config()
            .map_err(|error| Error::Io(std::io::Error::other(error)))?;
        Ok(reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .no_proxy()
            .unix_socket(self.path.clone()))
    }
}

/// Happy Eyeballs (RFC 8305): a connection attempt that has not completed
/// after this delay does not hold back the next address.
const ATTEMPT_DELAY: Duration = Duration::from_millis(250);

/// The resolver's order with address families alternating, starting with
/// the family of its first answer: an unusable family (no route, or a
/// blackhole that only times out) cannot delay the other.
fn interleave(addresses: Vec<std::net::SocketAddr>) -> Vec<std::net::SocketAddr> {
    let first_v6 = addresses.first().is_some_and(|address| address.is_ipv6());
    let (mut v6, mut v4): (std::collections::VecDeque<_>, std::collections::VecDeque<_>) =
        addresses.into_iter().partition(|address| address.is_ipv6());
    let mut ordered = Vec::with_capacity(v6.len() + v4.len());
    let mut take_v6 = first_v6;
    while !v6.is_empty() || !v4.is_empty() {
        let next = if take_v6 {
            v6.pop_front().or_else(|| v4.pop_front())
        } else {
            v4.pop_front().or_else(|| v6.pop_front())
        };
        ordered.extend(next);
        take_v6 = !take_v6;
    }
    ordered
}

async fn relay(
    local: UnixStream,
    host: &str,
    port: u16,
    shared: Arc<Shared>,
    resolver: Resolver,
    timeout: Duration,
) -> Result<()> {
    // Only a path change can leave this connection on a dead path; a
    // refresh (another interface, a temporary address) keeps it.
    let mut changes = shared.network.changes.subscribe();
    let path = changes.borrow_and_update().path;
    tokio::select! { biased;
        _ = changes.wait_for(|change| change.path != path) => {
            tracing::debug!("Closing control connection after underlay path change");
            Ok(())
        },
        result = relay_current(local, host, port, shared, resolver, timeout) => result,
    }
}

async fn relay_current(
    local: UnixStream,
    host: &str,
    port: u16,
    shared: Arc<Shared>,
    resolver: Resolver,
    timeout: Duration,
) -> Result<()> {
    let scope = shared.bootstrap();
    let establish = async {
        let addresses = resolver.resolve(host, port).await?;
        let (options, tcp) = (Default::default(), Default::default());
        let connect = |address| connect_tcp(scope.clone(), address, None, &options, &tcp, timeout);
        let mut queue = interleave(addresses).into_iter();
        let mut attempts = futures_util::stream::FuturesUnordered::new();
        attempts.extend(queue.next().map(connect));
        let mut last = Error::Resolver;
        while !attempts.is_empty() {
            let stagger = tokio::time::sleep(ATTEMPT_DELAY);
            tokio::select! { biased;
                Some(result) = futures_util::StreamExt::next(&mut attempts) => match result {
                    Ok(stream) => return Ok(stream),
                    Err(error) => {
                        scope.check()?;
                        // Protection/capacity failures are terminal, never a retry without protection.
                        if !matches!(error, Error::Io(_)) {
                            return Err(error);
                        }
                        last = error;
                        attempts.extend(queue.next().map(connect));
                    }
                },
                _ = stagger, if queue.len() != 0 => attempts.extend(queue.next().map(connect)),
            }
        }
        Err(last)
    };
    let remote = tokio::select! { biased;
        _ = scope.ended() => return Err(Error::Conflict),
        result = tokio::time::timeout(timeout, establish) => result.map_err(|_| Error::Io(io::ErrorKind::TimedOut.into()))??,
    };
    // The route toward this address is part of the path fingerprint.
    if let Ok(peer) = remote.peer_addr() {
        shared.network.control_reached(peer.ip());
    }
    // Fixed per-direction copy buffers; a connection quiet in both directions
    // for IDLE closes, and reqwest establishes a fresh protected one. Quiet,
    // not age: a request in flight (a sync long poll holds up to ~55 s with
    // no bytes) is never cut by the relay's own clock.
    let activity = Activity::default();
    let mut local = Active {
        inner: local,
        activity: &activity,
    };
    let mut remote = Active {
        inner: remote,
        activity: &activity,
    };
    tokio::select! { biased;
        _ = scope.ended() => Err(Error::Conflict),
        _ = activity.quiet(IDLE) => Ok(()),
        result = tokio::io::copy_bidirectional_with_sizes(&mut local, &mut remote, 8192, 8192) => {
            result?;
            Ok(())
        },
    }
}

/// A relayed connection with no byte in either direction this long closes.
/// Longer than any request the Control client holds open.
const IDLE: Duration = Duration::from_secs(120);

/// When a byte last crossed the relay in either direction.
struct Activity(std::sync::Mutex<tokio::time::Instant>);
impl Default for Activity {
    fn default() -> Self {
        Self(std::sync::Mutex::new(tokio::time::Instant::now()))
    }
}
impl Activity {
    fn touch(&self) {
        if let Ok(mut last) = self.0.lock() {
            *last = tokio::time::Instant::now();
        }
    }
    async fn quiet(&self, idle: Duration) {
        loop {
            let Ok(last) = self.0.lock().map(|last| *last) else {
                return;
            };
            if tokio::time::Instant::now() >= last + idle {
                return;
            }
            tokio::time::sleep_until(last + idle).await;
        }
    }
}

/// One side of the relay, recording every byte it delivers.
struct Active<'a, S> {
    inner: S,
    activity: &'a Activity,
}
impl<S: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for Active<'_, S> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let before = buffer.filled().len();
        let result = std::pin::Pin::new(&mut self.inner).poll_read(cx, buffer);
        if buffer.filled().len() > before {
            self.activity.touch();
        }
        result
    }
}
impl<S: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for Active<'_, S> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        std::pin::Pin::new(&mut self.inner).poll_write(cx, bytes)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

async fn recover(directory: &Directory, path: &std::path::Path, uid: u32) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    // Reclaiming needs a gone predecessor (connect refused) and a continuous
    // identity (same inode). The device number is not stable across mounts
    // and boots, so it never vetoes. Without a record, the private directory
    // under its lock is the ownership proof.
    let record: Option<Record> = match directory.file(crate::files::BOOTSTRAP_RECORD, false)? {
        Some(file) => {
            let mut bytes = Vec::new();
            tokio::fs::File::from_std(file)
                .take(1025)
                .read_to_end(&mut bytes)
                .await?;
            if bytes.len() > 1024 {
                return Err(Error::Journal);
            }
            Some(serde_json::from_slice(&bytes).map_err(|_| Error::Journal)?)
        }
        None => None,
    };
    let device = std::fs::metadata(directory.path())?.dev();
    if !matches(&metadata, record.as_ref(), uid, device) {
        return Err(foreign());
    }
    match tokio::time::timeout(Duration::from_millis(200), UnixStream::connect(path)).await {
        Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionRefused => {}
        _ => return Err(Error::Conflict),
    }
    let current = std::fs::symlink_metadata(path)?;
    if !matches(&current, record.as_ref(), uid, device) || current.ino() != metadata.ino() {
        return Err(foreign());
    }
    if record.is_none() {
        tracing::warn!(
            path = %path.display(),
            "Reclaiming an unrecorded stale bootstrap socket in the private state directory"
        );
    }
    std::fs::remove_file(path)?;
    directory.sync()?;
    Ok(())
}

/// A socket on the relay's path this owner cannot prove its own (another
/// type, owner, mode or inode than it bound and recorded): a permission it
/// lacks, proven, never reclaimed. A live listener on the path is not proof
/// (the ended term's own relay may still be closing): `Conflict`, retried.
fn foreign() -> Error {
    Error::Io(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "a socket on the relay path this owner cannot prove its own",
    ))
}

fn matches(metadata: &std::fs::Metadata, record: Option<&Record>, uid: u32, device: u64) -> bool {
    metadata.file_type().is_socket()
        && metadata.dev() == device
        && record.is_none_or(|record| metadata.ino() == record.inode)
        && metadata.uid() == uid
        && metadata.mode() & 0o7777 == 0o600
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.cancel();
        self.socket.close();
    }
}

/// The relay's UDS. It binds only after reclaiming its own stale socket,
/// records the inode it bound, and removes only that inode.
struct Socket {
    directory: Arc<Directory>,
    path: PathBuf,
    uid: u32,
    state: std::sync::Mutex<Bound>,
}

#[derive(Default)]
struct Bound {
    record: Option<Record>,
    /// The Proxy was dropped: nothing binds again.
    closed: bool,
}

impl Socket {
    async fn bind(&self) -> Result<UnixListener> {
        recover(&self.directory, &self.path, self.uid).await?;
        let (listener, bytes) = {
            let mut bound = self.state.lock().map_err(|_| Error::Worker)?;
            if bound.closed {
                return Err(Error::Worker);
            }
            let listener = UnixListener::bind(&self.path)?;
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))?;
            let metadata = std::fs::symlink_metadata(&self.path)?;
            let record = Record {
                device: metadata.dev(),
                inode: metadata.ino(),
            };
            let bytes = serde_json::to_vec(&record).map_err(|_| Error::Journal)?;
            bound.record = Some(record);
            (listener, bytes)
        };
        // Accepted replacement owns its directory lock even if the caller
        // cancels.
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || {
            directory.replace(crate::files::BOOTSTRAP_RECORD, &bytes)
        })
        .await
        .map_err(|_| Error::Worker)??;
        Ok(listener)
    }

    fn close(&self) {
        let Ok(mut bound) = self.state.lock() else {
            return;
        };
        bound.closed = true;
        let device = std::fs::metadata(self.directory.path()).map(|meta| meta.dev());
        if let (Ok(device), Some(record)) = (device, bound.record.as_ref())
            && std::fs::symlink_metadata(&self.path)
                .is_ok_and(|metadata| matches(&metadata, Some(record), self.uid, device))
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// The relay listener's recovery, the U1 listener cadence: a cause that can
/// clear by itself (`EADDRINUSE`, a live or not yet reclaimable socket, a
/// stalled accept) is retried 1 s after it, doubling to 60 s, without a
/// total limit; one that needs an operator (a permission, `EACCES`) is
/// proven and read again every 60 s.
fn relay_recovery() -> meshway_model::recovery::Recovery {
    const LIMIT: Duration = Duration::from_secs(60);
    meshway_model::recovery::Recovery::new(meshway_model::recovery::Policy {
        backoff: meshway_model::Backoff::new(Duration::from_secs(1), LIMIT),
        horizon: meshway_model::recovery::Horizon::Unbounded,
        reread_blocked: Some(LIMIT),
    })
}

/// The relay owner's judgement of a failed listen or accept.
fn judged(operation: &'static str, error: &Error) -> meshway_model::recovery::Observation {
    let reason = format!("{operation}: {}", describe(error));
    match error {
        Error::Io(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            meshway_model::recovery::Observation::proven("relay_permission", reason)
        }
        _ => meshway_model::recovery::Observation::unknown("relay_unavailable", reason),
    }
}

/// Record one failed listen or accept: how long the owner waits.
fn failed(
    recovery: &mut meshway_model::recovery::Recovery,
    operation: &'static str,
    error: &Error,
) -> Duration {
    let now = std::time::Instant::now();
    match recovery.observe(judged(operation, error), now) {
        meshway_model::recovery::Step::RetryAt(at)
        | meshway_model::recovery::Step::Block(Some(at)) => at.saturating_duration_since(now),
        _ => Duration::from_secs(60),
    }
}

/// The relay's connections: bounded by its permits and the Host's task
/// capacity. At either limit a new connection is refused (closed) and the
/// current ones are kept; the first connection admitted again ends that.
struct Connections<R> {
    tasks: super::tasks::Tasks,
    permits: Arc<Semaphore>,
    uid: u32,
    relay: R,
    /// Connections refused since the limit was reached.
    refused: u64,
    state: State,
}

enum Admission {
    Admitted,
    Refused,
    /// The Host's tasks stop.
    Stopped,
}

impl<R, F> Connections<R>
where
    R: Fn(UnixStream) -> F,
    F: std::future::Future<Output = ()> + Send + 'static,
{
    fn admit(&mut self, stream: UnixStream) -> Admission {
        let allowed = stream
            .peer_cred()
            .is_ok_and(|credential| credential.uid() == 0 || credential.uid() == self.uid);
        if !allowed {
            return Admission::Refused;
        }
        let Ok(permit) = self.permits.clone().try_acquire_owned() else {
            return self.refuse("connections");
        };
        let relay = (self.relay)(stream);
        match self.tasks.spawn(async move {
            let _permit = permit;
            relay.await;
        }) {
            Ok(()) => {
                if self.refused > 0 {
                    tracing::info!(
                        refused = self.refused,
                        "Protected control relay accepts connections again"
                    );
                    self.refused = 0;
                    self.state.send_replace(None);
                }
                Admission::Admitted
            }
            Err(Error::Capacity) => self.refuse("tasks"),
            Err(_) => Admission::Stopped,
        }
    }

    fn refuse(&mut self, limit: &'static str) -> Admission {
        if self.refused == 0 {
            tracing::warn!(
                limit,
                "Protected control relay at capacity; refusing new connections, keeping current ones"
            );
        }
        self.refused = self.refused.saturating_add(1);
        self.state
            .send_replace(Some(crate::ControlRelayState::AtCapacity {
                limit,
                refused: self.refused,
            }));
        Admission::Refused
    }
}

/// The relay's listener owner. A socket that cannot listen and a failed
/// accept are retried on [`relay_recovery`], logged once per distinct error and
/// published at every attempt; neither ends kernel ownership (the relay's
/// connections already end with it). Returns once the Host's tasks stop or
/// the Proxy closes.
async fn serve<R, F>(
    socket: Arc<Socket>,
    first: Result<UnixListener>,
    mut connections: Connections<R>,
) where
    R: Fn(UnixStream) -> F,
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let mut recovery = relay_recovery();
    let mut reported: Option<String> = None;
    let mut next = first;
    let listener = loop {
        match next {
            Ok(listener) => break listener,
            Err(Error::Worker) => return,
            Err(error) => {
                let delay = failed(&mut recovery, "cannot listen", &error);
                report(
                    &mut reported,
                    &error,
                    recovery.attempts(),
                    delay,
                    "Protected control relay cannot listen; retrying, kernel ownership kept",
                );
                publish(&connections.state, &recovery);
                tokio::time::sleep(delay).await;
                next = socket.bind().await;
            }
        }
    };
    let failures = recovery.attempts();
    if failures > 0 {
        tracing::info!(
            attempts = failures + 1,
            "Protected control relay listening again"
        );
        reported = None;
    }
    recovery.observe(
        meshway_model::recovery::Observation::Holds,
        std::time::Instant::now(),
    );
    connections.state.send_replace(None);
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let failures = recovery.attempts();
                if failures > 0 {
                    tracing::info!(failures, "Protected control relay accepting again");
                    reported = None;
                    recovery.observe(
                        meshway_model::recovery::Observation::Holds,
                        std::time::Instant::now(),
                    );
                    connections.state.send_replace(None);
                }
                if let Admission::Stopped = connections.admit(stream) {
                    return;
                }
            }
            Err(error) => {
                let error = Error::Io(error);
                let delay = failed(&mut recovery, "accept failed", &error);
                report(
                    &mut reported,
                    &error,
                    recovery.attempts(),
                    delay,
                    "Protected control relay accept failed; retrying, kernel ownership kept",
                );
                publish(&connections.state, &recovery);
                tokio::time::sleep(delay).await;
            }
        }
    }
}

/// Why the relay's socket did not listen or accept: a socket on its path
/// that [`recover`] does not reclaim is named as such, not as the Host's
/// ownership conflict the same error kind stands for elsewhere.
fn describe(error: &Error) -> String {
    match error {
        Error::Conflict => {
            "its path is held by a live listener or a socket it cannot reclaim".into()
        }
        error => error.to_string(),
    }
}

/// The relay owner's current retry, as Status shows it.
fn publish(state: &State, recovery: &meshway_model::recovery::Recovery) {
    if let Some(pending) = recovery.health().pending() {
        state.send_replace(Some(if recovery.health().is_blocked() {
            crate::ControlRelayState::Blocked(pending)
        } else {
            crate::ControlRelayState::Retrying(pending)
        }));
    }
}

fn report(
    reported: &mut Option<String>,
    error: &Error,
    attempts: u32,
    delay: Duration,
    message: &'static str,
) {
    let text = describe(error);
    if reported.as_ref() != Some(&text) {
        tracing::warn!(error = %text, attempts, retry_ms = delay.as_millis() as u64, "{message}");
        *reported = Some(text);
    }
}

#[cfg(test)]
mod tests;
