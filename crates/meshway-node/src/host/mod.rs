//! Platform attachment, separate from the Node's authority and packet owners.
use meshway_engine::Desired;
use meshway_protocol::ipc::Problem;
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::watch;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::Linux;
#[cfg(target_os = "macos")]
pub mod apple;

pub(crate) fn owns_file(name: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        name == apple::JOURNAL
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = name;
        false
    }
}

pub type Local<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;
pub type SendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Linux(#[from] meshway_underlay::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Tun(#[from] meshway_tun::Error),
    #[error("platform capability is unavailable: {0}")]
    Unsupported(&'static str),
}
impl From<std::io::ErrorKind> for Error {
    fn from(kind: std::io::ErrorKind) -> Self {
        Self::Io(kind.into())
    }
}
/// Construct the owner future on its executor, rather than moving a !Send
/// future between threads. The Linux adapter alone chooses blocking + block_on.
pub type Worker = Box<dyn FnOnce() -> Local<'static, std::result::Result<(), Problem>> + Send>;

#[derive(Clone, Debug, Default)]
pub struct Capabilities {
    pub packets: bool,
    pub path_observation: bool,
    pub exit_families: Vec<meshway_model::exit::ExitFamily>,
}

/// Opens one platform owner per runtime, and selects that owner's executor.
/// There is no packet or authority state in this factory.
pub trait Platform: Send + Sync + 'static {
    fn capabilities(&self) -> Capabilities;
    fn interfaces(&self) -> std::io::Result<Vec<InterfaceAddress>>;
    fn open<'a>(
        &'a self,
        directory: Arc<meshway_private_storage::Directory>,
        origin: &'a str,
        options: meshway_underlay::Options,
        dns: meshway_dns::SystemDnsSettings,
    ) -> SendFuture<'a, std::result::Result<Box<dyn Host>, Problem>>;
    fn spawn(&self, worker: Worker) -> tokio::task::JoinHandle<std::result::Result<(), Problem>>;
}

/// Explicit control-only platform, useful to embedded hosts before packet
/// attachment exists. It never advertises or opens a packet plane.
pub struct ControlOnly;
impl Platform for ControlOnly {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }
    fn interfaces(&self) -> std::io::Result<Vec<InterfaceAddress>> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
    fn open<'a>(
        &'a self,
        _: Arc<meshway_private_storage::Directory>,
        _: &'a str,
        _: meshway_underlay::Options,
        _: meshway_dns::SystemDnsSettings,
    ) -> SendFuture<'a, std::result::Result<Box<dyn Host>, Problem>> {
        Box::pin(async { Err(Problem::UnsupportedMode) })
    }
    fn spawn(&self, _: Worker) -> tokio::task::JoinHandle<std::result::Result<(), Problem>> {
        tokio::spawn(async { Err(Problem::UnsupportedMode) })
    }
}

#[derive(Clone, Debug)]
pub struct InterfaceAddress {
    pub name: String,
    pub address: std::net::IpAddr,
}

/// Capture and physical resources only. Empty apply retains the negative
/// boundary; down removes owned resources; shutdown joins every producer.
pub trait Host: Send {
    fn install(&mut self, desired: Desired) -> Local<'_, Result<()>>;
    fn suspend(&mut self) -> Local<'_, Result<()>>;
    fn down(&mut self) -> Local<'_, Result<()>>;
    fn shutdown(self: Box<Self>, keep: bool) -> Local<'static, Result<()>>;
    fn tun(&self) -> Option<&dyn meshway_tun::PacketDevice>;
    fn sockets(&self) -> Arc<dyn meshway_connectivity::SocketFactory>;
    fn resolver_exchange(&self) -> Arc<dyn meshway_dns::ResolverExchange>;
    fn client_builder(&self) -> Result<reqwest::ClientBuilder>;
    fn request_interrupt(&self) -> Option<Arc<dyn meshway_control_client::RequestInterrupt>> {
        None
    }
    fn network_changes(&self) -> watch::Receiver<meshway_underlay::NetworkChange>;
    fn pool_overlap(&self) -> watch::Receiver<Option<meshway_underlay::PoolOverlap>>;
    fn repairs(&self) -> watch::Receiver<u64>;
    fn owned_interfaces(&self) -> Vec<String>;
    fn ephemeral_ports(&self) -> Option<std::ops::RangeInclusive<u16>> {
        None
    }
    fn port_holders(&self, _port: u16, _udp: bool) -> Vec<(u32, String)> {
        Vec::new()
    }
    fn ownership_ended(&self) -> Option<meshway_underlay::OwnershipEnd>;
    fn ended(&self) -> Local<'_, ()>;
    fn recovery_at(&self) -> Option<std::time::Instant>;
    fn recover(&mut self) -> Local<'_, Result<()>>;
    fn repair_at(&self) -> Option<std::time::Instant>;
    fn repair(&mut self) -> Local<'_, Result<()>>;
    fn kernel_changes(
        &self,
    ) -> Option<watch::Receiver<Option<meshway_underlay::KernelChangePending>>> {
        None
    }
    fn exit_cleanups(&self) -> Option<watch::Receiver<Option<meshway_underlay::RetryPending>>> {
        None
    }
    fn control_relay(
        &self,
    ) -> Option<watch::Receiver<Option<meshway_underlay::ControlRelayState>>> {
        None
    }
    fn clamp_mss(&mut self, limits: meshway_tun::PathLimits) -> Result<()>;
    fn prepare_dns(
        &mut self,
    ) -> Local<'_, std::result::Result<Box<dyn SystemDns>, meshway_dns::SystemDnsFailure>>;
    /// Restore an unstarted owner's persisted DNS attachment before removing
    /// its capture anchor. Also handles an unused reservation after a crash.
    fn restore_dns(&mut self) -> Local<'_, std::result::Result<(), meshway_dns::SystemDnsError>>;
    fn remove_dns_anchor(&mut self) -> Local<'_, Result<()>>;
    fn probe_exit<'a>(
        &'a mut self,
        instance: meshway_model::InstanceId,
        netmap: meshway_model::NetmapRef,
        generation: meshway_model::Digest,
        families: &'a [meshway_model::exit::ExitFamily],
    ) -> Local<'a, meshway_model::exit::ExitCapability>;
    fn install_netmap_exit_role<'a>(
        &'a mut self,
        role: &'a meshway_configuration::netmap::ExitRole,
    ) -> Local<'a, Result<()>>;
    fn start_exit(
        &self,
        plan: meshway_configuration::netmap::ExitRole,
    ) -> std::result::Result<Option<ExitAttachment>, Problem>;
    fn diagnostics(&self) -> Arc<dyn LocalDiagnostics>;
}

pub trait LocalDiagnostics: Send + Sync {
    fn direct_icmp(&self) -> std::io::Result<Box<dyn Icmp>>;
    fn resolve<'a>(
        &'a self,
        name: &'a str,
    ) -> SendFuture<'a, std::io::Result<Vec<std::net::IpAddr>>>;
}

/// System installation/provenance only; query scopes and DNS selection remain
/// in meshway-dns and the Node's one DNS actor.
pub trait SystemDns: Send {
    fn install<'a>(
        &'a mut self,
        plan: &'a meshway_dns::SystemDnsPlan,
    ) -> SendFuture<'a, std::result::Result<(), meshway_dns::SystemDnsError>>;
    fn observe(&mut self) -> SendFuture<'_, std::result::Result<(), meshway_dns::SystemDnsError>>;
    fn recover(&mut self) -> SendFuture<'_, std::result::Result<(), meshway_dns::SystemDnsError>>;
    fn down(&mut self) -> SendFuture<'_, std::result::Result<(), meshway_dns::SystemDnsError>>;
    fn shutdown(
        &mut self,
        keep: bool,
    ) -> SendFuture<'_, std::result::Result<(), meshway_dns::SystemDnsError>>;
    fn failure(&self) -> Option<&meshway_dns::SystemDnsFailure>;
    fn system_resolvers(&self) -> Option<meshway_dns::SystemResolverSnapshot>;
    fn served_resolvers(
        &self,
    ) -> std::result::Result<
        meshway_dns::SystemResolverSnapshot,
        meshway_model::exit::ExitDnsUnavailableReason,
    >;
}

/// Direct diagnostics socket. Receive returns a complete IPv4 packet, as
/// expected by the shared Echo scheduler; no Mesh authorization is conferred.
pub trait Icmp: Send + Sync {
    fn send<'a>(
        &'a self,
        target: std::net::Ipv4Addr,
        bytes: &'a [u8],
    ) -> SendFuture<'a, std::io::Result<()>>;
    fn recv<'a>(&'a self, bytes: &'a mut [u8]) -> SendFuture<'a, std::io::Result<usize>>;
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct ExitHealth {
    pub flows: u32,
    pub failed: bool,
    pub ended: bool,
}

/// The optional host exit driver. It consumes the unique SecureNode's
/// projection callbacks; it cannot create its own flows or grant authority.
pub trait ExitDriver: Send {
    fn drive(&mut self, secure: &mut meshway_dataplane::secure::SecureNode) -> bool;
    fn packet(
        &mut self,
        secure: &mut meshway_dataplane::secure::SecureNode,
        packet: meshway_dataplane::PendingAction,
    );
    fn synchronize<'a>(
        &'a mut self,
        secure: &'a mut meshway_dataplane::secure::SecureNode,
    ) -> Local<'a, std::result::Result<(), Problem>>;
    fn stop<'a>(
        self: Box<Self>,
        secure: &'a mut meshway_dataplane::secure::SecureNode,
    ) -> Local<'a, std::result::Result<(), Problem>>;
    fn recv_batch(&mut self, max: usize) -> Local<'_, Result<Vec<Vec<u8>>>>;
}

pub struct ExitAttachment {
    pub plan: meshway_configuration::netmap::ExitRole,
    pub health: watch::Receiver<ExitHealth>,
    pub driver: Box<dyn ExitDriver>,
}
impl std::ops::Deref for ExitAttachment {
    type Target = dyn ExitDriver;
    fn deref(&self) -> &Self::Target {
        self.driver.as_ref()
    }
}
impl std::ops::DerefMut for ExitAttachment {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.driver.as_mut()
    }
}
impl ExitAttachment {
    pub async fn stop(
        self,
        secure: &mut meshway_dataplane::secure::SecureNode,
    ) -> std::result::Result<(), Problem> {
        self.driver.stop(secure).await
    }
}
