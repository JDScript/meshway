use std::{fmt, io, net::SocketAddr, path::Path};

use meshway_dataplane::{LabConfig, MAX_PACKET_PAYLOAD, TransportKind};
use meshway_protocol::local;
use meshway_transport_vless::{ClientConfig, SecretUuid, ServerConfig, Target};
use serde::{Deserialize, Serialize};

use super::{Result, tun::TunConfig};

/// Serialized only into the private experiment configuration, never status.
#[derive(Clone, Serialize)]
pub struct Credential(String);

impl Credential {
    pub fn new(value: String) -> Result<Self> {
        let _: SecretUuid = value.parse()?;
        Ok(Self(value))
    }

    fn parsed(&self) -> Result<SecretUuid> {
        Ok(self.0.parse()?)
    }
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credential([REDACTED])")
    }
}

impl<'de> Deserialize<'de> for Credential {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(|_| serde::de::Error::custom("invalid VLESS credential"))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VlessListener {
    pub bind: SocketAddr,
    pub credential: Credential,
    pub target: SocketAddr,
}

impl VlessListener {
    pub fn adapter(&self) -> Result<ServerConfig> {
        Ok(ServerConfig {
            uuid: self.credential.parsed()?,
            expected_target: Target::ip(self.target.ip(), self.target.port())?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VlessOutgoing {
    pub peer: String,
    pub endpoint: SocketAddr,
    pub credential: Credential,
    pub target: SocketAddr,
    pub priority: u16,
}

impl VlessOutgoing {
    pub fn adapter(&self) -> Result<ClientConfig> {
        Ok(ClientConfig {
            uuid: self.credential.parsed()?,
            target: Target::ip(self.target.ip(), self.target.port())?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub dataplane: LabConfig,
    #[serde(default)]
    pub vless_listeners: Vec<VlessListener>,
    #[serde(default)]
    pub vless_outgoing: Vec<VlessOutgoing>,
    #[serde(default)]
    pub tun: Option<TunConfig>,
}

impl Config {
    pub fn read(path: &Path) -> Result<Self> {
        let raw = local::read_private_bounded(path, 128 * 1024)?;
        // A serde error can include an attacker-controlled value or field name.
        // Never repeat secret-bearing input in CLI logs.
        let config: Self = serde_json::from_str(&raw).map_err(|error| {
            io::Error::other(format!(
                "invalid lab JSON at line {}, column {}",
                error.line(),
                error.column()
            ))
        })?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        self.dataplane.validate()?;
        if self.schema_version != 1
            || self.vless_listeners.len() > 4
            || self.vless_outgoing.len() > 16
            || self
                .dataplane
                .outgoing
                .iter()
                .any(|item| item.transport == TransportKind::Vless)
        {
            return Err(io::Error::other("invalid lab schema or carrier configuration").into());
        }
        for listener in &self.vless_listeners {
            listener.adapter()?;
        }
        for outgoing in &self.vless_outgoing {
            outgoing.adapter()?;
            if outgoing.endpoint.port() == 0
                || !self.dataplane.trusted_peers.iter().any(|peer| {
                    peer.node_id == outgoing.peer && peer.transports.contains(&TransportKind::Vless)
                })
            {
                return Err(io::Error::other("invalid VLESS outgoing peer or endpoint").into());
            }
        }
        // The experimental CLI maintains one VLESS channel per peer. Avoid
        // silently ignoring additional candidates while any channel is alive.
        let mut vless_peers = std::collections::BTreeSet::new();
        if self.vless_outgoing.iter().any(|candidate| {
            !vless_peers.insert(&candidate.peer)
                || self
                    .dataplane
                    .outgoing
                    .iter()
                    .any(|other| other.peer == candidate.peer)
        }) {
            return Err(io::Error::other(
                "lab CLI supports one VLESS candidate per peer, without mixed native candidates",
            )
            .into());
        }
        if let Some(tun) = &self.tun {
            tun.validate(MAX_PACKET_PAYLOAD)?;
            let captures_underlay = |address: std::net::IpAddr| {
                address == std::net::IpAddr::V4(tun.local_ipv4)
                    || address == std::net::IpAddr::V4(tun.peer_ipv4)
            };
            if self
                .vless_outgoing
                .iter()
                .any(|item| captures_underlay(item.endpoint.ip()))
                || self.dataplane.outgoing.iter().any(|item| {
                    item.endpoint
                        .parse::<SocketAddr>()
                        .is_ok_and(|address| captures_underlay(address.ip()))
                })
            {
                return Err(io::Error::other(
                    "TUN endpoint pair would capture its own carrier traffic",
                )
                .into());
            }
            let node = &self.dataplane.node_id;
            if tun.send_path.first() != Some(node)
                || tun.receive_path.last() != Some(node)
                || tun.send_path.last() != tun.receive_path.first()
                || !self.dataplane.allowed_paths.contains(&tun.send_path)
                || !self.dataplane.allowed_paths.contains(&tun.receive_path)
            {
                return Err(io::Error::other(
                    "TUN paths must explicitly authorize this endpoint pair",
                )
                .into());
            }
            // The lab uses an IPv4/1500 underlay. Keep the complete encrypted UDP
            // frame within it, including the variable-length ordered path.
            for path in [&tun.send_path, &tun.receive_path] {
                if meshway_dataplane::native_udp_datagram_len(path, usize::from(tun.mtu))? + 48
                    > 1500
                {
                    return Err(
                        io::Error::other("TUN MTU exceeds the lab's outer packet budget").into(),
                    );
                }
            }
        }
        Ok(())
    }
}
