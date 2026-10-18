//! NE DNS installation only. Queries and their validity belong to meshway-dns.
use super::*;
use meshway_dns::{SystemDnsError as DnsError, SystemDnsFailure as Failure, SystemDnsPlan};
use std::net::Ipv4Addr;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub server: Ipv4Addr,
    pub match_domains: Vec<String>,
    pub search_domains: Vec<String>,
}
impl Settings {
    pub(super) fn valid(&self) -> bool {
        let local = meshway_dns::SystemDnsSettings {
            anchor_address: self.server,
            ..Default::default()
        };
        local.validate().is_ok()
            && !self.match_domains.is_empty()
            && self.match_domains.len() <= 256
            && self.search_domains.len() <= 256
            && self.match_domains.iter().all(|name| {
                name.is_empty()
                    || meshway_configuration::normalize_dns_name(name)
                        .is_ok_and(|normalized| normalized.trim_end_matches('.') == name)
            })
            && self
                .search_domains
                .iter()
                .all(|name| !name.is_empty() && self.match_domains.contains(name))
    }
}
pub(super) struct Attachment {
    address: Ipv4Addr,
    mtu: usize,
    port: Mutex<Option<Arc<meshway_dns::packet::Port>>>,
}
impl Attachment {
    pub fn new(address: Ipv4Addr, mtu: usize) -> Self {
        Self {
            address,
            mtu,
            port: Mutex::new(None),
        }
    }
    pub fn port(&self) -> Option<Arc<meshway_dns::packet::Port>> {
        self.port
            .lock()
            .ok()?
            .as_ref()
            .filter(|port| !port.stopped())
            .cloned()
    }
    pub fn close(&self) {
        if let Ok(mut port) = self.port.lock()
            && let Some(port) = port.take()
        {
            port.close();
        }
    }
    pub async fn prepare(
        self: &Arc<Self>,
        settings: Arc<tokio::sync::Mutex<network_settings::Owner>>,
    ) -> std::result::Result<Box<dyn SystemDns>, Failure> {
        conflict(self.address, &settings.lock().await.pools)?;
        Ok(Box::new(Owner {
            attachment: self.clone(),
            settings,
            installed: None,
            failure: None,
        }))
    }
}
pub(super) fn conflict(
    address: Ipv4Addr,
    pools: &[ipnet::IpNet],
) -> std::result::Result<(), Failure> {
    if pools.iter().any(|pool| pool.contains(&IpAddr::V4(address))) {
        return Err(Failure::new(DnsError::Conflict, "apple_dns_mesh_pool")
            .expected("DNS service address outside Mesh pools")
            .actual(address));
    }
    for interface in nix::ifaddrs::getifaddrs()
        .map_err(|error| Failure::new(DnsError::Backend, "apple_dns_interfaces").actual(error))?
    {
        if interface
            .address
            .and_then(|socket| socket.as_sockaddr_in().map(|socket| socket.ip()))
            == Some(address)
        {
            return Err(
                Failure::new(DnsError::Conflict, "apple_dns_interface_address")
                    .expected("unassigned local DNS service address")
                    .actual(format!("{} owns {address}", interface.interface_name)),
            );
        }
    }
    Ok(())
}
struct Owner {
    attachment: Arc<Attachment>,
    settings: Arc<tokio::sync::Mutex<network_settings::Owner>>,
    installed: Option<Settings>,
    failure: Option<Failure>,
}
impl Owner {
    async fn verify(&self) -> std::result::Result<(), Failure> {
        let settings = self.settings.lock().await;
        conflict(self.attachment.address, &settings.pools)?;
        if settings.failed {
            return Err(Failure::new(DnsError::Backend, "apple_ne_settings")
                .actual("last NE mutation has no successful completion"));
        }
        if let Some(expected) = &self.installed
            && settings
                .current
                .as_ref()
                .and_then(|settings| settings.dns.as_ref())
                != Some(expected)
        {
            return Err(Failure::new(DnsError::Conflict, "apple_dns_settings")
                .actual("NE DNS attachment differs from this owner"));
        }
        Ok(())
    }
    async fn install_plan(&mut self, plan: &SystemDnsPlan) -> std::result::Result<(), Failure> {
        let mut owner = self.settings.lock().await;
        conflict(self.attachment.address, &owner.pools)?;
        let mut settings = owner.current.clone().ok_or_else(|| {
            Failure::new(DnsError::Backend, "apple_dns_capture")
                .actual("Mesh capture not installed")
        })?;
        let domains = plan.domains();
        let dns = (!domains.is_empty()).then(|| Settings {
            server: self.attachment.address,
            match_domains: domains
                .iter()
                .map(|(domain, _)| {
                    if domain == "." {
                        String::new()
                    } else {
                        domain.trim_end_matches('.').to_owned()
                    }
                })
                .collect(),
            search_domains: domains
                .iter()
                .filter(|(_, only)| !only)
                .map(|(domain, _)| domain.trim_end_matches('.').to_owned())
                .collect(),
        });
        settings.dns = dns.clone();
        owner
            .apply(Some(settings))
            .await
            .map_err(|error| Failure::new(DnsError::Backend, "apple_ne_dns_apply").actual(error))?;
        self.installed = dns;
        Ok(())
    }
}
impl SystemDns for Owner {
    fn packet_transport(
        &mut self,
    ) -> std::result::Result<Option<meshway_dns::packet::Transport>, DnsError> {
        self.attachment.close();
        let (port, transport) =
            meshway_dns::packet::channel(self.attachment.address, self.attachment.mtu)
                .map_err(|_| DnsError::Settings)?;
        *self.attachment.port.lock().map_err(|_| DnsError::Backend)? = Some(port);
        Ok(Some(transport))
    }
    fn install<'a>(
        &'a mut self,
        plan: &'a SystemDnsPlan,
    ) -> SendFuture<'a, std::result::Result<(), DnsError>> {
        Box::pin(async move {
            let result = self.install_plan(plan).await;
            self.failure = result.as_ref().err().cloned();
            result.map_err(|failure| failure.error)
        })
    }
    fn observe(&mut self) -> SendFuture<'_, std::result::Result<(), DnsError>> {
        Box::pin(async {
            let result = self.verify().await;
            self.failure = result.as_ref().err().cloned();
            result.map_err(|failure| failure.error)
        })
    }
    fn recover(&mut self) -> SendFuture<'_, std::result::Result<(), DnsError>> {
        self.observe()
    }
    fn down(&mut self) -> SendFuture<'_, std::result::Result<(), DnsError>> {
        Box::pin(async {
            let mut owner = self.settings.lock().await;
            if let Some(mut settings) = owner.current.clone() {
                settings.dns = None;
                owner
                    .apply(Some(settings))
                    .await
                    .map_err(|_| DnsError::Backend)?;
            }
            self.installed = None;
            Ok(())
        })
    }
    fn shutdown(&mut self, keep: bool) -> SendFuture<'_, std::result::Result<(), DnsError>> {
        Box::pin(async move {
            self.attachment.close();
            if keep { Ok(()) } else { self.down().await }
        })
    }
    fn failure(&self) -> Option<&Failure> {
        self.failure.as_ref()
    }
    fn system_resolvers(&self) -> Option<meshway_dns::SystemResolverSnapshot> {
        None
    }
    fn served_resolvers(
        &self,
    ) -> std::result::Result<
        meshway_dns::SystemResolverSnapshot,
        meshway_model::exit::ExitDnsUnavailableReason,
    > {
        Err(meshway_model::exit::ExitDnsUnavailableReason::Unobserved)
    }
}
