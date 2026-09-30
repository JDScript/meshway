//! Linux-only TUN adapter for a fixed, explicitly authorized experimental IP pair.

use std::{io, net::Ipv4Addr};

use serde::{Deserialize, Serialize};

#[cfg(any(target_os = "linux", test))]
use super::packet;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TunConfig {
    pub name: String,
    pub local_ipv4: Ipv4Addr,
    pub peer_ipv4: Ipv4Addr,
    /// Exact directional paths in the experiment's static authorization table.
    pub send_path: Vec<String>,
    pub receive_path: Vec<String>,
    pub mtu: u16,
}

impl TunConfig {
    pub fn validate(&self, maximum_payload: usize) -> io::Result<()> {
        if self.name.is_empty()
            || self.name.len() > 15
            || !self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
            || self.local_ipv4 == self.peer_ipv4
            || [self.local_ipv4, self.peer_ipv4].iter().any(|ip| {
                ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() || ip.is_broadcast()
            })
            || self.send_path.is_empty()
            || self.receive_path.is_empty()
            || !(576..=1400).contains(&self.mtu)
            || usize::from(self.mtu) > maximum_payload
        {
            return Err(io::Error::other("invalid experimental TUN configuration"));
        }
        Ok(())
    }

    #[cfg(any(target_os = "linux", test))]
    pub fn accepts_outgoing(&self, bytes: &[u8]) -> bool {
        bytes.len() <= usize::from(self.mtu)
            && packet::accepts(bytes, self.local_ipv4, self.peer_ipv4)
    }

    #[cfg(any(target_os = "linux", test))]
    pub fn accepts_incoming(&self, path: &[String], bytes: &[u8]) -> bool {
        path == self.receive_path
            && bytes.len() <= usize::from(self.mtu)
            && packet::accepts(bytes, self.peer_ipv4, self.local_ipv4)
    }
}

#[cfg(target_os = "linux")]
pub fn open(config: &TunConfig) -> io::Result<tun_rs::AsyncDevice> {
    tun_rs::DeviceBuilder::new()
        .name(&config.name)
        .mtu(config.mtu)
        .ipv4(config.local_ipv4, 32, Some(config.peer_ipv4))
        .offload(false)
        .build_async()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_endpoint_pair_or_payload_budget_cannot_create_a_tun() {
        let mut config = TunConfig {
            name: "mwlab0".into(),
            local_ipv4: Ipv4Addr::new(198, 18, 0, 1),
            peer_ipv4: Ipv4Addr::new(198, 18, 0, 2),
            send_path: vec!["nyc".into(), "lax".into(), "sha".into()],
            receive_path: vec!["sha".into(), "lax".into(), "nyc".into()],
            mtu: 1280,
        };
        assert!(config.validate(1280).is_ok());
        assert!(!config.accepts_outgoing(&[]));
        assert!(!config.accepts_incoming(&config.receive_path, &[]));
        assert!(config.validate(1279).is_err());
        config.peer_ipv4 = config.local_ipv4;
        assert!(config.validate(1280).is_err());
        config.peer_ipv4 = Ipv4Addr::LOCALHOST;
        assert!(config.validate(1280).is_err());
    }
}
