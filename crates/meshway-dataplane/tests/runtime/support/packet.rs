//! Small wire fixture builder, not an alternative parser/admission engine.
use meshway_dataplane::Packet;
use std::net::IpAddr;

fn checksum(bytes: &[u8]) -> u16 {
    let mut sum = bytes.chunks(2).fold(0_u64, |sum, word| {
        sum + (u64::from(word[0]) << 8) + u64::from(*word.get(1).unwrap_or(&0))
    });
    while sum >> 16 != 0 {
        sum = (sum & 65535) + (sum >> 16);
    }
    !(sum as u16)
}

pub fn udp(
    source: IpAddr,
    destination: IpAddr,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
) -> Packet {
    let mut body = vec![0; 8];
    body[..2].copy_from_slice(&source_port.to_be_bytes());
    body[2..4].copy_from_slice(&destination_port.to_be_bytes());
    let length = u16::try_from(payload.len() + 8).unwrap();
    body[4..6].copy_from_slice(&length.to_be_bytes());
    body.extend(payload);
    let mut pseudo = Vec::new();
    let mut header = match (source, destination) {
        (IpAddr::V4(source), IpAddr::V4(destination)) => {
            pseudo.extend(source.octets());
            pseudo.extend(destination.octets());
            pseudo.extend([0, 17]);
            pseudo.extend(length.to_be_bytes());
            let mut header = vec![0; 20];
            header[0] = 0x45;
            header[2..4].copy_from_slice(&(length + 20).to_be_bytes());
            header[8] = 64;
            header[9] = 17;
            header[12..16].copy_from_slice(&source.octets());
            header[16..20].copy_from_slice(&destination.octets());
            let check = checksum(&header);
            header[10..12].copy_from_slice(&check.to_be_bytes());
            header
        }
        (IpAddr::V6(source), IpAddr::V6(destination)) => {
            pseudo.extend(source.octets());
            pseudo.extend(destination.octets());
            pseudo.extend(u32::from(length).to_be_bytes());
            pseudo.extend([0, 0, 0, 17]);
            let mut header = vec![0; 40];
            header[0] = 0x60;
            header[4..6].copy_from_slice(&length.to_be_bytes());
            header[6] = 17;
            header[7] = 64;
            header[8..24].copy_from_slice(&source.octets());
            header[24..40].copy_from_slice(&destination.octets());
            header
        }
        _ => panic!("fixture address families must agree"),
    };
    pseudo.extend(&body);
    let check = checksum(&pseudo);
    body[6..8].copy_from_slice(&if check == 0 { u16::MAX } else { check }.to_be_bytes());
    header.extend(body);
    Packet::parse(header).unwrap()
}
