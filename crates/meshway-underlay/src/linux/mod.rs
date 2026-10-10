mod datagram;
mod traversal;
pub use traversal::{DatagramAuthorization, TraversalDatagram};
mod dns;
pub use datagram::{DatagramCandidate, ProtectedDatagram};
mod kernel;
pub use kernel::egress::EgressCapture;
pub use kernel::exit::{
    CtConfirmation, ExitCutoff, ExitIncarnation, ExitReply, ExitReplyReader, ExitRole, ExitScope,
    ExitSendGuard, ExitTransport, Invalidated, OriginalTuple, ProjectionAck,
};
mod protection;
mod proxy;
mod resolver_exchange;
pub mod sockets;
mod tasks;
#[cfg(test)]
mod tests;

use meshway_authority::ExecutionSnapshot;
use meshway_private_storage::Directory;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::{Error, Options, Result};
use kernel::Kernel;
use proxy::Proxy;
use sockets::SocketFactory;
use tasks::Tasks;

static HOST_OWNED: AtomicBool = AtomicBool::new(false);

struct HostOwner;

impl HostOwner {
    fn acquire() -> Result<Self> {
        HOST_OWNED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::Conflict)?;
        Ok(Self)
    }
}

impl Drop for HostOwner {
    fn drop(&mut self) {
        HOST_OWNED.store(false, Ordering::Release);
    }
}

/// One actor owns Host. Optional Control clients use its independently driven UDS relay;
/// data factories are cloneable but retain only opaque, revocable socket scope.
/// Drop stops socket tasks and drops TUN while retaining negative kernel rules.
/// Call shutdown to join producers and finish accepted private writes explicitly.
pub struct Host {
    kernel: Kernel,
    proxy: Option<Proxy>,
    directory: Arc<Directory>,
    sockets: SocketFactory,
    tasks: Tasks,
    // Released after the kernel, proxy and private-directory ownership.
    _owner: HostOwner,
}

impl Host {
    pub async fn install_egress_capture(
        &mut self,
        plan: meshway_configuration::execution::EgressCapturePlan,
    ) -> Result<EgressCapture> {
        self.kernel.install_egress_capture(plan).await
    }
    /// Probe the exit prerequisites for the role of `netmap` with
    /// `role_generation`, on behalf of `instance`.
    pub async fn probe_exit(
        &mut self,
        instance: meshway_model::InstanceId,
        netmap: meshway_model::NetmapRef,
        role_generation: meshway_model::Digest,
    ) -> meshway_model::exit::ExitCapability {
        self.kernel
            .probe_exit(instance, netmap, role_generation)
            .await
    }
    /// Prepare the installed netmap's exit role. It is bound to the content,
    /// not to an instance: re-acquiring an instance keeps it.
    pub async fn install_netmap_exit_role(
        &mut self,
        role: &meshway_configuration::netmap::ExitRole,
    ) -> Result<ExitRole> {
        self.kernel.install_exit_role(role.into()).await
    }
    pub fn exit_role(&self) -> Option<ExitRole> {
        self.kernel
            .exit
            .as_ref()
            .filter(|owner| owner.role.is_active())
            .map(|owner| owner.role.clone())
    }
    /// Construct a joinable pre-install probe, including for hosts where opening
    /// the protected socket owner fails. Keep it alive through `finish()`.
    pub fn exit_probe() -> Result<kernel::exit::ExitProbe> {
        Ok(kernel::exit::ExitProbe::new(HostOwner::acquire()?))
    }
    /// Open the sole kernel owner and protected socket factory, without contacting
    /// a Server or opening a resolver. Call attach_control for a fixed-origin relay.
    pub async fn open(directory: Arc<Directory>, options: Options) -> Result<Self> {
        options.validate()?;
        let owner = HostOwner::acquire()?;
        let path = directory.path().to_owned();
        // The directory lock also excludes other processes from this installation.
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
            let sockets = SocketFactory::new(kernel.shared.clone(), options);
            Ok::<_, Error>((kernel, sockets))
        }
        .await;
        match opened {
            Ok((kernel, sockets)) => {
                opening.0 = None;
                Ok(Self {
                    kernel,
                    proxy: None,
                    directory,
                    sockets,
                    tasks,
                    _owner: owner,
                })
            }
            Err(error) => {
                drop(opening);
                tasks.shutdown().await?;
                Err(error)
            }
        }
    }

    /// Attach the bootstrap resolver and fixed-origin Control relay to this
    /// owner's protected sockets. A live attachment cannot be replaced/rebound.
    pub async fn attach_control(&mut self, origin: &str) -> Result<()> {
        if self.proxy.is_some() {
            return Err(Error::Conflict);
        }
        let origin = meshway_control::server_origin(origin).map_err(|_| Error::Origin)?;
        self.kernel.shared.check()?;
        let resolver = self.sockets.resolver().await?.clone();
        self.proxy = Some(
            Proxy::open(
                self.directory.clone(),
                &origin,
                self.kernel.shared.clone(),
                resolver,
                self.sockets.options.max_sockets,
                self.sockets.options.connect_timeout,
            )
            .await?,
        );
        Ok(())
    }

    pub fn client_builder(&self) -> Result<reqwest::ClientBuilder> {
        self.proxy
            .as_ref()
            .ok_or(Error::ControlUnavailable)?
            .client_builder()
    }
    pub fn sockets(&self) -> SocketFactory {
        self.sockets.clone()
    }
    /// Coalesced link/address/route changes observed by the existing kernel
    /// monitor. Observations carry no execution authority.
    pub fn network_changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.kernel.shared.network_changes.subscribe()
    }
    /// Exact interface names held by this owner's journal (TUN and DNS anchor).
    pub fn owned_interfaces(&self) -> Vec<String> {
        self.kernel.owned_interfaces()
    }
    pub async fn dns_anchor(
        &mut self,
        reservation: &meshway_dns::SystemDnsReservation,
    ) -> Result<Arc<dyn meshway_dns::OwnedDnsLink>> {
        self.kernel.dns_anchor(reservation).await
    }
    pub async fn remove_dns_anchor(&mut self) -> Result<()> {
        self.kernel.remove_dns_anchor().await
    }
    pub fn tun(&self) -> Option<&meshway_tun::Tun> {
        if self.kernel.shared.check().is_err() {
            return None;
        }
        let snapshot = self.kernel.shared.installed().ok()??;
        snapshot.ensure_current().ok()?;
        self.kernel.tun.as_ref()
    }

    /// Select this around the actor's TUN/packet loop in addition to the execution
    /// snapshot's invalidation. A foreign kernel mutation stops positive I/O.
    pub async fn invalidated(&self) {
        // A superseded execution guard is not a broken physical owner. The
        // packet actor waits on its snapshot separately and can hand compatible
        // resources to the next netmap without deleting the TUN or channels.
        tokio::select! {
            _ = self.kernel.shared.conflict.cancelled() => {},
            _ = self.kernel.shared.tasks.stop.cancelled() => {},
        }
    }

    /// Install capture under fresh execution authority. Success does not claim
    /// carrier establishment, business DNS installation, or an applied receipt.
    ///
    /// The same netmap content (lease renewal, new instance binding) only
    /// replaces the installed snapshot: no socket, TUN or kernel object is
    /// touched and every guard keeps running with the raised deadline. A
    /// Control-validated continuation (a larger serial, or a legacy direct
    /// successor) keeps the owned TUN when the capture pools are unchanged,
    /// keeps every descriptor and exit role whose exact purpose is unchanged
    /// and re-stages only a changed capture plan. Calling suspend first
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
        let egress = self.kernel.finish_egress().await;
        let tasks = self.tasks.shutdown().await;
        cleanup?;
        storage?;
        egress?;
        tasks
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.kernel.shared.suspend();
        self.tasks.stop.cancel();
    }
}
