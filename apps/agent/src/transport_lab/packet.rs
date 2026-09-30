//! Fixed IPv4 endpoint boundary for the isolated TUN experiment.
//! This is deliberately not a policy compiler, subnet router, or NAT engine.

use std::net::Ipv4Addr;

/// Accept one complete, unfragmented packet for the explicitly configured pair.
/// The caller binds this pair to the authenticated end-to-end source/path.
pub fn accepts(packet: &[u8], source: Ipv4Addr, destination: Ipv4Addr) -> bool {
    if packet.len() < 20 || packet[0] >> 4 != 4 {
        return false;
    }
    let header_len = usize::from(packet[0] & 0x0f) * 4;
    // This fixed-pair experiment does not interpret IP options (including source
    // routes) or nested IP tunnels. Keep that boundary explicit before injection.
    if header_len != 20 || !matches!(packet[9], 1 | 6 | 17) {
        return false;
    }
    let length = usize::from(u16::from_be_bytes([packet[2], packet[3]]));
    let fragment = u16::from_be_bytes([packet[6], packet[7]]);
    if length != packet.len() || fragment & 0xbfff != 0 || packet[8] == 0 {
        return false;
    }
    if packet[12..16] != source.octets() || packet[16..20] != destination.octets() {
        return false;
    }
    let mut checksum = packet[..header_len]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|word| u32::from(u16::from_be_bytes([word[0], word[1]])))
        .sum::<u32>();
    while checksum > 0xffff {
        checksum = (checksum & 0xffff) + (checksum >> 16);
    }
    checksum == 0xffff
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL: Ipv4Addr = Ipv4Addr::new(198, 18, 0, 1);
    const PEER: Ipv4Addr = Ipv4Addr::new(198, 18, 0, 2);

    fn packet(protocol: u8, flags: u16) -> Vec<u8> {
        let mut value = vec![0u8; 28];
        value[0] = 0x45;
        value[2..4].copy_from_slice(&28u16.to_be_bytes());
        value[6..8].copy_from_slice(&flags.to_be_bytes());
        value[8] = 64;
        value[9] = protocol;
        value[12..16].copy_from_slice(&LOCAL.octets());
        value[16..20].copy_from_slice(&PEER.octets());
        let sum: u32 = value[..20]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|word| u32::from(u16::from_be_bytes([word[0], word[1]])))
            .sum();
        let sum = (sum & 0xffff) + (sum >> 16);
        let checksum = !((sum & 0xffff) + (sum >> 16)) as u16;
        value[10..12].copy_from_slice(&checksum.to_be_bytes());
        value
    }

    #[test]
    fn accepts_icmp_tcp_udp_only_for_the_authorized_ip_direction() {
        for protocol in [1, 6, 17] {
            let value = packet(protocol, 0x4000);
            assert!(accepts(&value, LOCAL, PEER));
            assert!(!accepts(&value, PEER, LOCAL));
            assert!(!accepts(&value, LOCAL, Ipv4Addr::LOCALHOST));
        }
    }

    #[test]
    fn rejects_fragments_truncation_trailing_bytes_and_bad_headers() {
        for flags in [0x8000, 0x2000, 0x0001] {
            assert!(!accepts(&packet(6, flags), LOCAL, PEER));
        }
        let valid = packet(17, 0);
        for length in 0..valid.len() {
            assert!(!accepts(&valid[..length], LOCAL, PEER));
        }
        let mut extended = valid.clone();
        extended.push(0);
        assert!(!accepts(&extended, LOCAL, PEER));
        for (offset, value) in [(0, 0x65), (0, 0x44), (0, 0x4f), (8, 0), (10, 0)] {
            let mut malformed = valid.clone();
            malformed[offset] = value;
            assert!(!accepts(&malformed, LOCAL, PEER));
        }
        for protocol in [4, 41, 47] {
            assert!(!accepts(&packet(protocol, 0), LOCAL, PEER));
        }
    }
}
