//! The ICMP error this Node itself reports for one of its own packets that
//! does not fit the path: IPv4 fragmentation needed (3/4) or ICMPv6 packet
//! too big (2/0), as a router on that path would send it; ICMPv6
//! destination unreachable, no route (1/0), for a path that cannot carry
//! IPv6 at all (a channel below the IPv6 minimum MTU); and ICMPv6
//! destination unreachable, administratively prohibited (1/1), for an IPv6
//! exit destination no exit of its decision is ready to carry.
use super::*;

/// IPv6 minimum link MTU (RFC 8200): a channel below it carries no IPv6.
pub(crate) const IPV6_MIN_MTU: u16 = 1280;

/// Bytes of the offending packet an IPv6 error quotes at most: what keeps
/// the error within the IPv6 minimum MTU.
const QUOTE6_MAX: usize = 1280 - 48;

/// The error reporting that `packet` exceeds `mtu`, from its destination to
/// its source. `None` for a packet no error may quote (an ICMP error or echo
/// reply itself): it is only dropped. No IPv6 error names an MTU below the
/// IPv6 minimum: such a path gets [`no_route`] instead, since a packet too
/// big naming 1,280 would never let the sender fit.
pub(crate) fn too_big(packet: &Packet, mtu: u16) -> Option<Packet> {
    if packet.traffic().destination.is_ipv6() && mtu < IPV6_MIN_MTU {
        return None;
    }
    error(packet, (3, 4, mtu), (2, 0, u32::from(mtu)))
}

/// ICMPv6 destination unreachable (no route) for an IPv6 `packet` whose path
/// carries no IPv6. `None` for IPv4 or a packet no error may quote.
pub(crate) fn no_route(packet: &Packet) -> Option<Packet> {
    if !packet.traffic().destination.is_ipv6() {
        return None;
    }
    error(packet, (3, 4, 0), (1, 0, 0))
}

/// ICMPv6 destination unreachable (administratively prohibited) for an IPv6
/// `packet` refused before any flow: Linux reports it to the socket as
/// `EACCES`, a hard error, so `connect` fails at once and the application
/// falls back to its next address. `None` for IPv4 or a packet no error may
/// quote.
pub(crate) fn prohibited(packet: &Packet) -> Option<Packet> {
    if !packet.traffic().destination.is_ipv6() {
        return None;
    }
    error(packet, (3, 13, 0), (1, 1, 0))
}

/// `(type, code, next-hop MTU)` for IPv4 and `(type, code, 32-bit field)`
/// for ICMPv6.
fn error(packet: &Packet, v4: (u8, u8, u16), v6: (u8, u8, u32)) -> Option<Packet> {
    if !matches!(
        packet.transport(),
        Transport::Tcp(_) | Transport::Udp | Transport::Echo { reply: false, .. }
    ) {
        return None;
    }
    let original = packet.as_bytes();
    let traffic = packet.traffic();
    let bytes = match (traffic.source, traffic.destination) {
        (IpAddr::V4(source), IpAddr::V4(destination)) => {
            let quote = &original[..ERROR_QUOTE];
            let total = 20 + 8 + quote.len();
            let mut bytes = Vec::with_capacity(total);
            bytes.extend_from_slice(&[0x45, 0]);
            bytes.extend_from_slice(&(total as u16).to_be_bytes());
            bytes.extend_from_slice(&[0, 0, 0, 0, 64, 1, 0, 0]);
            bytes.extend_from_slice(&destination.octets());
            bytes.extend_from_slice(&source.octets());
            let header = finish(sum(0, &bytes[..20]));
            bytes[10..12].copy_from_slice(&header.to_be_bytes());
            bytes.extend_from_slice(&[v4.0, v4.1, 0, 0, 0, 0]);
            bytes.extend_from_slice(&v4.2.to_be_bytes());
            bytes.extend_from_slice(quote);
            let checksum = finish(sum(0, &bytes[20..]));
            bytes[22..24].copy_from_slice(&checksum.to_be_bytes());
            bytes
        }
        (IpAddr::V6(source), IpAddr::V6(destination)) => {
            // Only a quote whose next header is the transport itself.
            if original[6] != 6 && original[6] != 17 && original[6] != 58 {
                return None;
            }
            let quote = &original[..original.len().min(QUOTE6_MAX)];
            let length = 8 + quote.len();
            let mut bytes = Vec::with_capacity(40 + length);
            bytes.extend_from_slice(&[0x60, 0, 0, 0]);
            bytes.extend_from_slice(&(length as u16).to_be_bytes());
            bytes.extend_from_slice(&[58, 64]);
            bytes.extend_from_slice(&destination.octets());
            bytes.extend_from_slice(&source.octets());
            bytes.extend_from_slice(&[v6.0, v6.1, 0, 0]);
            bytes.extend_from_slice(&v6.2.to_be_bytes());
            bytes.extend_from_slice(quote);
            let checksum = finish(sum(
                pseudo(destination.into(), source.into(), 58, length),
                &bytes[40..],
            ));
            bytes[42..44].copy_from_slice(&checksum.to_be_bytes());
            bytes
        }
        _ => return None,
    };
    Packet::parse(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tcp(source: &str, destination: &str, payload: usize) -> Packet {
        let (source, destination): (IpAddr, IpAddr) =
            (source.parse().unwrap(), destination.parse().unwrap());
        let mut segment = vec![0u8; 20 + payload];
        segment[..4].copy_from_slice(&[0x30, 0x39, 0x01, 0xbb]);
        segment[12] = 5 << 4;
        segment[13] = 0x10;
        let checksum = finish(sum(pseudo(source, destination, 6, segment.len()), &segment));
        segment[16..18].copy_from_slice(&checksum.to_be_bytes());
        let mut bytes = match (source, destination) {
            (IpAddr::V4(s), IpAddr::V4(d)) => {
                let mut header = vec![0x45, 0];
                header.extend_from_slice(&((20 + segment.len()) as u16).to_be_bytes());
                header.extend_from_slice(&[0, 0, 0x40, 0, 64, 6, 0, 0]);
                header.extend_from_slice(&s.octets());
                header.extend_from_slice(&d.octets());
                let checksum = finish(sum(0, &header));
                header[10..12].copy_from_slice(&checksum.to_be_bytes());
                header
            }
            (IpAddr::V6(s), IpAddr::V6(d)) => {
                let mut header = vec![0x60, 0, 0, 0];
                header.extend_from_slice(&(segment.len() as u16).to_be_bytes());
                header.extend_from_slice(&[6, 64]);
                header.extend_from_slice(&s.octets());
                header.extend_from_slice(&d.octets());
                header
            }
            _ => unreachable!(),
        };
        bytes.extend_from_slice(&segment);
        Packet::parse(bytes).unwrap()
    }

    #[test]
    fn a_too_big_error_quotes_the_packet_and_parses_as_its_flow_error() {
        for (source, destination, mtu, reported) in [
            ("100.64.0.1", "100.64.0.4", 1146, 1146),
            ("fd00::1", "fd00::4", 1280, 1280),
        ] {
            let packet = tcp(source, destination, 1400);
            let error = too_big(&packet, mtu).unwrap();
            let traffic = error.traffic();
            assert_eq!(traffic.source, packet.traffic().destination);
            assert_eq!(traffic.destination, packet.traffic().source);
            let Transport::Error(icmp) = error.transport() else {
                panic!("{error:?}");
            };
            assert_eq!(icmp.quoted.traffic, packet.traffic());
            let bytes = error.as_bytes();
            if traffic.source.is_ipv4() {
                assert_eq!((icmp.kind, icmp.code), (3, 4));
                assert_eq!(word(bytes, 26), reported);
                assert_eq!(bytes.len(), 56);
            } else {
                assert_eq!((icmp.kind, icmp.code), (2, 0));
                assert_eq!(word32(bytes, 44), u32::from(reported));
                assert!(bytes.len() <= 1280);
            }
            assert!(error.facts().validate().is_ok());
        }
    }

    /// Below the IPv6 minimum no packet too big is made (naming 1,280 would
    /// never converge); the path reports no route instead.
    #[test]
    fn an_ipv6_path_below_the_minimum_reports_no_route() {
        let packet = tcp("fd00::1", "fd00::4", 1000);
        assert!(too_big(&packet, 1146).is_none());
        let error = no_route(&packet).unwrap();
        let Transport::Error(icmp) = error.transport() else {
            panic!("{error:?}");
        };
        assert_eq!((icmp.kind, icmp.code), (1, 0));
        assert_eq!(icmp.quoted.traffic, packet.traffic());
        assert_eq!(error.traffic().destination, packet.traffic().source);
        assert!(error.facts().validate().is_ok());
        assert!(no_route(&tcp("100.64.0.1", "100.64.0.4", 100)).is_none());
    }

    #[test]
    fn a_refused_ipv6_exit_packet_is_answered_administratively_prohibited() {
        let packet = tcp("fd00::1", "2606:4700:4700::1111", 100);
        let error = prohibited(&packet).unwrap();
        let Transport::Error(icmp) = error.transport() else {
            panic!("{error:?}");
        };
        assert_eq!((icmp.kind, icmp.code), (1, 1));
        assert_eq!(icmp.quoted.traffic, packet.traffic());
        assert_eq!(error.traffic().destination, packet.traffic().source);
        assert!(error.facts().validate().is_ok());
        // IPv4 refusals stay silent drops.
        assert!(prohibited(&tcp("100.64.0.1", "8.8.8.8", 100)).is_none());
        assert!(prohibited(&error).is_none());
    }

    #[test]
    fn no_error_quotes_an_error() {
        let packet = tcp("100.64.0.1", "100.64.0.4", 1400);
        let error = too_big(&packet, 1330).unwrap();
        assert!(too_big(&error, 1280).is_none());
    }
}
