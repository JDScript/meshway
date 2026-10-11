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
        let path = directory.path().join("underlay-bootstrap.sock");
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
        tokio::task::spawn_blocking(move || directory.replace("underlay-bootstrap.json", &bytes))
            .await
            .map_err(|_| Error::Worker)??;
        let permits = Arc::new(Semaphore::new(limit));
        let tasks = shared.tasks.clone();
        let worker = shared.clone();
        shared.tasks.spawn(async move {
            loop {
                let accepted = listener.accept().await;
                let Ok((stream, _)) = accepted else {
                    worker.conflict.cancel();
                    break;
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

async fn relay(
    local: UnixStream,
    host: &str,
    port: u16,
    shared: Arc<Shared>,
    resolver: Resolver,
    timeout: Duration,
) -> Result<()> {
    let mut changes = shared.network_changes.subscribe();
    tokio::select! { biased;
        _ = changes.changed() => {
            tracing::debug!("Closing control connection after external network change");
            Ok(())
        },
        result = relay_current(local, host, port, shared, resolver, timeout) => result,
    }
}

async fn relay_current(
    mut local: UnixStream,
    host: &str,
    port: u16,
    shared: Arc<Shared>,
    resolver: Resolver,
    timeout: Duration,
) -> Result<()> {
    let scope = shared.bootstrap();
    let establish = async {
        let addresses = resolver.resolve(host, port).await?;
        let mut last = Error::Resolver;
        for address in addresses {
            match connect_tcp(
                scope.clone(),
                address,
                None,
                &Default::default(),
                &Default::default(),
                timeout,
            )
            .await
            {
                Ok(stream) => return Ok(stream),
                Err(error) => {
                    scope.check()?;
                    if !matches!(error, Error::Io(_)) {
                        return Err(error);
                    }
                    last = error;
                }
            }
        }
        Err(last)
    };
    let mut remote = tokio::select! { biased;
        _ = scope.ended() => return Err(Error::Conflict),
        result = tokio::time::timeout(timeout, establish) => result.map_err(|_| Error::Io(io::ErrorKind::TimedOut.into()))??,
    };
    // Finite pooled-connection lifetime plus fixed per-direction copy buffers.
    // Expiry closes the connection; reqwest can establish a fresh protected one.
    tokio::select! { biased;
        _ = scope.ended() => Err(Error::Conflict),
        result = tokio::time::timeout(Duration::from_secs(300), tokio::io::copy_bidirectional_with_sizes(&mut local, &mut remote, 8192, 8192)) => {
            result.map_err(|_| Error::Io(io::ErrorKind::TimedOut.into()))??;
            Ok(())
        },
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
    let record: Option<Record> = match directory.file("underlay-bootstrap.json", false)? {
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
