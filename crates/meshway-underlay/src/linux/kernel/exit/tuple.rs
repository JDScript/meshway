//! Kernel projection keys, independent of the data-plane business flow table.
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

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

/// Exact original conntrack tuple, before source translation; source and
/// destination are of one family.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalTuple {
    pub source: IpAddr,
    pub destination: IpAddr,
    pub transport: ExitTransport,
}

/// A packet's network header: addresses, transport protocol and where the
/// transport header starts. IPv4 without options or fragmentation; IPv6
/// whose next header is the transport itself (no extension headers). `total`
/// also requires the length field to match the packet (a quote is cut).
fn network(packet: &[u8], total: bool) -> Option<(IpAddr, IpAddr, u8, usize)> {
    match packet.first()? >> 4 {
        4 if packet.len() >= 20 && packet[0] == 0x45 => {
            if u16::from_be_bytes([packet[6], packet[7]]) & 0x3fff != 0
                || (total
                    && usize::from(u16::from_be_bytes([packet[2], packet[3]])) != packet.len())
            {
                return None;
            }
            let source: [u8; 4] = packet[12..16].try_into().ok()?;
            let destination: [u8; 4] = packet[16..20].try_into().ok()?;
            Some((source.into(), destination.into(), packet[9], 20))
        }
        6 if packet.len() >= 40 => {
            if total && usize::from(u16::from_be_bytes([packet[4], packet[5]])) + 40 != packet.len()
            {
                return None;
            }
            let source: [u8; 16] = packet[8..24].try_into().ok()?;
            let destination: [u8; 16] = packet[24..40].try_into().ok()?;
            Some((source.into(), destination.into(), packet[6], 40))
        }
        _ => None,
    }
}

/// The ICMP protocol number of the family of `address`.
pub(super) fn icmp(address: IpAddr) -> u8 {
    if address.is_ipv4() { 1 } else { 58 }
}

impl OriginalTuple {
    /// (protocol, source key, destination key) as conntrack names them; for
    /// echo the identifier and the request's type in the high byte.
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
            ExitTransport::IcmpEcho { identifier, .. } if self.source.is_ipv4() => {
                (1, identifier, 0x0800)
            }
            ExitTransport::IcmpEcho { identifier, .. } => (58, identifier, 0x8000),
        }
    }

    pub(super) fn echo(self) -> bool {
        matches!(self.transport, ExitTransport::IcmpEcho { .. })
    }

    pub(super) fn key(self) -> serde_json::Value {
        let (protocol, source, destination) = self.fields();
        if self.echo() {
            serde_json::json!({"concat": [self.source.to_string(), self.destination.to_string(), source, self.sequence()]})
        } else {
            serde_json::json!({"concat": [self.source.to_string(), self.destination.to_string(), protocol, source, destination]})
        }
    }
    pub(super) fn set(self) -> &'static str {
        match (self.echo(), self.source.is_ipv4()) {
            (true, true) => "echo_flows",
            (false, true) => "flows",
            (true, false) => "echo_flows6",
            (false, false) => "flows6",
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

    /// Ownership framing only. Full packet/service admission remains upstream.
    pub(super) fn matches(self, packet: &[u8]) -> bool {
        network(packet, true).is_some()
            && self.header(packet)
            && (self.fields().0 != 6 || packet.len() >= self.transport_offset() + 20)
    }

    fn transport_offset(self) -> usize {
        if self.source.is_ipv4() { 20 } else { 40 }
    }

    /// Whether `packet` is an ICMP error addressed to this projection's
    /// source quoting its forward packet: ICMPv4 destination unreachable or
    /// time exceeded, ICMPv6 destination unreachable, packet too big or time
    /// exceeded (the quote is truncated; its sender may be any router).
    /// Framing only: the data-plane flow owner admits it.
    pub(super) fn related(self, packet: &[u8]) -> bool {
        let Some((_, to, protocol, offset)) = network(packet, true) else {
            return false;
        };
        let kinds: &[u8] = if self.source.is_ipv4() {
            &[3, 11]
        } else {
            &[1, 2, 3]
        };
        to == self.source
            && protocol == icmp(self.source)
            && packet.len() >= offset + 8 + self.transport_offset() + 8
            && kinds.contains(&packet[offset])
            && self.header(&packet[offset + 8..])
    }

    /// The forward packet's header and ports (echo: identifier/sequence).
    fn header(self, packet: &[u8]) -> bool {
        let Some((source, destination, protocol, offset)) = network(packet, false) else {
            return false;
        };
        if source != self.source || destination != self.destination || packet.len() < offset + 8 {
            return false;
        }
        let (expected, source, destination) = self.fields();
        if protocol != expected {
            return false;
        }
        let at =
            |index: usize| u16::from_be_bytes([packet[offset + index], packet[offset + index + 1]]);
        if self.echo() {
            packet[offset] == (destination >> 8) as u8
                && packet[offset + 1] == 0
                && at(4) == source
                && at(6) == self.sequence()
        } else {
            at(0) == source && at(2) == destination
        }
    }

    /// The exact reverse-translated reply of this projection.
    pub(super) fn reply(self, packet: &[u8]) -> bool {
        let Some((source, destination, protocol, offset)) = network(packet, true) else {
            return false;
        };
        if source != self.destination || destination != self.source || packet.len() < offset + 8 {
            return false;
        }
        let (expected, from, to) = self.fields();
        if protocol != expected {
            return false;
        }
        let at =
            |index: usize| u16::from_be_bytes([packet[offset + index], packet[offset + index + 1]]);
        if self.echo() {
            // Echo reply: 0 (IPv4), 129 (IPv6).
            packet[offset] == if self.source.is_ipv4() { 0 } else { 129 }
                && packet[offset + 1] == 0
                && at(4) == from
                && at(6) == self.sequence()
        } else {
            (protocol != 6 || packet.len() >= offset + 20) && at(0) == to && at(2) == from
        }
    }

    pub(super) fn validate(
        self,
        pools: &[ipnet::IpNet],
        local: &[IpAddr],
    ) -> Result<(), ExitError> {
        if self.source.is_ipv4() != self.destination.is_ipv4()
            || !pools.iter().any(|pool| pool.contains(&self.source))
            || local.contains(&self.source)
            || pools.iter().any(|pool| pool.contains(&self.destination))
            || local.contains(&self.destination)
            || !meshway_policy::is_exit_destination(self.destination)
            || !meshway_policy::is_exit_destination(self.source)
        {
            return Err(Reason::OwnershipConflict.into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

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
            source: source.into(),
            destination: destination.into(),
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
            source: source.into(),
            destination: destination.into(),
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
        let pools: [ipnet::IpNet; 2] = [
            "100.96.0.0/11".parse().unwrap(),
            "fd79::/64".parse().unwrap(),
        ];
        assert!(tuple.validate(&pools, &[]).is_ok());
        for destination in [
            "100.96.0.2",
            "127.0.0.1",
            "169.254.1.1",
            "224.0.0.1",
            "255.255.255.255",
            "0.1.2.3",
        ] {
            let destination: IpAddr = destination.parse().unwrap();
            assert!(
                OriginalTuple {
                    destination,
                    ..tuple
                }
                .validate(&pools, &[])
                .is_err()
            );
        }
        assert!(tuple.validate(&pools, &[tuple.destination]).is_err());
    }

    fn ipv6(source: IpAddr, destination: IpAddr, protocol: u8, body: &[u8]) -> Vec<u8> {
        let (IpAddr::V6(source), IpAddr::V6(destination)) = (source, destination) else {
            unreachable!()
        };
        let mut packet = vec![0x60, 0, 0, 0];
        packet.extend(u16::try_from(body.len()).unwrap().to_be_bytes());
        packet.extend([protocol, 64]);
        packet.extend(source.octets());
        packet.extend(destination.octets());
        packet.extend(body);
        packet
    }

    #[test]
    fn ipv6_projections_frame_replies_echo_and_related_errors_by_their_own_family() {
        let source: IpAddr = "fd79::5".parse().unwrap();
        let destination: IpAddr = "3ffe:1:2:3::7".parse().unwrap();
        let router: IpAddr = "3ffe:1:2:4::1".parse().unwrap();
        let udp = OriginalTuple {
            source,
            destination,
            transport: ExitTransport::Udp {
                source_port: 3456,
                destination_port: 443,
            },
        };
        assert_eq!(udp.set(), "flows6");
        let forward = ipv6(source, destination, 17, &[13, 128, 1, 187, 0, 8, 0, 0]);
        assert!(udp.matches(&forward));
        let reply = ipv6(destination, source, 17, &[1, 187, 13, 128, 0, 8, 0, 0]);
        assert!(udp.reply(&reply));
        assert!(!udp.reply(&forward));
        let error = |kind: u8, to: IpAddr, quoted: &[u8]| {
            let mut body = vec![kind, 0, 0, 0, 0, 0, 5, 0];
            body.extend(quoted);
            ipv6(router, to, 58, &body)
        };
        for kind in [1, 2, 3] {
            assert!(udp.related(&error(kind, source, &forward)), "{kind}");
        }
        // Parameter problem and errors to another address are not related.
        assert!(!udp.related(&error(4, source, &forward)));
        assert!(!udp.related(&error(1, destination, &forward)));
        // An IPv4 tuple never frames an IPv6 packet.
        let v4 = OriginalTuple {
            source: "100.96.0.1".parse().unwrap(),
            destination: "198.51.100.7".parse().unwrap(),
            ..udp
        };
        assert!(!v4.matches(&forward) && !v4.related(&error(1, source, &forward)));
        let echo = OriginalTuple {
            source,
            destination,
            transport: ExitTransport::IcmpEcho {
                identifier: 7,
                sequence: 9,
            },
        };
        assert_eq!(
            (echo.set(), echo.fields()),
            ("echo_flows6", (58, 7, 0x8000))
        );
        let request = ipv6(source, destination, 58, &[128, 0, 0, 0, 0, 7, 0, 9]);
        assert!(echo.matches(&request));
        let answer = ipv6(destination, source, 58, &[129, 0, 0, 0, 0, 7, 0, 9]);
        assert!(echo.reply(&answer));
        assert!(echo.related(&error(3, source, &request)));
        let pools: [ipnet::IpNet; 1] = ["fd79::/64".parse().unwrap()];
        assert!(udp.validate(&pools, &[]).is_ok());
        for forbidden in [
            "::1",
            "fe80::1",
            "ff02::1",
            "::ffff:1.2.3.4",
            "64:ff9b::808:808",
            "fd79::9",
        ] {
            let destination: IpAddr = forbidden.parse().unwrap();
            assert!(
                OriginalTuple { destination, ..udp }
                    .validate(&pools, &[])
                    .is_err(),
                "{forbidden}"
            );
        }
    }
}
