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
    record: Record,
    uid: u32,
    _directory: Arc<Directory>,
}

impl Proxy {
    pub async fn open(
        directory: Arc<Directory>,
        origin: &str,
        shared: Arc<Shared>,
        resolver: Resolver,
        limit: usize,
        timeout: Duration,
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
        recover(&directory, &path, uid).await?;
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let metadata = std::fs::symlink_metadata(&path)?;
        let record = Record {
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        let proxy = Self {
            path,
            record,
            uid,
            _directory: directory.clone(),
        };
        let bytes = serde_json::to_vec(&proxy.record).map_err(|_| Error::Journal)?;
        // Accepted replacement owns its directory lock even if startup's caller
        // cancels. No accept/network work starts before durable completion.
        tokio::task::spawn_blocking(move || {
            directory.replace(crate::files::BOOTSTRAP_RECORD, &bytes)
        })
        .await
        .map_err(|_| Error::Worker)??;
        let permits = Arc::new(Semaphore::new(limit));
        let tasks = shared.tasks.clone();
        let worker = shared.clone();
        shared.tasks.spawn(async move {
            loop {
                let accepted = listener.accept().await;
                let (stream, _) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        tracing::warn!(%error, "Protected control relay listener failed; kernel ownership ends");
                        worker.conflict.cancel();
                        break;
                    }
                };
                let allowed = stream
                    .peer_cred()
                    .is_ok_and(|credential| credential.uid() == 0 || credential.uid() == uid);
                if !allowed {
                    continue;
                }
                let Ok(permit) = permits.clone().try_acquire_owned() else {
                    continue;
                };
                let resolver = resolver.clone();
                let shared = worker.clone();
                let host = host.clone();
                if tasks
                    .spawn(async move {
                        let _permit = permit;
                        if let Err(error) =
                            relay(stream, &host, port, shared, resolver, timeout).await
                        {
                            tracing::warn!(error = %error, "Protected control relay failed");
                        }
                    })
                    .is_err()
                {
                    tracing::warn!("Protected control relay task capacity exhausted; kernel ownership ends");
                    worker.conflict.cancel();
                    break;
                }
            }
        })?;
        Ok(proxy)
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
        return Err(Error::Conflict);
    }
    match tokio::time::timeout(Duration::from_millis(200), UnixStream::connect(path)).await {
        Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionRefused => {}
        _ => return Err(Error::Conflict),
    }
    let current = std::fs::symlink_metadata(path)?;
    if !matches(&current, record.as_ref(), uid, device) || current.ino() != metadata.ino() {
        return Err(Error::Conflict);
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

fn matches(metadata: &std::fs::Metadata, record: Option<&Record>, uid: u32, device: u64) -> bool {
    metadata.file_type().is_socket()
        && metadata.dev() == device
        && record.is_none_or(|record| metadata.ino() == record.inode)
        && metadata.uid() == uid
        && metadata.mode() & 0o7777 == 0o600
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let device = std::fs::metadata(self._directory.path()).map(|meta| meta.dev());
        if let Ok(device) = device
            && std::fs::symlink_metadata(&self.path)
                .is_ok_and(|metadata| matches(&metadata, Some(&self.record), self.uid, device))
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn a_held_request_is_not_cut_and_only_quiet_connections_close() {
        // Scaled: 200 ms stands for IDLE.
        let idle = Duration::from_millis(200);
        let activity = Activity::default();
        // A held request: no byte for most of the idle time, then the answer.
        let quiet = activity.quiet(idle);
        tokio::pin!(quiet);
        tokio::select! {
            _ = &mut quiet => panic!("closed while a request was held"),
            _ = tokio::time::sleep(idle * 3 / 4) => activity.touch(),
        }
        // An active connection lives past any fixed age.
        let started = tokio::time::Instant::now();
        let alive = async {
            for _ in 0..10 {
                tokio::time::sleep(idle / 2).await;
                activity.touch();
            }
        };
        tokio::select! {
            _ = activity.quiet(idle) => panic!("an active connection closed"),
            _ = alive => {},
        }
        assert!(started.elapsed() >= idle * 5);
        let quiet_since = tokio::time::Instant::now();
        activity.quiet(idle).await;
        assert!(quiet_since.elapsed() >= idle * 9 / 10);
    }

    #[test]
    fn families_alternate_from_the_resolvers_first_answer() {
        let addresses = |list: &[&str]| -> Vec<std::net::SocketAddr> {
            list.iter()
                .map(|address| address.parse().unwrap())
                .collect()
        };
        assert_eq!(
            super::interleave(addresses(&[
                "[2001:db8::1]:443",
                "[2001:db8::2]:443",
                "192.0.2.1:443",
                "192.0.2.2:443",
            ])),
            addresses(&[
                "[2001:db8::1]:443",
                "192.0.2.1:443",
                "[2001:db8::2]:443",
                "192.0.2.2:443",
            ])
        );
        assert_eq!(
            super::interleave(addresses(&[
                "192.0.2.1:443",
                "192.0.2.2:443",
                "[2001:db8::1]:443"
            ])),
            addresses(&["192.0.2.1:443", "[2001:db8::1]:443", "192.0.2.2:443"])
        );
        assert!(super::interleave(Vec::new()).is_empty());
    }

    fn directory() -> (tempfile::TempDir, Directory, PathBuf, std::fs::Metadata) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().canonicalize().unwrap().join("state");
        let directory = Directory::open(&path, "underlay.lock").unwrap();
        let socket = path.join("underlay-bootstrap.sock");
        drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let metadata = std::fs::symlink_metadata(&socket).unwrap();
        (root, directory, socket, metadata)
    }

    fn record(directory: &Directory, device: u64, inode: u64) {
        let bytes = serde_json::to_vec(&Record { device, inode }).unwrap();
        directory
            .replace("underlay-bootstrap.json", &bytes)
            .unwrap();
    }

    #[tokio::test]
    async fn stale_socket_on_a_renumbered_device_or_without_record_is_reclaimed() {
        let (_root, directory, socket, metadata) = directory();
        let uid = metadata.uid();
        record(&directory, metadata.dev().wrapping_add(1), metadata.ino());
        recover(&directory, &socket, uid).await.unwrap();
        assert!(!socket.exists());

        let (_root, directory, socket, metadata) = self::directory();
        recover(&directory, &socket, metadata.uid()).await.unwrap();
        assert!(!socket.exists());
    }

    #[tokio::test]
    async fn a_different_inode_or_a_live_listener_is_never_reclaimed() {
        let (_root, directory, socket, metadata) = directory();
        let uid = metadata.uid();
        record(&directory, metadata.dev(), metadata.ino().wrapping_add(1));
        assert!(recover(&directory, &socket, uid).await.is_err());
        std::fs::remove_file(&socket).unwrap();
        let live = tokio::net::UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let metadata = std::fs::symlink_metadata(&socket).unwrap();
        record(&directory, metadata.dev(), metadata.ino());
        assert!(recover(&directory, &socket, uid).await.is_err());
        assert!(socket.exists());
        drop(live);
    }
}
