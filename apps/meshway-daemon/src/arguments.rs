use clap::Parser;
#[derive(Parser)]
#[command(name = "meshwayd", version, about = "Meshway Node daemon")]
pub struct Arguments {
    #[command(flatten)]
    pub logging: meshway_logging::Options,
    /// Private machine state directory; its parent must already exist.
    #[arg(long, default_value = "/var/lib/meshway")]
    data_dir: std::path::PathBuf,
    /// Server origin. HTTPS is required outside numeric loopback.
    #[arg(long, env = "MESHWAY_SERVER_URL")]
    server: String,
    /// Browser console origin, when served separately from the Server.
    #[arg(long)]
    console_origin: Option<String>,
    /// Suggested label for first enrollment; later business settings live on Server.
    #[arg(long)]
    hostname: Option<String>,
    /// Local packet attachment. TUN requires Linux device/routing permissions;
    /// control-only explicitly maintains control without installing networking.
    #[arg(
        long,
        env = "MESHWAY_NETWORK_MODE",
        default_value = "tun",
        value_name = "tun|control-only"
    )]
    network_mode: meshway_ipc::NetworkMode,
    /// Fixed bootstrap mapping for the Server or configured carrier names.
    #[arg(long, value_name = "NAME=IP", value_parser = bootstrap_host)]
    bootstrap_host: Vec<(String, std::net::IpAddr)>,
    /// Real upstream for protected bootstrap DNS; repeat for multiple servers.
    #[arg(long, value_name = "IP:PORT")]
    bootstrap_nameserver: Vec<std::net::SocketAddr>,
    /// Host DNS installer; resolv-conf is an explicit operator choice.
    #[arg(long, value_enum, default_value = "resolved")]
    dns_backend: DnsBackend,
    /// Loopback DNS service address (resolved: 127.0.0.1:5354; resolv-conf: 127.0.0.1:53).
    #[arg(long)]
    dns_bind: Option<std::net::SocketAddr>,
    /// Address on the separately owned persistent DNS anchor.
    #[arg(long)]
    dns_anchor_address: Option<std::net::Ipv4Addr>,
    /// Owned resolver file; only /etc/resolv.conf is supported.
    #[arg(long)]
    dns_resolv_conf: Option<std::path::PathBuf>,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum DnsBackend {
    Resolved,
    ResolvConf,
}
impl Arguments {
    pub fn validate(self) -> Result<meshway_daemon::Settings, &'static str> {
        let mut dns = meshway_dns::SystemDnsSettings::default();
        match self.dns_backend {
            DnsBackend::Resolved => {
                if self.dns_resolv_conf.is_some() {
                    return Err("--dns-resolv-conf requires --dns-backend resolv-conf.");
                }
            }
            DnsBackend::ResolvConf => {
                dns.bind.set_port(53);
                dns.backend = meshway_dns::SystemDnsBackend::OwnedResolvConf {
                    path: self
                        .dns_resolv_conf
                        .unwrap_or_else(|| "/etc/resolv.conf".into()),
                };
            }
        }
        if let Some(bind) = self.dns_bind {
            dns.bind = bind;
        }
        if let Some(address) = self.dns_anchor_address {
            dns.anchor_address = address;
        }
        let mut hosts = std::collections::BTreeMap::<String, Vec<std::net::IpAddr>>::new();
        for (name, ip) in self.bootstrap_host {
            let addresses = hosts.entry(name).or_default();
            if !addresses.contains(&ip) {
                addresses.push(ip);
            }
        }
        let nameservers =
            (!self.bootstrap_nameserver.is_empty()).then_some(self.bootstrap_nameserver);
        meshway_daemon::Settings::new(
            self.data_dir,
            &self.server,
            self.console_origin.as_deref(),
            self.hostname.as_deref(),
        )
        .and_then(|settings| {
            settings
                .with_network_mode(self.network_mode)
                .with_bootstrap(hosts, nameservers)
                .with_dns_settings(dns)
        })
    }
}

fn bootstrap_host(value: &str) -> Result<(String, std::net::IpAddr), &'static str> {
    let (name, address) = value.split_once('=').ok_or("Expected NAME=IP.")?;
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    if name.len() > 253
        || name.is_empty()
        || !name
            .split('.')
            .all(meshway_configuration::valid_node_hostname)
    {
        return Err("Bootstrap host must be a DNS name.");
    }
    let address: std::net::IpAddr = address
        .parse()
        .map_err(|_| "Bootstrap value must be an IP address.")?;
    if address.is_unspecified() || address.is_multicast() {
        return Err("Bootstrap value must be a unicast address.");
    }
    Ok((name, address))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(extra: &[&str]) -> Arguments {
        Arguments::try_parse_from(
            [
                "meshwayd",
                "--server",
                "http://127.0.0.1:8430",
                "--hostname",
                "test-node",
            ]
            .into_iter()
            .chain(extra.iter().copied()),
        )
        .unwrap()
    }

    #[test]
    fn dns_backend_choice_has_explicit_defaults_and_rejects_incompatible_flags() {
        assert!(arguments(&[]).validate().is_ok());
        assert!(
            arguments(&["--dns-backend", "resolv-conf"])
                .validate()
                .is_ok()
        );
        for invalid in [
            vec!["--dns-resolv-conf", "/etc/resolv.conf"],
            vec![
                "--dns-backend",
                "resolv-conf",
                "--dns-bind",
                "127.0.0.1:5354",
            ],
            vec![
                "--dns-backend",
                "resolv-conf",
                "--dns-resolv-conf",
                "/private/resolv.conf",
            ],
            vec!["--dns-bind", "192.0.2.1:5354"],
        ] {
            assert!(arguments(&invalid).validate().is_err(), "{invalid:?}");
        }
    }
}
