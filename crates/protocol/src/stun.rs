//! Small bounded RFC 8489 Binding codec, implemented directly from the RFC.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

const COOKIE: [u8; 4] = [0x21, 0x12, 0xa4, 0x42];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    Ignore,
    Invalid,
    Error,
    Mapped(SocketAddr),
}

pub fn request(transaction: [u8; 12]) -> [u8; 20] {
    let mut packet = [0; 20];
    packet[1] = 1;
    packet[4..8].copy_from_slice(&COOKIE);
    packet[8..20].copy_from_slice(&transaction);
    packet
}

pub fn parse(packet: &[u8], transaction: [u8; 12], ipv4: bool) -> Response {
    if packet.len() < 20 {
        return Response::Invalid;
    }
    let kind = u16::from_be_bytes([packet[0], packet[1]]);
    if packet[0] & 0xc0 != 0
        || !matches!(kind, 0x0101 | 0x0111)
        || packet[4..8] != COOKIE
        || packet[8..20] != transaction
    {
        return Response::Ignore;
    }
    let body_len = usize::from(u16::from_be_bytes([packet[2], packet[3]]));
    if packet.len() > 1024 || body_len % 4 != 0 || body_len + 20 != packet.len() {
        return Response::Invalid;
    }
    let mut offset = 20;
    let mut attributes = 0;
    let mut xor = None;
    let mut fallback = None;
    let mut error = None;
    while offset < packet.len() {
        attributes += 1;
        if attributes > 64 || packet.len() - offset < 4 {
            return Response::Invalid;
        }
        let attr = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
        let len = usize::from(u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]));
        let end = offset + 4 + len.div_ceil(4) * 4;
        if end > packet.len() {
            return Response::Invalid;
        }
        let value = &packet[offset + 4..offset + 4 + len];
        match attr {
            0x0001 | 0x0020 => {
                let slot = if attr == 0x0020 {
                    &mut xor
                } else {
                    &mut fallback
                };
                if slot.is_some() {
                    return Response::Invalid;
                }
                let Some(endpoint) = address(value, attr == 0x0020, transaction, ipv4) else {
                    return Response::Invalid;
                };
                *slot = Some(endpoint);
            }
            0x0009 => {
                // Reserved bits are ignored per RFC; class must be 3..6, number 0..99.
                if error.is_some()
                    || value.len() < 4
                    || !(3..=6).contains(&(value[2] & 7))
                    || value[3] > 99
                    || std::str::from_utf8(&value[4..]).is_err()
                {
                    return Response::Invalid;
                }
                error = Some(u16::from(value[2] & 7) * 100 + u16::from(value[3]));
            }
            0x8028 => {
                // FINGERPRINT must be last, exactly four bytes and CRC-32/IEEE valid.
                if len != 4 || end != packet.len() {
                    return Response::Invalid;
                }
                let received = u32::from_be_bytes([value[0], value[1], value[2], value[3]]);
                if received != crc32(&packet[..offset]) ^ 0x5354_554e {
                    return Response::Invalid;
                }
            }
            // No authentication context exists: integrity attributes are explicitly unsupported.
            0x0008 | 0x001c => return Response::Invalid,
            _ if attr < 0x8000 => return Response::Invalid,
            _ => {}
        }
        offset = end;
    }
    if kind == 0x0111 {
        return if error.is_some() && xor.is_none() && fallback.is_none() {
            Response::Error
        } else {
            Response::Invalid
        };
    }
    if error.is_some() || matches!((xor, fallback), (Some(a), Some(b)) if a != b) {
        return Response::Invalid;
    }
    match xor.or(fallback) {
        Some(endpoint) => Response::Mapped(endpoint),
        None => Response::Invalid,
    }
}

fn address(value: &[u8], xor: bool, transaction: [u8; 12], ipv4: bool) -> Option<SocketAddr> {
    if value.len() < 4 {
        return None;
    }
    // First byte is reserved and ignored by RFC 8489.
    let family = value[1];
    if (family == 1) != ipv4 {
        return None;
    }
    let len = match family {
        1 => 4,
        2 => 16,
        _ => return None,
    };
    if value.len() != len + 4 {
        return None;
    }
    let mut port = u16::from_be_bytes([value[2], value[3]]);
    let mut bytes = [0; 16];
    bytes[..len].copy_from_slice(&value[4..]);
    if xor {
        port ^= 0x2112;
        for (i, byte) in bytes[..len].iter_mut().enumerate() {
            *byte ^= if i < 4 { COOKIE[i] } else { transaction[i - 4] };
        }
    }
    let ip = if len == 4 {
        IpAddr::V4(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]))
    } else {
        let ip = Ipv6Addr::from(bytes);
        if ip.to_ipv4_mapped().is_some() {
            return None;
        }
        IpAddr::V6(ip)
    };
    if port == 0
        || ip.is_unspecified()
        || ip.is_multicast()
        || matches!(ip, IpAddr::V4(v4) if v4.is_broadcast())
    {
        return None;
    }
    Some(SocketAddr::new(ip, port))
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}
