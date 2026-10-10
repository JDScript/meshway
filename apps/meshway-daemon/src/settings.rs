use meshway_model::state::SystemState;
use std::path::PathBuf;

#[derive(Clone)]
pub struct Settings {
    pub(crate) data_dir: PathBuf,
    pub(crate) server: String,
    pub(crate) hostname: String,
    pub(crate) system: SystemState,
    pub(crate) network_mode: meshway_protocol::ipc::NetworkMode,
    pub(crate) underlay: meshway_underlay::Options,
    pub(crate) dns: meshway_dns::SystemDnsSettings,
}
impl Settings {
    pub fn new(
        data_dir: PathBuf,
        server: &str,
        hostname: Option<&str>,
    ) -> Result<Self, &'static str> {
        let server =
            meshway_control::server_origin(server).map_err(|_| "Invalid Server origin.")?;
        let system = rustix::system::uname();
        let machine_hostname = system
            .nodename()
            .to_str()
            .map_err(|_| "System hostname is not UTF-8.")?
            .to_owned();
        let hostname = hostname
            .unwrap_or_else(|| machine_hostname.split('.').next().unwrap_or("meshway"))
            .to_ascii_lowercase();
        if !meshway_configuration::valid_node_hostname(&hostname) {
            return Err("Use --hostname with a single DNS label.");
        }
        Ok(Self {
            data_dir,
            server,
            hostname,
            network_mode: meshway_protocol::ipc::NetworkMode::Tun,
            underlay: meshway_underlay::Options {
                mtu: meshway_dataplane::secure::DEFAULT_INNER_MTU,
                ..meshway_underlay::Options::default()
            },
            dns: meshway_dns::SystemDnsSettings::default(),
            system: SystemState {
                hostname: machine_hostname,
                os: std::env::consts::OS.into(),
                arch: std::env::consts::ARCH.into(),
                agent_version: env!("CARGO_PKG_VERSION").into(),
            },
        })
    }

    /// Select the local packet attachment mode. This does not override any
    /// Server-owned listener, Access, routing or DNS setting.
    pub fn with_network_mode(mut self, mode: meshway_protocol::ipc::NetworkMode) -> Self {
        self.network_mode = mode;
        self
    }

    /// Machine-local DNS attachment, independent of Server-owned DNS policy.
    pub fn with_dns_settings(
        mut self,
        settings: meshway_dns::SystemDnsSettings,
    ) -> Result<Self, &'static str> {
        settings
            .validate()
            .map_err(|_| "Invalid DNS installation settings.")?;
        if matches!(&settings.backend, meshway_dns::SystemDnsBackend::OwnedResolvConf { path } if path != std::path::Path::new("/etc/resolv.conf"))
        {
            return Err("The daemon resolv-conf backend only supports /etc/resolv.conf.");
        }
        self.dns = settings;
        Ok(self)
    }

    /// Machine bootstrap inputs for the protected Server/candidate resolver.
    /// These never replace the network's business DNS settings.
    pub fn with_bootstrap(
        mut self,
        hosts: std::collections::BTreeMap<String, Vec<std::net::IpAddr>>,
        nameservers: Option<Vec<std::net::SocketAddr>>,
    ) -> Self {
        self.underlay.bootstrap_hosts = hosts;
        self.underlay.bootstrap_nameservers = nameservers;
        self
    }
}
