//! Test-only packet construction. Fixtures have materialized Internet checksums
//! and are always fed through the production Packet parser before admission.
use crate::{Packet, TcpSegment};
use std::net::{IpAddr, Ipv4Addr};

pub(crate) fn checksum(bytes: &[u8]) -> u16 {
    let sum = bytes.chunks(2).fold(0_u64, |sum, word| {
        sum + (u64::from(word[0]) << 8) + u64::from(*word.get(1).unwrap_or(&0))
    });
    let folded = (sum & 65535) + (sum >> 16);
    !(((folded & 65535) + (folded >> 16)) as u16)
}

pub(crate) fn address(node: u8) -> IpAddr {
    Ipv4Addr::new(100, 64, 0, node).into()
}

pub(crate) fn datagram(
    source: IpAddr,
    destination: IpAddr,
    protocol: u8,
    mut body: Vec<u8>,
) -> Vec<u8> {
    let mut pseudo = Vec::new();
    let mut header = match (source, destination) {
        (IpAddr::V4(source), IpAddr::V4(destination)) => {
            pseudo.extend(source.octets());
            pseudo.extend(destination.octets());
            pseudo.extend([0, protocol]);
            pseudo.extend((body.len() as u16).to_be_bytes());
            let mut header = vec![0; 20];
            header[0] = 0x45;
            header[2..4].copy_from_slice(&((body.len() + 20) as u16).to_be_bytes());
            header[8] = 64;
            header[9] = protocol;
            header[12..16].copy_from_slice(&source.octets());
            header[16..20].copy_from_slice(&destination.octets());
            let check = checksum(&header);
            header[10..12].copy_from_slice(&check.to_be_bytes());
            header
        }
        (IpAddr::V6(source), IpAddr::V6(destination)) => {
            pseudo.extend(source.octets());
            pseudo.extend(destination.octets());
            pseudo.extend((body.len() as u32).to_be_bytes());
            pseudo.extend([0, 0, 0, protocol]);
            let mut header = vec![0; 40];
            header[0] = 0x60;
            header[4..6].copy_from_slice(&(body.len() as u16).to_be_bytes());
            header[6] = protocol;
            header[7] = 64;
            header[8..24].copy_from_slice(&source.octets());
            header[24..40].copy_from_slice(&destination.octets());
            header
        }
        _ => panic!("fixture address families must agree"),
    };
    if protocol == 1 {
        pseudo.clear();
    }
    let offset = match protocol {
        6 => 16,
        17 => 6,
        1 | 58 => 2,
        _ => panic!("fixture protocol"),
    };
    body[offset..offset + 2].fill(0);
    pseudo.extend(&body);
    let check = checksum(&pseudo);
    let check = if protocol == 17 && check == 0 {
        u16::MAX
    } else {
        check
    };
    body[offset..offset + 2].copy_from_slice(&check.to_be_bytes());
    header.extend(body);
    header
}

pub(crate) fn udp_body(source: u16, destination: u16, payload: &[u8]) -> Vec<u8> {
    let mut body = vec![0; 8];
    body[..2].copy_from_slice(&source.to_be_bytes());
    body[2..4].copy_from_slice(&destination.to_be_bytes());
    body[4..6].copy_from_slice(&((payload.len() + 8) as u16).to_be_bytes());
    body.extend(payload);
    body
}

pub(crate) fn udp(source: u8, destination: u8, source_port: u16, destination_port: u16) -> Packet {
    Packet::parse(datagram(
        address(source),
        address(destination),
        17,
        udp_body(source_port, destination_port, b"private test payload"),
    ))
    .unwrap()
}

pub(crate) fn tcp_body(source_port: u16, destination_port: u16, segment: TcpSegment) -> Vec<u8> {
    let mut body = vec![0; 20 + usize::from(segment.payload_len)];
    body[..2].copy_from_slice(&source_port.to_be_bytes());
    body[2..4].copy_from_slice(&destination_port.to_be_bytes());
    body[4..8].copy_from_slice(&segment.sequence.to_be_bytes());
    body[8..12].copy_from_slice(&segment.acknowledgement.to_be_bytes());
    body[12] = 0x50;
    body[13] = segment.flags;
    body[14..16].copy_from_slice(&4096_u16.to_be_bytes());
    body
}

pub(crate) fn tcp(
    reverse: bool,
    sequence: u32,
    acknowledgement: u32,
    flags: u8,
    payload_len: u16,
) -> Packet {
    let (source, destination, source_port, destination_port) = if reverse {
        (3, 1, 443, 50000)
    } else {
        (1, 3, 50000, 443)
    };
    Packet::parse(datagram(
        address(source),
        address(destination),
        6,
        tcp_body(
            source_port,
            destination_port,
            TcpSegment {
                sequence,
                acknowledgement,
                flags,
                payload_len,
            },
        ),
    ))
    .unwrap()
}

pub(crate) fn echo(reverse: bool, identifier: u16, sequence: u16) -> Packet {
    let (source, destination, kind) = if reverse { (3, 1, 0) } else { (1, 3, 8) };
    let mut body = vec![kind, 0, 0, 0];
    body.extend(identifier.to_be_bytes());
    body.extend(sequence.to_be_bytes());
    body.extend(b"echo contents");
    Packet::parse(datagram(address(source), address(destination), 1, body)).unwrap()
}
