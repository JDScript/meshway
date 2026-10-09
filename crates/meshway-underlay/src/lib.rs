//! Owned Linux capture and purpose-bound bootstrap/data sockets.
//!
//! Kernel rules have no incarnation cookie or compare-and-delete operation.
//! Recovery requires exact, unique journal matches and exclusive mutation of
//! the recorded resources. An identical privileged replacement is not detectable.
//! Conflicts retain the journal and negative boundary; they never trigger flushes.
#![forbid(unsafe_code)]

mod resource;
pub use resource::resource_compatible;

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
    CtConfirmation, DatagramAuthorization, DatagramCandidate, EgressCapture, ExitIncarnation,
    ExitReply, ExitReplyReader, ExitRole, ExitScope, ExitSendGuard, ExitTransport, Host,
    Invalidated, OriginalTuple, ProjectionAck, ProtectedDatagram, TraversalDatagram,
    sockets::{CandidateHandle, ListenerHandle, ProtectedListener, ProtectedTcp, SocketFactory},
};

/// Local installation inputs, never business DNS or execution authority.
#[derive(Clone)]
pub struct Options {
    pub mtu: usize,
    pub max_pools: usize,
    /// Total protected TCP/UDP descriptors; a bounded share is reserved for
    /// Server bootstrap so data connections cannot starve lease renewal.
    pub max_sockets: usize,
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
            max_kernel_objects: 16_384,
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
    #[error("kernel ownership or routing protection conflict; resources retained")]
    Conflict,
    #[error("socket mark preparation or readback failed")]
    Protection,
    #[error("private underlay journal is invalid or requires recovery")]
    Journal,
    #[error(
        "bootstrap DNS requires explicit real upstreams or a hosts entry; local stub upstream protection is unknown"
    )]
    Resolver,
    #[error("private underlay storage failed")]
    Storage(#[from] meshway_private_storage::Error),
    #[error("underlay I/O failed: {0}")]
    Io(#[from] std::io::Error),
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
