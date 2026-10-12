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
        sequence: u16,
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
            ExitTransport::IcmpEcho { identifier, .. } => (1, identifier, 0x0800),
        }
    }

    pub(super) fn key(self) -> serde_json::Value {
        let (protocol, source, destination) = self.fields();
        if protocol == 1 {
            serde_json::json!({"concat": [self.source.to_string(), self.destination.to_string(), source, self.sequence()]})
        } else {
            serde_json::json!({"concat": [self.source.to_string(), self.destination.to_string(), protocol, source, destination]})
        }
    }
    pub(super) fn set(self) -> &'static str {
        if self.fields().0 == 1 {
            "echo_flows"
        } else {
            "flows"
        }
    }

    pub(super) fn sequence(self) -> u16 {
        match self.transport {
            ExitTransport::IcmpEcho { sequence, .. } => sequence,
            _ => 0,
        }
    }
    pub(super) fn same_connection(self, other: Self) -> bool {
        self.source == other.source
            && self.destination == other.destination
            && self.fields() == other.fields()
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
        self.header(packet)
            && usize::from(u16::from_be_bytes([packet[2], packet[3]])) == packet.len()
            && (self.fields().0 != 6 || packet.len() >= 40)
    }

    /// Whether `packet` is an ICMPv4 destination-unreachable or time-exceeded
    /// error addressed to this projection's source, quoting its forward packet
    /// (the quote is truncated; its sender may be any router). Framing only:
    /// the data-plane flow owner admits it.
    pub(super) fn related(self, packet: &[u8]) -> bool {
        packet.len() >= 20 + 8 + 28
            && packet[0] == 0x45
            && usize::from(u16::from_be_bytes([packet[2], packet[3]])) == packet.len()
            && u16::from_be_bytes([packet[6], packet[7]]) & 0x3fff == 0
            && packet[9] == 1
            && packet[16..20] == self.source.octets()
            && matches!(packet[20], 3 | 11)
            && self.header(&packet[28..])
    }

    /// The forward packet's header and ports (echo: identifier/sequence).
    fn header(self, packet: &[u8]) -> bool {
        if packet.len() < 28
            || packet[0] != 0x45
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
                && u16::from_be_bytes([packet[26], packet[27]]) == self.sequence()
        } else {
            u16::from_be_bytes([packet[20], packet[21]]) == source
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

    fn ipv4(source: Ipv4Addr, destination: Ipv4Addr, protocol: u8, body: &[u8]) -> Vec<u8> {
        let mut packet = vec![0x45, 0, 0, 0, 0, 0, 0x40, 0, 64, protocol, 0, 0];
        packet.extend(source.octets());
        packet.extend(destination.octets());
        packet.extend(body);
        let length = u16::try_from(packet.len()).unwrap().to_be_bytes();
        packet[2..4].copy_from_slice(&length);
        packet
    }

    #[test]
    fn related_errors_quote_the_exact_forward_packet_to_its_source() {
        let source: Ipv4Addr = "100.96.0.1".parse().unwrap();
        let destination: Ipv4Addr = "198.51.100.7".parse().unwrap();
        let router: Ipv4Addr = "203.0.113.1".parse().unwrap();
        let udp = OriginalTuple {
            source,
            destination,
            transport: ExitTransport::Udp {
                source_port: 3456,
                destination_port: 443,
            },
        };
        let forward = ipv4(source, destination, 17, &[13, 128, 1, 187, 0, 8, 0, 0]);
        assert!(udp.matches(&forward));
        let error = |kind: u8, to: Ipv4Addr, quoted: &[u8]| {
            let mut body = vec![kind, 0, 0, 0, 0, 0, 0, 0];
            body.extend(quoted);
            ipv4(router, to, 1, &body)
        };
        assert!(udp.related(&error(11, source, &forward)));
        assert!(udp.related(&error(3, source, &forward)));
        assert!(!udp.related(&error(5, source, &forward)));
        assert!(!udp.related(&error(11, destination, &forward)));
        let other = ipv4(source, destination, 17, &[13, 129, 1, 187, 0, 8, 0, 0]);
        assert!(!udp.related(&error(11, source, &other)));
        assert!(!udp.related(&error(11, source, &forward[..27])));
        let echo = OriginalTuple {
            source,
            destination,
            transport: ExitTransport::IcmpEcho {
                identifier: 7,
                sequence: 9,
            },
        };
        let request = ipv4(source, destination, 1, &[8, 0, 0, 0, 0, 7, 0, 9]);
        assert!(echo.related(&error(11, source, &request)));
        let later = ipv4(source, destination, 1, &[8, 0, 0, 0, 0, 7, 0, 10]);
        assert!(!echo.related(&error(11, source, &later)));
        // An error is never mistaken for the exact reply.
        assert!(!udp.matches(&error(11, source, &forward)));
    }

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
