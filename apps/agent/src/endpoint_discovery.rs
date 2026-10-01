//! Explicit observer configuration shared by the managed CLI and library API.
use std::{collections::BTreeSet, net::SocketAddr};

/// Empty means disabled. Observers are numeric unicast endpoints with no defaults.
#[derive(Clone, Debug, Default)]
pub struct EndpointDiscoveryOptions {
    pub(crate) servers: Vec<SocketAddr>,
}

impl EndpointDiscoveryOptions {
    pub fn new(servers: Vec<SocketAddr>) -> Result<Self, String> {
        if servers.len() > meshway_protocol::MAX_DISCOVERY_OBSERVERS {
            return Err("--stun-server accepts at most four observers".into());
        }
        let mut seen = BTreeSet::new();
        for server in &servers {
            crate::netcheck::validate_address(*server, false)?;
            if !seen.insert(*server) {
                return Err("duplicate STUN server endpoint".into());
            }
        }
        Ok(Self { servers })
    }
}

pub fn parse_stun_server(value: &str) -> Result<SocketAddr, String> {
    let address = crate::netcheck::parse_endpoint(value)?;
    crate::netcheck::validate_address(address, false)?;
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_numeric_unicast_and_bounded_observers() {
        for invalid in [
            "stun.example:3478",
            "0.0.0.0:3478",
            "127.0.0.1:0",
            "224.0.0.1:3478",
            "255.255.255.255:3478",
            "[::ffff:127.0.0.1]:3478",
            "[fe80::1%3]:3478",
            "[::]:3478",
        ] {
            assert!(parse_stun_server(invalid).is_err(), "{invalid}");
        }
        let v4 = parse_stun_server("127.0.0.1:3478").unwrap();
        let v6 = parse_stun_server("[::1]:3478").unwrap();
        assert!(EndpointDiscoveryOptions::new(vec![v4, v6]).is_ok());
        assert!(EndpointDiscoveryOptions::new(vec![v4, v4]).is_err());
        assert!(
            EndpointDiscoveryOptions::new(
                (1..=5)
                    .map(|port| std::net::SocketAddr::new(v4.ip(), port))
                    .collect()
            )
            .is_err()
        );
        assert!(EndpointDiscoveryOptions::default().servers.is_empty());
    }
}
