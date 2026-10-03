use super::{Protocol, Tuple};
use crate::packet_io::ipv4;
use std::net::Ipv4Addr;

#[derive(Clone, Copy)]
pub(super) struct Packet {
    pub tuple: Tuple,
    pub tcp: Option<Tcp>,
    pub opener: bool,
}

#[derive(Clone, Copy)]
pub(super) struct Tcp {
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub len: u32,
    pub window: u16,
    pub scale: Option<u8>,
}

impl Packet {
    pub fn parse(bytes: &[u8], mtu: u16) -> Option<Self> {
        let (src, dst) = ipv4(bytes, mtu)?;
        let body = &bytes[20..];
        let mut tuple = Tuple {
            src,
            dst,
            protocol: Protocol::Udp,
            a: 0,
            b: 0,
        };
        let (tcp, opener) = match bytes[9] {
            6 => {
                if body.len() < 20 || body[12] & 15 != 0 || body[13] & 0xc0 != 0 {
                    return None;
                }
                let header = usize::from(body[12] >> 4) * 4;
                if header < 20 || header > body.len() || !transport_checksum(src, dst, 6, body) {
                    return None;
                }
                tuple.protocol = Protocol::Tcp;
                tuple.a = word(body, 0);
                tuple.b = word(body, 2);
                let tcp = Tcp {
                    seq: u32::from_be_bytes(body[4..8].try_into().ok()?),
                    ack: u32::from_be_bytes(body[8..12].try_into().ok()?),
                    flags: body[13],
                    len: u32::try_from(body.len() - header).ok()?,
                    window: word(body, 14),
                    scale: window_scale(&body[20..header], body[13] & 2 != 0)?,
                };
                (Some(tcp), tcp.flags == 2 && tcp.len == 0 && tcp.ack == 0)
            }
            17 => {
                if body.len() < 8
                    || usize::from(word(body, 4)) != body.len()
                    || (word(body, 6) != 0 && !transport_checksum(src, dst, 17, body))
                {
                    return None;
                }
                tuple.a = word(body, 0);
                tuple.b = word(body, 2);
                (None, true)
            }
            1 => {
                if body.len() < 8 || body[1] != 0 || !checksum(body, 0) || !matches!(body[0], 0 | 8)
                {
                    return None;
                }
                tuple.protocol = Protocol::Echo;
                tuple.a = word(body, 4);
                tuple.b = word(body, 6);
                (None, body[0] == 8)
            }
            _ => return None,
        };
        if tuple.protocol != Protocol::Echo && (tuple.a == 0 || tuple.b == 0) {
            return None;
        }
        Some(Self { tuple, tcp, opener })
    }
}

fn window_scale(mut options: &[u8], syn: bool) -> Option<Option<u8>> {
    let mut scale = None;
    while let Some(&kind) = options.first() {
        if kind == 0 {
            break;
        }
        if kind == 1 {
            options = &options[1..];
            continue;
        }
        let length = usize::from(*options.get(1)?);
        if length < 2 || length > options.len() {
            return None;
        }
        if kind == 3 {
            if !syn || length != 3 || options[2] > 14 || scale.is_some() {
                return None;
            }
            scale = Some(options[2]);
        }
        options = &options[length..];
    }
    Some(scale)
}

fn word(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

fn transport_checksum(src: Ipv4Addr, dst: Ipv4Addr, protocol: u8, body: &[u8]) -> bool {
    let sum = src
        .octets()
        .as_chunks::<2>()
        .0
        .iter()
        .chain(dst.octets().as_chunks::<2>().0.iter())
        .map(|w| u32::from(u16::from_be_bytes(*w)))
        .sum::<u32>()
        + u32::from(protocol)
        + body.len() as u32;
    checksum(body, sum)
}

fn checksum(bytes: &[u8], mut sum: u32) -> bool {
    let (words, tail) = bytes.as_chunks::<2>();
    sum += words
        .iter()
        .map(|w| u32::from(u16::from_be_bytes(*w)))
        .sum::<u32>();
    if let Some(byte) = tail.first() {
        sum += u32::from(*byte) << 8;
    }
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    sum == 0xffff
}
