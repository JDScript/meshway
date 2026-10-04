use super::*;
use crate::fixtures::*;

#[test]
fn rfc1071_checksum_and_owned_redacted_packet() {
    assert_eq!(
        finish(sum(0, &[0, 1, 0xf2, 3, 0xf4, 0xf5, 0xf6, 0xf7])),
        0x220d
    );
    let bytes = datagram(
        address(1),
        address(3),
        17,
        udp_body(50000, 443, b"secret payload"),
    );
    let pointer = bytes.as_ptr();
    let packet = Packet::parse(bytes).unwrap();
    assert_eq!(packet.as_bytes().as_ptr(), pointer);
    assert_eq!(packet.traffic().destination_port.unwrap().get(), 443);
    assert!(!format!("{packet:?}").contains("secret payload"));
    let returned = packet.into_bytes();
    assert_eq!(returned.as_ptr(), pointer);
}

#[test]
fn ipv4_exact_lengths_checksums_ports_and_fragment_rejection() {
    let valid = udp(1, 3, 50000, 443).into_bytes();
    for end in 0..valid.len() {
        assert!(
            Packet::parse(valid[..end].to_vec()).is_err(),
            "prefix {end}"
        );
    }
    let mut extra = valid.clone();
    extra.push(0);
    assert_eq!(Packet::parse(extra).unwrap_err(), PacketError::Length);
    for (index, value, error) in [
        (0, 0x44, PacketError::Length),
        (0, 0x46, PacketError::Unsupported),
        (0, 0x75, PacketError::Unsupported),
        (6, 0x20, PacketError::Fragment),
        (7, 1, PacketError::Fragment),
        (6, 0x80, PacketError::Header),
        (8, 0, PacketError::Header),
        (10, valid[10] ^ 1, PacketError::Checksum),
    ] {
        let mut bad = valid.clone();
        bad[index] = value;
        assert_eq!(Packet::parse(bad).unwrap_err(), error);
    }
    let mut corrupt = valid.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert_eq!(Packet::parse(corrupt).unwrap_err(), PacketError::Checksum);
    // IPv4 UDP legitimately allows a zero checksum, IPv6 does not.
    let mut absent = valid;
    absent[26..28].fill(0);
    assert!(Packet::parse(absent).is_ok());
    for body in [udp_body(0, 443, &[]), udp_body(50000, 0, &[])] {
        assert_eq!(
            Packet::parse(datagram(address(1), address(3), 17, body)).unwrap_err(),
            PacketError::Header
        );
    }
    let mut body = udp_body(50000, 443, b"one");
    body[5] -= 1;
    assert_eq!(
        Packet::parse(datagram(address(1), address(3), 17, body)).unwrap_err(),
        PacketError::Length
    );
}

fn ipv6_udp() -> Vec<u8> {
    datagram(
        "fd00::1".parse().unwrap(),
        "fd00::3".parse().unwrap(),
        17,
        udp_body(50000, 443, b"odd"),
    )
}
fn extension(mut bytes: Vec<u8>, kind: u8, mut contents: [u8; 8]) -> Vec<u8> {
    contents[0] = bytes[6];
    bytes[6] = kind;
    bytes.splice(40..40, contents);
    let length = (bytes.len() - 40) as u16;
    bytes[4..6].copy_from_slice(&length.to_be_bytes());
    bytes
}

#[test]
fn ipv6_bounded_padding_extensions_and_mandatory_checksum() {
    let valid = ipv6_udp();
    assert!(Packet::parse(valid.clone()).is_ok());
    for end in 0..valid.len() {
        assert!(Packet::parse(valid[..end].to_vec()).is_err());
    }
    let mut missing = valid.clone();
    missing[46..48].fill(0);
    assert_eq!(Packet::parse(missing).unwrap_err(), PacketError::Checksum);
    let mut corrupted = valid.clone();
    *corrupted.last_mut().unwrap() ^= 1;
    assert_eq!(Packet::parse(corrupted).unwrap_err(), PacketError::Checksum);
    let destination = extension(valid.clone(), 60, [0, 0, 1, 4, 0, 0, 0, 0]);
    assert!(Packet::parse(destination.clone()).is_ok());
    let both = extension(destination, 0, [0; 8]);
    assert!(Packet::parse(both.clone()).is_ok());
    assert_eq!(
        Packet::parse(extension(both, 0, [0; 8])).unwrap_err(),
        PacketError::Unsupported
    );
    for kind in [43, 51, 50, 59] {
        assert_eq!(
            Packet::parse(extension(valid.clone(), kind, [0; 8])).unwrap_err(),
            PacketError::Unsupported
        );
    }
    // Includes an atomic fragment: zero offset and M=0 still requires the
    // bounded fragment implementation and is deliberately rejected here.
    for more in [0, 1] {
        assert_eq!(
            Packet::parse(extension(valid.clone(), 44, [0, 0, 0, more, 0, 0, 0, 0])).unwrap_err(),
            PacketError::Fragment
        );
    }
    assert_eq!(
        Packet::parse(extension(valid.clone(), 60, [0, 0, 0xc9, 4, 0, 0, 0, 0])).unwrap_err(),
        PacketError::Unsupported
    );
    assert_eq!(
        Packet::parse(extension(valid.clone(), 0, [0, 0, 0xc2, 4, 0, 1, 0, 0])).unwrap_err(),
        PacketError::Unsupported
    );
    let mut malformed = extension(valid.clone(), 0, [0; 8]);
    malformed[41] = 255;
    assert_eq!(Packet::parse(malformed).unwrap_err(), PacketError::Length);
    let mut jumbo = valid;
    jumbo[4..6].fill(0);
    assert_eq!(Packet::parse(jumbo).unwrap_err(), PacketError::Unsupported);
}

#[test]
fn tcp_bounds_options_and_checksums() {
    let segment = TcpSegment {
        sequence: 100,
        acknowledgement: 0,
        flags: 2,
        payload_len: 0,
    };
    let body = tcp_body(50000, 443, segment);
    for length in [0x40, 0xf0] {
        let mut bad = body.clone();
        bad[12] = length;
        assert_eq!(
            Packet::parse(datagram(address(1), address(3), 6, bad)).unwrap_err(),
            PacketError::Length
        );
    }
    for flags in [3, 6, 5] {
        let mut bad = body.clone();
        bad[13] = flags;
        assert_eq!(
            Packet::parse(datagram(address(1), address(3), 6, bad)).unwrap_err(),
            PacketError::Header
        );
    }
    let mut options = body.clone();
    options[12] = 0x60;
    options.extend([2, 4, 5, 180]);
    assert!(Packet::parse(datagram(address(1), address(3), 6, options.clone())).is_ok());
    options[21] = 5;
    assert_eq!(
        Packet::parse(datagram(address(1), address(3), 6, options)).unwrap_err(),
        PacketError::Length
    );
    let mut malformed = body;
    malformed[12] = 0x51;
    assert_eq!(
        Packet::parse(datagram(address(1), address(3), 6, malformed)).unwrap_err(),
        PacketError::Header
    );
    let mut corrupt = tcp(false, 100, 0, 2, 0).into_bytes();
    corrupt[36] ^= 1;
    assert_eq!(Packet::parse(corrupt).unwrap_err(), PacketError::Checksum);
}

#[test]
fn only_echo_icmp_and_family_correct_checksums() {
    assert!(matches!(
        echo(false, 9, 8).transport(),
        Transport::Echo {
            reply: false,
            identifier: 9,
            sequence: 8
        }
    ));
    assert!(matches!(
        echo(true, 9, 8).transport(),
        Transport::Echo { reply: true, .. }
    ));
    let src = "fd00::1".parse().unwrap();
    let dst = "fd00::3".parse().unwrap();
    for kind in [128, 129] {
        let bytes = datagram(src, dst, 58, vec![kind, 0, 0, 0, 0, 9, 0, 8]);
        assert!(Packet::parse(bytes.clone()).is_ok());
        let mut bad = bytes;
        bad[42] ^= 1;
        assert_eq!(Packet::parse(bad).unwrap_err(), PacketError::Checksum);
    }
    assert_eq!(
        Packet::parse(datagram(
            address(1),
            address(3),
            1,
            vec![3, 0, 0, 0, 0, 0, 0, 0]
        ))
        .unwrap_err(),
        PacketError::Unsupported
    );
    assert_eq!(
        Packet::parse(datagram(src, dst, 1, vec![8, 0, 0, 0, 0, 0, 0, 0])).unwrap_err(),
        PacketError::Unsupported
    );
    assert_eq!(
        Packet::parse(datagram(
            address(1),
            address(3),
            58,
            vec![128, 0, 0, 0, 0, 0, 0, 0]
        ))
        .unwrap_err(),
        PacketError::Unsupported
    );
}
