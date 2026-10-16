mod datagram;
mod traversal;
pub use traversal::{DatagramAuthorization, TraversalDatagram};
mod dns;
pub use datagram::{DatagramCandidate, ProtectedDatagram};
mod kernel;
pub use kernel::egress::EgressCapture;
pub use kernel::exit::{
    CtConfirmation, ExitCleanupPending, ExitCutoff, ExitIncarnation, ExitReply, ExitReplyReader,
    ExitRole, ExitScope, ExitSendGuard, ExitTransport, ExitWrite, ExitWriter, Invalidated,
    OriginalTuple, ProjectionAck,
};
mod protection;

/// This host's `net.ipv4.ip_local_port_range`: ports the kernel may give an
/// unbound outgoing socket of any process, so a listener there can find its
/// port held for a moment. `None` when it cannot be read.
pub fn ephemeral_ports() -> Option<std::ops::RangeInclusive<u16>> {
    let text = std::fs::read_to_string("/proc/sys/net/ipv4/ip_local_port_range").ok()?;
    let mut fields = text.split_ascii_whitespace().map(str::parse::<u16>);
    let (Some(Ok(first)), Some(Ok(last)), None) = (fields.next(), fields.next(), fields.next())
    else {
        return None;
    };
    (first <= last).then_some(first..=last)
}

/// Processes holding a socket bound to local `port` (UDP or TCP, IPv4 or
/// IPv6), as `(pid, command)`: the diagnosis of a listener whose bind found
/// its port in use. Bounded and best effort; unreadable entries are skipped.
pub fn port_holders(port: u16, udp: bool) -> Vec<(u32, String)> {
    let suffix = format!(":{port:04X}");
    let mut inodes = std::collections::BTreeSet::new();
    for table in if udp {
        ["udp", "udp6"]
    } else {
        ["tcp", "tcp6"]
    } {
        let Ok(text) = std::fs::read_to_string(format!("/proc/net/{table}")) else {
            continue;
        };
        for line in text.lines().skip(1).take(65536) {
            let fields: Vec<_> = line.split_ascii_whitespace().collect();
            if fields.get(1).is_some_and(|local| local.ends_with(&suffix))
                && let Some(inode) = fields.get(9).filter(|inode| **inode != "0")
            {
                inodes.insert(format!("socket:[{inode}]"));
            }
        }
    }
    let mut holders = Vec::new();
    let Ok(processes) = std::fs::read_dir("/proc") else {
        return holders;
    };
    for process in processes.flatten().take(65536) {
        let Some(pid) = process
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(descriptors) = std::fs::read_dir(process.path().join("fd")) else {
            continue;
        };
        if descriptors.flatten().take(65536).any(|fd| {
            std::fs::read_link(fd.path())
                .is_ok_and(|target| target.to_str().is_some_and(|t| inodes.contains(t)))
        }) {
            let command = std::fs::read_to_string(process.path().join("comm"))
                .map(|comm| comm.trim().to_owned())
                .unwrap_or_default();
            holders.push((pid, command));
        }
    }
    holders
}
mod proxy;
mod resolver_exchange;
pub mod sockets;
mod tasks;
#[cfg(test)]
mod tests;

use meshway_engine::Desired;
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
    options: Options,
    /// The fixed Server origin of the attached Control relay.
    origin: Option<String>,
    reproof: Reproof,
    // Released after the kernel, proxy and private-directory ownership.
    _owner: HostOwner,
}

/// The first re-proof of an unavailable end, and the longest wait between two.
const REPROOF: std::time::Duration = std::time::Duration::from_secs(1);
const REPROOF_MAX: std::time::Duration = std::time::Duration::from_secs(8);

/// Re-proofs of the ended term's ownership so far.
#[derive(Default)]
struct Reproof {
    /// The ended term's owners are joined and its tasks stopped.
    closed: bool,
    attempts: u32,
    last: Option<std::time::Instant>,
    /// The last failure, logged once while it repeats.
    failure: Option<String>,
}

impl Reproof {
    /// When the next attempt is due after an unavailable end at `ended`.
    fn due(&self, ended: std::time::Instant) -> std::time::Instant {
        let wait = REPROOF
            .saturating_mul(1 << self.attempts.min(5))
            .min(REPROOF_MAX);
        self.last.unwrap_or(ended) + if self.attempts == 0 { REPROOF } else { wait }
    }
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
    /// The cleanup of a stopped exit role its owner is retrying.
    pub fn exit_cleanup(&self) -> Option<ExitCleanupPending> {
        self.kernel
            .exit
            .as_ref()
            .and_then(|owner| owner.role.cleanup_pending())
    }
    /// The stopped exit role's cleanup its owner is retrying, live: set
    /// when an attempt fails, cleared the moment the cleanup completes.
    pub fn exit_cleanups(&self) -> tokio::sync::watch::Receiver<Option<ExitCleanupPending>> {
        self.kernel.shared.cleanups.subscribe()
    }
    /// The kernel change an owner of this Host is retrying, live: it reads
    /// the owner's own fact, also while a Host operation waits for it.
    pub fn kernel_changes(
        &self,
    ) -> tokio::sync::watch::Receiver<Option<crate::KernelChangePending>> {
        self.kernel.shared.changes.subscribe()
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
            tokio::task::spawn_blocking(move || Directory::open(&path, crate::files::LOCK))
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
            let sockets = SocketFactory::new(kernel.shared.clone(), options.clone());
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
                    options,
                    origin: None,
                    reproof: Reproof::default(),
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
        self.open_proxy(&origin, proxy::Listen::Now).await?;
        self.origin = Some(origin);
        Ok(())
    }

    async fn open_proxy(&mut self, origin: &str, listen: proxy::Listen) -> Result<()> {
        self.kernel.shared.check()?;
        let resolver = self.sockets.resolver().await?.clone();
        self.proxy = Some(
            Proxy::open(
                self.directory.clone(),
                origin,
                self.kernel.shared.clone(),
                resolver,
                self.sockets.options.max_sockets,
                self.sockets.options.connect_timeout,
                listen,
            )
            .await?,
        );
        Ok(())
    }

    /// Why this Host's kernel ownership ended; `None` while it holds.
    pub fn ownership_ended(&self) -> Option<crate::OwnershipEnd> {
        self.kernel.shared.ended().map(|(cause, _)| cause)
    }

    /// When [`Self::recover`] is due: only after an unavailable end, 1 s
    /// after it, then doubling to at most 8 s between attempts.
    pub fn recovery_at(&self) -> Option<std::time::Instant> {
        match self.kernel.shared.ended()? {
            (crate::OwnershipEnd::Unavailable { .. }, ended) => Some(self.reproof.due(ended)),
            (crate::OwnershipEnd::Conflict { .. }, _) => None,
        }
    }

    /// Prove kernel ownership again after an unavailable end. The ended
    /// term's owners are joined, its TUN closed and its tasks (Control relay
    /// included) stopped; then a new term opens exactly as at process start:
    /// every journal is recovered and verified against the kernel as it is
    /// now. Kernel objects read back as recorded (or a clean start) give a
    /// new term with the same journal, published channels and relay origin;
    /// the caller installs its content again. A read or change that fails
    /// again keeps the end unavailable (`Netlink`/`Io`, retried at
    /// [`Self::recovery_at`]); anything else read back (a foreign object, an
    /// unrecoverable journal) is a conflict that stays.
    pub async fn recover(&mut self) -> Result<()> {
        match self.kernel.shared.ended() {
            None => return Ok(()),
            Some((crate::OwnershipEnd::Conflict { .. }, _)) => return Err(Error::Conflict),
            Some((crate::OwnershipEnd::Unavailable { .. }, _)) => {}
        }
        if !self.reproof.closed {
            // The exit owner retries its cleanup until the Host's tasks stop.
            self.tasks.stop.cancel();
            self.kernel.close_term().await;
            self.proxy = None;
            let _ = self.tasks.shutdown().await;
            self.reproof.closed = true;
        }
        self.reproof.attempts = self.reproof.attempts.saturating_add(1);
        self.reproof.last = Some(std::time::Instant::now());
        let tasks = Tasks::new(self.options.max_sockets * 4 + 32);
        let opened = async {
            let kernel = Kernel::open_term(
                self.directory.clone(),
                self.options.clone(),
                tasks.clone(),
                self.kernel.shared.published(),
            )
            .await?;
            let sockets = SocketFactory::new(kernel.shared.clone(), self.options.clone());
            Ok::<_, Error>((kernel, sockets))
        }
        .await;
        let (kernel, sockets) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                let _ = tasks.shutdown().await;
                return Err(self.reproof_failed(error));
            }
        };
        let attempts = self.reproof.attempts;
        let ended = self.kernel.shared.ended();
        self.kernel = kernel;
        self.sockets = sockets;
        self.tasks = tasks;
        if let Some(origin) = self.origin.clone()
            && let Err(error) = self.open_proxy(&origin, proxy::Listen::Retrying).await
        {
            // The new term holds; only its relay's resolver did not open (a
            // socket that cannot listen yet is the relay's own retry). End
            // it as unavailable: the next attempt opens both again.
            tracing::warn!(%error, "Protected control relay did not open in the new ownership term");
            self.kernel.shared.end(crate::OwnershipEnd::Unavailable {
                check: "relay_unavailable",
            });
            self.reproof = Reproof {
                attempts,
                last: Some(std::time::Instant::now()),
                ..Reproof::default()
            };
            return Err(error);
        }
        let (check, elapsed_ms) = ended.map_or(("unrecorded", 0), |(cause, at)| {
            (cause.check(), at.elapsed().as_millis() as u64)
        });
        tracing::info!(
            check,
            attempts,
            elapsed_ms,
            "Kernel ownership proved again; the Host owns a new term"
        );
        self.reproof = Reproof::default();
        Ok(())
    }

    /// A failed re-proof: a read or change that failed again keeps the end
    /// unavailable; anything else makes it a conflict.
    fn reproof_failed(&mut self, error: Error) -> Error {
        let transient = matches!(error, Error::Netlink | Error::Io(_) | Error::Drain);
        let failure = error.to_string();
        if self.reproof.failure.as_ref() != Some(&failure) {
            if transient {
                tracing::warn!(
                    attempts = self.reproof.attempts,
                    error = %failure,
                    "Kernel ownership re-proof unavailable; retrying"
                );
            } else {
                tracing::warn!(
                    attempts = self.reproof.attempts,
                    error = %failure,
                    "Kernel ownership re-proof read back a conflict; ownership stays ended until restart"
                );
            }
            self.reproof.failure = Some(failure);
        }
        if !transient {
            self.kernel
                .shared
                .end(crate::OwnershipEnd::Conflict { check: "reproof" });
        }
        error
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
    /// External network changes, coalesced and classified as path or
    /// refresh ([`crate::NetworkChange`]). Observations carry no authority.
    pub fn network_changes(&self) -> tokio::sync::watch::Receiver<crate::NetworkChange> {
        self.kernel.shared.network.changes.subscribe()
    }
    /// The external address inside a Mesh pool that keeps capture failed
    /// closed, while one is assigned. Its departure is the recovery signal.
    pub fn pool_overlap(&self) -> tokio::sync::watch::Receiver<Option<crate::PoolOverlap>> {
        self.kernel.shared.network.overlap.subscribe()
    }
    /// Counts removals of owned capture objects outside this owner; each
    /// change asks for [`Self::repair`].
    pub fn repairs(&self) -> tokio::sync::watch::Receiver<u64> {
        self.kernel.shared.network.repairs.subscribe()
    }
    /// Reinstall owned capture rules and routes removed outside this owner
    /// (a network manager's cleanup of rules it does not know). Bounded: a
    /// fourth repair within a minute is a conflict, as is a slot another
    /// owner now holds.
    pub async fn repair(&mut self) -> Result<()> {
        self.kernel.repair().await
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
        self.kernel.shared.installed().ok()??;
        self.kernel.tun.as_ref()
    }

    /// Select this around the actor's TUN/packet loop. A foreign kernel
    /// mutation or Host stop ends the physical owner.
    pub async fn ended(&self) {
        tokio::select! {
            _ = self.kernel.shared.conflict.cancelled() => {},
            _ = self.kernel.shared.tasks.stop.cancelled() => {},
        }
    }

    /// Apply desired content to capture. Success does not claim carrier
    /// establishment, business DNS installation, or an applied receipt.
    ///
    /// The same content changes nothing. New content for the same Node keeps
    /// the owned TUN when the capture pools are unchanged, keeps every
    /// descriptor and exit role whose exact purpose is unchanged and
    /// re-stages only a changed capture plan. `suspend` (the empty apply)
    /// ends every operation and descriptor and keeps the negative boundary.
    /// The TCP MSS clamp of this Node's TUNs (see [`meshway_tun::PathLimits`]).
    pub fn clamp_mss(&mut self, limits: meshway_tun::PathLimits) -> Result<()> {
        self.kernel.clamp(limits)
    }
    pub async fn install(&mut self, desired: Desired) -> Result<()> {
        self.kernel.install(desired).await
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
