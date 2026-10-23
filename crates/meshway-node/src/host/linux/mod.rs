//! Linux attachment of the existing underlay, system DNS and exit owners.
use super::*;
use meshway_dns::{
    SystemDnsBackend, SystemDnsError, SystemDnsFailure, SystemDnsOwner, SystemDnsReservation,
    SystemDnsSettings,
};
mod exit;
mod icmp;

pub struct Linux;
impl Platform for Linux {
    fn interfaces(&self) -> std::io::Result<Vec<InterfaceAddress>> {
        use nix::{ifaddrs::getifaddrs, net::if_::InterfaceFlags};
        use std::net::IpAddr;
        Ok(getifaddrs()?
            .filter(|interface| {
                interface.flags.contains(InterfaceFlags::IFF_UP)
                    && !interface.flags.contains(InterfaceFlags::IFF_LOOPBACK)
            })
            .filter_map(|interface| {
                let socket = interface.address?;
                let address = socket
                    .as_sockaddr_in()
                    .map(|address| IpAddr::V4(address.ip()))
                    .or_else(|| {
                        socket
                            .as_sockaddr_in6()
                            .map(|address| IpAddr::V6(address.ip()))
                    })?;
                Some(InterfaceAddress {
                    name: interface.interface_name,
                    address,
                })
            })
            .collect())
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            packets: true,
            path_observation: true,
            exit_families: vec![
                meshway_model::exit::ExitFamily::Ipv4,
                meshway_model::exit::ExitFamily::Ipv6,
            ],
        }
    }
    fn open<'a>(
        &'a self,
        directory: Arc<meshway_private_storage::Directory>,
        origin: &'a str,
        mut options: meshway_underlay::Options,
        dns: SystemDnsSettings,
    ) -> SendFuture<'a, std::result::Result<Box<dyn Host>, OpenError>> {
        Box::pin(async move {
            let reservation = if matches!(dns.backend, SystemDnsBackend::OwnedResolvConf { .. }) {
                let reserved = SystemDnsOwner::reserve(dns.clone(), directory.clone())
                    .await
                    .map_err(dns_startup)?;
                if options.bootstrap_nameservers.is_none() && options.bootstrap_hosts.is_empty() {
                    options.bootstrap_nameservers = reserved
                        .bootstrap_nameservers()
                        .await
                        .map_err(dns_startup)?;
                }
                Some(reserved)
            } else {
                None
            };
            let mut inner = meshway_underlay::Host::open(directory.clone(), options)
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "Underlay startup failed");
                    crate::runtime::network::startup_failure(&error)
                })?;
            if let Err(error) = inner.attach_control(origin).await {
                tracing::warn!(%error, "Underlay Control attachment failed");
                if let Err(error) = inner.shutdown(true).await {
                    tracing::warn!(%error, "Underlay startup cleanup failed");
                }
                return Err(crate::runtime::network::startup_failure(&error));
            }
            Ok(Box::new(Attachment {
                inner,
                directory,
                dns,
                reservation,
            }) as Box<dyn Host>)
        })
    }
    fn spawn(&self, worker: Worker) -> tokio::task::JoinHandle<std::result::Result<(), Problem>> {
        // Live exit probes register under a synchronous monitor guard. Keep
        // this existing !Send execution choice in Linux, not in Node's worker.
        tokio::task::spawn_blocking(move || tokio::runtime::Handle::current().block_on(worker()))
    }
}

/// The resolv.conf reservation at start, judged as at run time: another
/// owner's reservation is proven, a refused setting blocks, anything else
/// (a read that failed) is retried.
fn dns_startup(error: SystemDnsError) -> OpenError {
    match error {
        SystemDnsError::Conflict => OpenError::proven(Problem::Dns),
        SystemDnsError::Settings | SystemDnsError::Unsupported => Problem::Dns.into(),
        _ => OpenError::unknown(Problem::Dns),
    }
}

struct Attachment {
    inner: meshway_underlay::Host,
    directory: Arc<meshway_private_storage::Directory>,
    dns: SystemDnsSettings,
    reservation: Option<SystemDnsReservation>,
}
impl Attachment {
    async fn reserve(&mut self) -> std::result::Result<(), SystemDnsError> {
        if self.reservation.is_none() {
            self.reservation =
                Some(SystemDnsOwner::reserve(self.dns.clone(), self.directory.clone()).await?);
        }
        Ok(())
    }
    async fn dns_owner(&mut self) -> std::result::Result<SystemDnsOwner, SystemDnsFailure> {
        // Another owner's reservation (or storage that is not private) is
        // the proven `owner_record` check; anything else proves nothing.
        self.reserve().await.map_err(|error| {
            let check = if error == SystemDnsError::Conflict {
                "owner_record"
            } else {
                "reserve"
            };
            SystemDnsFailure::new(error, check)
        })?;
        let reserved = self.reservation.as_ref().expect("reserved above");
        let anchor = if matches!(self.dns.backend, SystemDnsBackend::Resolved) {
            Some(self.inner.dns_anchor(reserved).await.map_err(|error| {
                SystemDnsFailure::new(SystemDnsError::Backend, "anchor_create")
                    .expected("the Kernel's DNS anchor for this reservation")
                    .actual(error)
                    .source("Underlay Kernel")
            })?)
        } else {
            None
        };
        SystemDnsOwner::prepare_reserved(self.reservation.take().expect("reserved above"), anchor)
            .await
    }
}

macro_rules! operation {
    ($($name:ident),* $(,)?) => { $(fn $name(&mut self) -> Local<'_, Result<()>> { Box::pin(async { self.inner.$name().await.map_err(Into::into) }) })* };
}
macro_rules! observation {
    ($($name:ident : $ty:ty),* $(,)?) => { $(fn $name(&self) -> $ty { self.inner.$name() })* };
}
impl Host for Attachment {
    fn ephemeral_ports(&self) -> Option<std::ops::RangeInclusive<u16>> {
        meshway_underlay::ephemeral_ports()
    }
    fn port_holders(&self, port: u16, udp: bool) -> Vec<(u32, String)> {
        meshway_underlay::port_holders(port, udp)
    }
    operation!(suspend, down, recover, repair, remove_dns_anchor);
    observation!(
        network_changes: watch::Receiver<meshway_underlay::NetworkChange>,
        pool_overlap: watch::Receiver<Option<meshway_underlay::PoolOverlap>>,
        repairs: watch::Receiver<u64>,
        owned_interfaces: Vec<String>,
        ownership_ended: Option<meshway_underlay::OwnershipEnd>,
        recovery_at: Option<std::time::Instant>,
        repair_at: Option<std::time::Instant>,
    );
    fn install(&mut self, desired: Desired) -> Local<'_, Result<()>> {
        Box::pin(async { self.inner.install(desired).await.map_err(Into::into) })
    }
    fn shutdown(self: Box<Self>, keep: bool) -> Local<'static, Result<()>> {
        Box::pin(async move { self.inner.shutdown(keep).await.map_err(Into::into) })
    }
    fn client_builder(&self) -> Result<reqwest::ClientBuilder> {
        self.inner.client_builder().map_err(Into::into)
    }
    fn kernel_changes(
        &self,
    ) -> Option<watch::Receiver<Option<meshway_underlay::KernelChangePending>>> {
        Some(self.inner.kernel_changes())
    }
    fn exit_cleanups(&self) -> Option<watch::Receiver<Option<meshway_underlay::RetryPending>>> {
        Some(self.inner.exit_cleanups())
    }
    fn control_relay(
        &self,
    ) -> Option<watch::Receiver<Option<meshway_underlay::ControlRelayState>>> {
        Some(self.inner.control_relay())
    }
    fn tun(&self) -> Option<&dyn meshway_tun::PacketDevice> {
        self.inner
            .tun()
            .map(|tun| tun as &dyn meshway_tun::PacketDevice)
    }
    fn sockets(&self) -> Arc<dyn meshway_connectivity::SocketFactory> {
        Arc::new(self.inner.sockets())
    }
    fn resolver_exchange(&self) -> Arc<dyn meshway_dns::ResolverExchange> {
        Arc::new(self.inner.sockets())
    }
    fn ownership(&self) -> Option<watch::Receiver<meshway_model::recovery::Health>> {
        Some(self.inner.ownership())
    }
    fn clamp_mss(&mut self, limits: meshway_tun::PathLimits) -> Result<()> {
        self.inner.clamp_mss(limits).map_err(Into::into)
    }
    fn prepare_dns(
        &mut self,
    ) -> Local<'_, std::result::Result<Box<dyn SystemDns>, SystemDnsFailure>> {
        Box::pin(async {
            self.dns_owner()
                .await
                .map(|owner| Box::new(owner) as Box<dyn SystemDns>)
        })
    }
    fn restore_dns(&mut self) -> Local<'_, std::result::Result<(), SystemDnsError>> {
        Box::pin(async {
            if self.reservation.is_none()
                && !SystemDnsOwner::has_reservation(self.directory.clone()).await?
            {
                return Ok(());
            }
            self.reserve().await?;
            if self
                .reservation
                .as_ref()
                .expect("reserved above")
                .has_backend_record()?
            {
                self.dns_owner()
                    .await
                    .map_err(|failure| failure.error)?
                    .shutdown(false)
                    .await?;
                self.inner
                    .remove_dns_anchor()
                    .await
                    .map_err(|_| SystemDnsError::Backend)?;
            } else {
                self.inner
                    .remove_dns_anchor()
                    .await
                    .map_err(|_| SystemDnsError::Backend)?;
                self.reservation
                    .as_mut()
                    .expect("reserved above")
                    .release_uninstalled()
                    .await?;
                self.reservation = None;
            }
            Ok(())
        })
    }
    fn probe_exit<'a>(
        &'a mut self,
        instance: meshway_model::InstanceId,
        netmap: meshway_model::NetmapRef,
        generation: meshway_model::Digest,
        families: &'a [meshway_model::exit::ExitFamily],
    ) -> Local<'a, meshway_model::exit::ExitCapability> {
        Box::pin(
            self.inner
                .probe_exit(instance, netmap, generation, families),
        )
    }
    fn install_netmap_exit_role<'a>(
        &'a mut self,
        role: &'a meshway_configuration::netmap::ExitRole,
    ) -> Local<'a, Result<()>> {
        Box::pin(async {
            self.inner
                .install_netmap_exit_role(role)
                .await
                .map(|_| ())
                .map_err(Into::into)
        })
    }
    fn start_exit(
        &self,
        plan: meshway_configuration::netmap::ExitRole,
    ) -> std::result::Result<Option<ExitAttachment>, Problem> {
        let Some(role) = self.inner.exit_role() else {
            return Ok(None);
        };
        let runtime = exit::Runtime::start(role)?;
        Ok(Some(ExitAttachment {
            plan,
            health: runtime.health.clone(),
            driver: Box::new(runtime),
        }))
    }
    fn diagnostics(&self) -> Arc<dyn LocalDiagnostics> {
        Arc::new(icmp::Diagnostics)
    }
}

macro_rules! dns_operation {
    ($($name:ident),*) => { $(fn $name(&mut self) -> SendFuture<'_, std::result::Result<(), SystemDnsError>> { Box::pin(SystemDnsOwner::$name(self)) })* };
}
impl SystemDns for SystemDnsOwner {
    dns_operation!(observe, recover, down);
    fn install<'a>(
        &'a mut self,
        plan: &'a meshway_dns::SystemDnsPlan,
    ) -> SendFuture<'a, std::result::Result<(), SystemDnsError>> {
        Box::pin(SystemDnsOwner::install(self, plan))
    }
    fn shutdown(&mut self, keep: bool) -> SendFuture<'_, std::result::Result<(), SystemDnsError>> {
        Box::pin(SystemDnsOwner::shutdown(self, keep))
    }
    fn failure(&self) -> Option<&SystemDnsFailure> {
        SystemDnsOwner::failure(self)
    }
    fn system_resolvers(&self) -> Option<meshway_dns::SystemResolverSnapshot> {
        SystemDnsOwner::system_resolvers(self)
    }
    fn served_resolvers(
        &self,
    ) -> std::result::Result<
        meshway_dns::SystemResolverSnapshot,
        meshway_model::exit::ExitDnsUnavailableReason,
    > {
        SystemDnsOwner::served_resolvers(self)
    }
}
