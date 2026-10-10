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
    let file = directory
        .file("underlay-bootstrap.json", false)?
        .ok_or(Error::Journal)?;
    let mut bytes = Vec::new();
    tokio::fs::File::from_std(file)
        .take(1025)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > 1024 {
        return Err(Error::Journal);
    }
    let record: Record = serde_json::from_slice(&bytes).map_err(|_| Error::Journal)?;
    if !matches(&metadata, &record, uid) {
        return Err(Error::Conflict);
    }
    match tokio::time::timeout(Duration::from_millis(200), UnixStream::connect(path)).await {
        Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionRefused => {}
        _ => return Err(Error::Conflict),
    }
    if !matches(&std::fs::symlink_metadata(path)?, &record, uid) {
        return Err(Error::Conflict);
    }
    std::fs::remove_file(path)?;
    directory.sync()?;
    Ok(())
}

fn matches(metadata: &std::fs::Metadata, record: &Record, uid: u32) -> bool {
    metadata.file_type().is_socket()
        && metadata.dev() == record.device
        && metadata.ino() == record.inode
        && metadata.uid() == uid
        && metadata.mode() & 0o7777 == 0o600
}

impl Drop for Proxy {
    fn drop(&mut self) {
        if std::fs::symlink_metadata(&self.path)
            .is_ok_and(|metadata| matches(&metadata, &self.record, self.uid))
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
