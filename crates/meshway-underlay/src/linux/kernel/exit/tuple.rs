//! Kernel projection keys, independent of the data-plane business flow table.
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

use super::ExitError;
use meshway_model::exit::ExitUnavailableReason as Reason;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "protocol", deny_unknown_fields)]
pub enum ExitTransport {
    Tcp {
        source_port: u16,
        destination_port: u16,
    },
    Udp {
        source_port: u16,
        destination_port: u16,
    },
    IcmpEcho {
        identifier: u16,
    },
}

/// Exact original IPv4 conntrack tuple, before source translation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalTuple {
    pub source: Ipv4Addr,
    pub destination: Ipv4Addr,
    pub transport: ExitTransport,
}

impl OriginalTuple {
    pub(super) fn fields(self) -> (u8, u16, u16) {
        match self.transport {
            ExitTransport::Tcp {
                source_port,
                destination_port,
            } => (6, source_port, destination_port),
            ExitTransport::Udp {
                source_port,
                destination_port,
            } => (17, source_port, destination_port),
            ExitTransport::IcmpEcho { identifier } => (1, identifier, 0x0800),
        }
    }

    pub(super) fn key(self) -> serde_json::Value {
        let (protocol, source, destination) = self.fields();
        serde_json::json!({"concat": [self.source.to_string(), self.destination.to_string(), protocol, source, destination]})
    }

    pub(super) fn filter(self) -> Vec<String> {
        let (protocol, source, destination) = self.fields();
        let mut arguments = vec![
            "-f".into(),
            "ipv4".into(),
            "-p".into(),
            protocol.to_string(),
            "--orig-src".into(),
            self.source.to_string(),
            "--orig-dst".into(),
            self.destination.to_string(),
        ];
        if protocol == 1 {
            arguments.extend([
                "--icmp-type".into(),
                "8".into(),
                "--icmp-code".into(),
                "0".into(),
                "--icmp-id".into(),
                source.to_string(),
            ]);
        } else {
            arguments.extend([
                "--sport".into(),
                source.to_string(),
                "--dport".into(),
                destination.to_string(),
            ]);
        }
        arguments
    }

    /// Ownership framing only. Full packet/service admission remains upstream.
    pub(super) fn matches(self, packet: &[u8]) -> bool {
        if packet.len() < 28
            || packet[0] != 0x45
            || usize::from(u16::from_be_bytes([packet[2], packet[3]])) != packet.len()
            || u16::from_be_bytes([packet[6], packet[7]]) & 0x3fff != 0
            || packet[12..16] != self.source.octets()
            || packet[16..20] != self.destination.octets()
        {
            return false;
        }
        let (protocol, source, destination) = self.fields();
        if packet[9] != protocol {
            return false;
        }
        if protocol == 1 {
            packet[20] == 8
                && packet[21] == 0
                && u16::from_be_bytes([packet[24], packet[25]]) == source
        } else {
            (protocol != 6 || packet.len() >= 40)
                && u16::from_be_bytes([packet[20], packet[21]]) == source
                && u16::from_be_bytes([packet[22], packet[23]]) == destination
        }
    }

    pub(super) fn validate(
        self,
        pools: &[ipnet::Ipv4Net],
        local: &[Ipv4Addr],
    ) -> Result<(), ExitError> {
        if !pools.iter().any(|pool| pool.contains(&self.source))
            || local.contains(&self.source)
            || pools.iter().any(|pool| pool.contains(&self.destination))
            || local.contains(&self.destination)
            || forbidden(self.destination)
            || forbidden(self.source)
        {
            return Err(Reason::OwnershipConflict.into());
        }
        Ok(())
    }
}

fn forbidden(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 0 || octets[0] == 127 || octets[0] >= 224 || (octets[0] == 169 && octets[1] == 254)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_is_exact_and_cannot_target_owned_or_special_addresses() {
        let tuple = OriginalTuple {
            source: "100.96.0.1".parse().unwrap(),
            destination: "192.168.1.2".parse().unwrap(),
            transport: ExitTransport::Udp {
                source_port: 3456,
                destination_port: 443,
            },
        };
        let pools = ["100.96.0.0/11".parse().unwrap()];
        assert!(tuple.validate(&pools, &[]).is_ok());
        for destination in [
            "100.96.0.2",
            "127.0.0.1",
            "169.254.1.1",
            "224.0.0.1",
            "255.255.255.255",
            "0.1.2.3",
        ] {
            assert!(
                OriginalTuple {
                    destination: destination.parse().unwrap(),
                    ..tuple
                }
                .validate(&pools, &[])
                .is_err()
            );
        }
        assert!(tuple.validate(&pools, &[tuple.destination]).is_err());
        let filter = tuple.filter();
        assert_eq!(
            filter,
            [
                "-f",
                "ipv4",
                "-p",
                "17",
                "--orig-src",
                "100.96.0.1",
                "--orig-dst",
                "192.168.1.2",
                "--sport",
                "3456",
                "--dport",
                "443"
            ]
        );
    }
}
