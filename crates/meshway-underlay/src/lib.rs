//! Owned Linux capture and purpose-bound bootstrap/data sockets.
//!
//! Kernel rules have no incarnation cookie or compare-and-delete operation.
//! Recovery requires exact, unique journal matches and exclusive mutation of
//! the recorded resources. An identical privileged replacement is not detectable.
//! Conflicts retain the journal and negative boundary; they never trigger flushes.
#![forbid(unsafe_code)]

mod resource;
mod target;
pub use resource::netmap_resource_compatible;

/// Names of this crate's private files in the directory given to `Host::open`.
mod files {
    pub(crate) const LOCK: &str = "underlay.lock";
    pub(crate) const JOURNAL: &str = "underlay.json";
    pub(crate) const EGRESS: &str = "underlay-egress.json";
    pub(crate) const EXIT: &str = "underlay-exit.json";
    pub(crate) const EXIT_CHILD: &str = "exit-child.json";
    pub(crate) const BOOTSTRAP_RECORD: &str = "underlay-bootstrap.json";
    pub(crate) const BOOTSTRAP_SOCKET: &str = "underlay-bootstrap.sock";
    /// Every journal that is archived with [`meshway_private_storage::Directory::archive_stale`].
    pub(crate) const ARCHIVED: [&str; 4] = [JOURNAL, EGRESS, EXIT, EXIT_CHILD];
}

/// Whether `name` is a file this crate keeps in the Node's state directory:
/// its lock, its resource journals with their bounded stale archives, and the
/// bootstrap proxy record and socket. The daemon that assembles the directory
/// uses this to tell its own leftovers from storage of unknown ownership.
pub fn owns_file(name: &str) -> bool {
    name == files::LOCK
        || name == files::BOOTSTRAP_RECORD
        || name == files::BOOTSTRAP_SOCKET
        || files::ARCHIVED.iter().any(|source| {
            name == *source || meshway_private_storage::stale_stamp(name, source).is_some()
        })
}

#[cfg(not(target_os = "linux"))]
use std::sync::Arc;
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{
    CtConfirmation, DatagramAuthorization, DatagramCandidate, EgressCapture, ExitCutoff,
    ExitIncarnation, ExitReply, ExitReplyReader, ExitRole, ExitScope, ExitSendGuard, ExitTransport,
    ExitWrite, ExitWriter, Host, Invalidated, OriginalTuple, ProjectionAck, ProtectedDatagram,
    TraversalDatagram, ephemeral_ports, port_holders,
    sockets::{CandidateHandle, ListenerHandle, ProtectedListener, ProtectedTcp, SocketFactory},
};

/// The Underlay's classification of external network changes, published by
/// `Host::network_changes`. An observation only: it carries no authority.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetworkChange {
    /// Every classified change, path or refresh.
    pub epoch: u64,
    /// Changes of the underlay path fingerprint: external up interfaces with
    /// their carrier and preferred addresses, and this Node's marked route
    /// toward the Server. Sockets may reach peers from another source now.
    pub path: u64,
    /// The latest change's class; `None` before the first.
    pub class: Option<NetworkChangeClass>,
    /// Wall-clock time of the latest change.
    pub at: Option<std::time::SystemTime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkChangeClass {
    /// The path fingerprint changed.
    Path,
    /// An external change that left the fingerprint as it was (an unrelated
    /// interface, a temporary address): local endpoints are sampled again.
    Refresh,
}

/// An address of an external interface inside a Mesh pool: capture fails
/// closed while it lasts and recovers by itself once the address leaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolOverlap {
    pub interface: String,
    pub address: IpAddr,
}

/// The kept resources of a role that stopped serving and whose cleanup
/// failed: the owner retries it (1 s, doubling to 60 s), reading the kernel
/// again before each attempt. A failed cleanup is not an absence proof; the
/// role installs again once the cleanup completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitCleanupPending {
    /// The last attempt's error.
    pub reason: String,
    /// Attempts so far, the first one included.
    pub attempts: u32,
    pub retry_at: std::time::SystemTime,
}

/// The kernel owner a retried change belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KernelChangeOwner {
    /// The egress capture owner (`pause`, `stage`, `switch`).
    EgressCapture,
    /// The exit role owner: a projection's retirement after the named
    /// request (`retire`, `renew`, `install`, `confirm`), or `maintain`.
    Exit,
}

/// A kernel change whose attempt failed and that its owner retries after
/// reading the kernel back: a failed change proves neither outcome. Present
/// while the retry lasts (at most 30 s from the first failure); the owner then
/// fails closed as before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelChangePending {
    pub owner: KernelChangeOwner,
    pub operation: String,
    /// The last attempt's error.
    pub reason: String,
    /// Attempts so far, the first one included.
    pub attempts: u32,
    pub retry_at: std::time::SystemTime,
}

/// Local installation inputs, never business DNS or execution authority.
#[derive(Clone)]
pub struct Options {
    pub mtu: usize,
    pub max_pools: usize,
    /// Total protected TCP/UDP descriptors; a bounded share is reserved for
    /// Server bootstrap so data connections cannot starve lease renewal.
    pub max_sockets: usize,
    /// Entries one netlink dump may return; the source-capture table alone
    /// holds up to `MAX_CAPTURE_ROUTES` routes.
    pub max_kernel_objects: usize,
    pub connect_timeout: Duration,
    pub bootstrap_nameservers: Option<Vec<SocketAddr>>,
    pub bootstrap_hosts: BTreeMap<String, Vec<IpAddr>>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mtu: 1280,
            max_pools: 64,
            max_sockets: 128,
            max_kernel_objects: 65_536,
            connect_timeout: Duration::from_secs(10),
            bootstrap_nameservers: None,
            bootstrap_hosts: BTreeMap::new(),
        }
    }
}

impl Options {
    fn validate(&self) -> Result<()> {
        if !(1280..=65535).contains(&self.mtu)
            || !(1..=128).contains(&self.max_pools)
            || !(1..=1024).contains(&self.max_sockets)
            || !(1024..=65536).contains(&self.max_kernel_objects)
            || self.connect_timeout.is_zero()
            || self.connect_timeout > Duration::from_secs(60)
            || self.bootstrap_hosts.len() > 128
            || self
                .bootstrap_hosts
                .values()
                .any(|v| v.is_empty() || v.len() > 16)
            || self
                .bootstrap_nameservers
                .as_ref()
                .is_some_and(|v| v.is_empty() || v.len() > 8)
        {
            return Err(Error::Options);
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Linux TUN underlay is unsupported on this platform")]
    Unsupported,
    #[error("invalid underlay installation options")]
    Options,
    #[error("invalid fixed Server origin")]
    Origin,
    #[error("fixed-origin Control proxy is not attached")]
    ControlUnavailable,
    #[error("execution snapshot is not current or does not match the installed scope")]
    Expired,
    #[error("unsupported execution target or socket declaration")]
    Target,
    #[error("underlay resource limit reached")]
    Capacity,
    #[error("kernel ownership or routing protection conflict; resources kept")]
    Conflict,
    #[error("socket mark preparation or readback failed")]
    Protection,
    #[error("private underlay journal is invalid or requires recovery")]
    Journal,
    /// A journal an earlier version wrote, still owning host resources this
    /// version cannot recover.
    #[error(
        "private underlay journal {file} has version {version} from an earlier Meshway and still owns host resources: run `meshway down` with that earlier version (or reboot the host), then upgrade"
    )]
    JournalVersion { file: &'static str, version: u64 },
    #[error(
        "bootstrap DNS requires explicit real upstreams or a hosts entry; local stub upstream protection is unknown"
    )]
    Resolver,
    #[error("private underlay storage failed")]
    Storage(#[from] meshway_private_storage::Error),
    #[error("underlay I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// A listener's local address is unavailable, e.g. another process holds it.
    #[error("cannot bind {address}: {source}")]
    Bind {
        address: std::net::SocketAddr,
        source: std::io::Error,
    },
    #[error("TUN installation failed: {0}")]
    Tun(#[from] meshway_tun::Error),
    #[error("kernel netlink operation failed")]
    Netlink,
    #[error("underlay worker stopped unexpectedly")]
    Worker,
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(not(target_os = "linux"))]
pub struct Host;

#[cfg(not(target_os = "linux"))]
impl Host {
    pub async fn open(
        _directory: Arc<meshway_private_storage::Directory>,
        options: Options,
    ) -> Result<Self> {
        options.validate()?;
        Err(Error::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn owned_files_are_the_lock_journals_their_archives_and_the_bootstrap_proxy() {
        for name in [
            "underlay.lock",
            "underlay.json",
            "underlay.json.stale-1760200000000000000",
            "underlay-egress.json",
            "underlay-egress.json.stale-1",
            "underlay-exit.json",
            "underlay-exit.json.stale-2",
            "exit-child.json",
            "exit-child.json.stale-3",
            "underlay-bootstrap.json",
            "underlay-bootstrap.sock",
        ] {
            assert!(super::owns_file(name), "{name}");
        }
        for name in [
            "daemon.json",
            "daemon.lock",
            "identity",
            "underlay.json.stale-",
            "underlay.json.stale-1x",
            "underlay.json.bak",
            "underlay.json.stale-1.bak",
            "notes.txt",
        ] {
            assert!(!super::owns_file(name), "{name}");
        }
    }
}
