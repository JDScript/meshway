mod dns;
mod kernel;
mod protection;
mod proxy;
pub mod sockets;
mod tasks;

use meshway_control::ExecutionSnapshot;
use meshway_private_storage::Directory;
use std::sync::Arc;

use crate::{Error, Options, Result};
use kernel::Kernel;
use proxy::Proxy;
use sockets::SocketFactory;
use tasks::Tasks;

/// One actor owns Host. Control clients use its independently driven UDS relay;
/// data factories are cloneable but retain only opaque, revocable socket scope.
/// Drop stops socket tasks and drops TUN while retaining negative kernel rules.
/// Call shutdown to join producers and finish accepted private writes explicitly.
pub struct Host {
    kernel: Kernel,
    proxy: Proxy,
    sockets: SocketFactory,
    tasks: Tasks,
}

impl Host {
    pub async fn open(directory: Arc<Directory>, origin: &str, options: Options) -> Result<Self> {
        options.validate()?;
        let origin = meshway_control::server_origin(origin).map_err(|_| Error::Origin)?;
        let path = directory.path().to_owned();
        // A second Host in the same process must not become a second kernel owner.
        let directory = Arc::new(
            tokio::task::spawn_blocking(move || Directory::open(&path, "underlay.lock"))
                .await
                .map_err(|_| Error::Worker)??,
        );
        let tasks = Tasks::new(options.max_sockets * 4 + 32);
        struct Opening(Option<Tasks>);
        impl Drop for Opening {
            fn drop(&mut self) {
                if let Some(tasks) = &self.0 {
                    tasks.stop.cancel();
                }
            }
        }
        let mut opening = Opening(Some(tasks.clone()));
        let opened = async {
            let kernel = Kernel::open(directory.clone(), options.clone(), tasks.clone()).await?;
            let resolver = dns::Resolver::open(kernel.shared.clone(), &options).await?;
            let proxy = Proxy::open(
                directory,
                &origin,
                kernel.shared.clone(),
                resolver.clone(),
                options.max_sockets,
                options.connect_timeout,
            )
            .await?;
            let sockets = SocketFactory {
                shared: kernel.shared.clone(),
                resolver,
                timeout: options.connect_timeout,
            };
            Ok::<_, Error>((kernel, proxy, sockets))
        }
        .await;
        match opened {
            Ok((kernel, proxy, sockets)) => {
                opening.0 = None;
                Ok(Self {
                    kernel,
                    proxy,
                    sockets,
                    tasks,
                })
            }
            Err(error) => {
                drop(opening);
                tasks.shutdown().await?;
                Err(error)
            }
        }
    }

    pub fn client_builder(&self) -> reqwest::ClientBuilder {
        self.proxy.client_builder()
    }
    pub fn sockets(&self) -> SocketFactory {
        self.sockets.clone()
    }
    pub fn tun(&self) -> Option<&meshway_tun::Tun> {
        if self.kernel.shared.check().is_err() {
            return None;
        }
        let active = self.kernel.shared.execution.read().ok()?;
        let (snapshot, token) = active.as_ref()?;
        if snapshot.ensure_current().is_err() || token.is_cancelled() {
            return None;
        }
        self.kernel.tun.as_ref()
    }

    /// Select this around the actor's TUN/packet loop in addition to the Control
    /// snapshot's invalidation. A foreign kernel mutation stops positive I/O.
    pub async fn invalidated(&self) {
        let snapshot = self
            .kernel
            .shared
            .execution
            .read()
            .ok()
            .and_then(|active| active.as_ref().map(|(snapshot, _)| snapshot.clone()));
        match snapshot {
            Some(snapshot) => {
                if let Ok(scope) = self.kernel.shared.data(&snapshot) {
                    scope.invalidated().await;
                }
            }
            None => self.kernel.shared.bootstrap().invalidated().await,
        }
    }

    /// Install capture under fresh Control authority. Success does not claim
    /// carrier establishment, business DNS installation, or an applied receipt.
    /// A verified direct publication continuation with unchanged capture keeps
    /// the owned TUN and replaces the data-socket epoch. Calling suspend first
    /// deliberately relinquishes that interface and prevents this handoff.
    pub async fn install(&mut self, snapshot: ExecutionSnapshot) -> Result<()> {
        self.kernel.install(snapshot).await
    }
    pub async fn suspend(&mut self) -> Result<()> {
        self.kernel.suspend().await
    }
    pub async fn down(&mut self) -> Result<()> {
        self.kernel.down().await
    }

    pub async fn shutdown(mut self, keep_sink: bool) -> Result<()> {
        let cleanup = if keep_sink {
            self.kernel.suspend().await
        } else {
            self.kernel.down().await
        };
        let storage = self.kernel.settle().await;
        let tasks = self.tasks.shutdown().await;
        cleanup?;
        storage?;
        tasks
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.kernel.shared.suspend();
        self.tasks.stop.cancel();
    }
}
