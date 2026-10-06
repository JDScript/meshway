//! Linux virtio-net header codec. All offsets are checked before packet access.
//! This is packet I/O normalization, not policy or business packet parsing.
use std::io;

pub(crate) const HEADER: usize = 10;
pub(crate) const MAX_SUPER: usize = 65535;

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid TUN offload packet")
}

fn word(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}

fn put(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_be_bytes());
}

fn sum(bytes: &[u8]) -> u32 {
    bytes
        .chunks(2)
        .map(|b| u32::from(u16::from_be_bytes([b[0], b.get(1).copied().unwrap_or(0)])))
        .sum()
}

fn fold(mut value: u32) -> u16 {
    while value > u32::from(u16::MAX) {
        value = (value & 0xffff) + (value >> 16);
    }
    !(value as u16)
}

#[derive(Clone, Copy)]
struct Layout {
    ip: usize,
    header: usize,
    checksum: usize,
    protocol: u8,
    v6: bool,
}

impl Layout {
    fn valid(self, packet: &[u8]) -> bool {
        let length_ok = if self.v6 {
            usize::from(word(packet, 4)) + 40 == packet.len()
        } else {
            usize::from(word(packet, 2)) == packet.len() && fold(sum(&packet[..self.ip])) == 0
        };
        length_ok
            && fold(self.pseudo(packet) + sum(&packet[self.ip..])) == 0
            && (self.protocol != 17
                || usize::from(word(packet, self.ip + 4)) == packet.len() - self.ip)
    }
    fn parse(packet: &[u8]) -> io::Result<Self> {
        let (ip, protocol, v6) = match packet.first().map(|b| b >> 4) {
            Some(4) if packet.len() >= 20 => {
                let ip = usize::from(packet[0] & 15) * 4;
                if ip < 20 || ip > packet.len() || word(packet, 6) & 0x3fff != 0 {
                    return Err(invalid());
                }
                (ip, packet[9], false)
            }
            Some(6) if packet.len() >= 40 => (40, packet[6], true),
            _ => return Err(invalid()),
        };
        let (transport, checksum) = match protocol {
            6 if packet.len() >= ip + 20 => {
                let size = usize::from(packet[ip + 12] >> 4) * 4;
                if size < 20 || ip + size > packet.len() {
                    return Err(invalid());
                }
                (size, ip + 16)
            }
            17 if packet.len() >= ip + 8 => (8, ip + 6),
            _ => return Err(invalid()),
        };
        Ok(Self {
            ip,
            header: ip + transport,
            checksum,
            protocol,
            v6,
        })
    }

    fn pseudo(self, packet: &[u8]) -> u32 {
        let length = (packet.len() - self.ip) as u32;
        let addresses = if self.v6 {
            &packet[8..40]
        } else {
            &packet[12..20]
        };
        sum(addresses) + length + u32::from(self.protocol)
    }

    fn finish(self, packet: &mut [u8]) {
        let length = packet.len();
        if self.v6 {
            put(packet, 4, (length - 40) as u16);
        } else {
            put(packet, 2, length as u16);
            put(packet, 10, 0);
            put(packet, 10, fold(sum(&packet[..self.ip])));
        }
        if self.protocol == 17 {
            put(packet, self.ip + 4, (length - self.ip) as u16);
        }
        put(packet, self.checksum, 0);
        let checksum = fold(self.pseudo(packet) + sum(&packet[self.ip..]));
        put(
            packet,
            self.checksum,
            if checksum == 0 && self.protocol == 17 {
                0xffff
            } else {
                checksum
            },
        );
    }
}

/// Split a kernel aggregate into complete, checksummed IP datagrams.
pub(crate) fn split(input: &[u8], mtu: usize) -> io::Result<Vec<Vec<u8>>> {
    if input.len() < HEADER || input.len() > HEADER + MAX_SUPER {
        return Err(invalid());
    }
    let flags = input[0];
    let kind = input[1];
    if flags & !3 != 0 {
        return Err(invalid());
    }
    // Without TUNSETVNETLE/TUNSETVNETBE, Linux uses native-endian virtio fields.
    let native = |at| u16::from_ne_bytes([input[at], input[at + 1]]);
    let packet = &input[HEADER..];
    if kind == 0 {
        if packet.is_empty() || packet.len() > mtu {
            return Err(invalid());
        }
        let mut packet = packet.to_vec();
        if flags & 1 != 0 {
            let start = usize::from(native(6));
            let checksum = start
                .checked_add(usize::from(native(8)))
                .ok_or_else(invalid)?;
            if checksum + 2 > packet.len() {
                return Err(invalid());
            }
            // CHECKSUM_PARTIAL includes the pseudo-header seed in the field.
            let value = fold(sum(&packet[start..]));
            put(
                &mut packet,
                checksum,
                if value == 0 { 0xffff } else { value },
            );
        }
        return Ok(vec![packet]);
    }
    let layout = Layout::parse(packet)?;
    let expected = if layout.protocol == 17 {
        5
    } else if layout.v6 {
        4
    } else {
        1
    };
    let size = usize::from(native(4));
    if kind != expected
        || flags & 1 == 0
        || size == 0
        || layout.header + size > mtu
        || usize::from(native(6)) != layout.ip
        || usize::from(native(8)) != layout.checksum - layout.ip
        || packet.len() <= layout.header
    {
        return Err(invalid());
    }
    let payload = &packet[layout.header..];
    let count = payload.len().div_ceil(size);
    // A small gso_size must not manufacture an unbounded number of packets.
    if count > 1024 {
        return Err(invalid());
    }
    let mut packets = Vec::with_capacity(count);
    let sequence = if layout.protocol == 6 {
        u32::from_be_bytes(
            packet[layout.ip + 4..layout.ip + 8]
                .try_into()
                .map_err(|_| invalid())?,
        )
    } else {
        0
    };
    for (index, payload) in payload.chunks(size).enumerate() {
        let mut segment = Vec::with_capacity(layout.header + payload.len());
        segment.extend_from_slice(&packet[..layout.header]);
        segment.extend_from_slice(payload);
        if !layout.v6 {
            put(&mut segment, 4, word(packet, 4).wrapping_add(index as u16));
        }
        if layout.protocol == 6 {
            segment[layout.ip + 4..layout.ip + 8]
                .copy_from_slice(&sequence.wrapping_add((index * size) as u32).to_be_bytes());
            if index + 1 != count {
                segment[layout.ip + 13] &= !(0x01 | 0x08);
            }
            if index != 0 {
                segment[layout.ip + 13] &= !0x80;
            }
        }
        layout.finish(&mut segment);
        packets.push(segment);
    }
    Ok(packets)
}

/// Coalesce only an adjacent compatible prefix. SYN/RST/URG and nonstandard
/// IP headers remain individual packets. No reordering or waiting for a batch.
pub(crate) fn merge(packets: &[&[u8]], udp: bool) -> (Vec<u8>, usize) {
    let first = packets[0];
    let mut output = vec![0; HEADER];
    output.extend_from_slice(first);
    let Ok(layout) = Layout::parse(first) else {
        return (output, 1);
    };
    if !layout.valid(first)
        || (!layout.v6 && layout.ip != 20)
        || (layout.protocol == 17 && !udp)
        || first.len() == layout.header
        || (layout.protocol == 6
            && (first[layout.ip + 13] & !0x18 != 0 || first[layout.ip + 12] & 15 != 0))
    {
        return (output, 1);
    }
    let size = first.len() - layout.header;
    let mut count = 1;
    let sequence = if layout.protocol == 6 {
        u32::from_be_bytes(
            first[layout.ip + 4..layout.ip + 8]
                .try_into()
                .expect("validated TCP header"),
        )
    } else {
        0
    };
    for packet in &packets[1..] {
        let Ok(next) = Layout::parse(packet) else {
            break;
        };
        let payload = packet.len() - next.header;
        if !next.valid(packet)
            || next.header != layout.header
            || next.ip != layout.ip
            || next.protocol != layout.protocol
            || next.v6 != layout.v6
            || payload == 0
            || payload > size
            || output.len() - HEADER + payload > MAX_SUPER
        {
            break;
        }
        let same = (0..layout.header).all(|at| {
            let mutable = if layout.v6 {
                (4..6).contains(&at)
            } else {
                (2..6).contains(&at) || (10..12).contains(&at)
            };
            let transport_mutable = (layout.checksum..layout.checksum + 2).contains(&at)
                || (layout.protocol == 17 && (layout.ip + 4..layout.ip + 6).contains(&at))
                || (layout.protocol == 6
                    && ((layout.ip + 4..layout.ip + 8).contains(&at) || at == layout.ip + 13));
            mutable || transport_mutable || first[at] == packet[at]
        });
        if !same || (!layout.v6 && word(packet, 4) != word(first, 4).wrapping_add(count as u16)) {
            break;
        }
        if layout.protocol == 6 {
            let seq = u32::from_be_bytes(
                packet[layout.ip + 4..layout.ip + 8]
                    .try_into()
                    .expect("validated TCP header"),
            );
            if seq != sequence.wrapping_add(((count) * size) as u32)
                || packet[layout.ip + 13] & !0x18 != 0
                || (packet[layout.ip + 13] ^ first[layout.ip + 13]) & !0x08 != 0
                || output[HEADER + layout.ip + 13] & 0x08 != 0
            {
                break;
            }
            output[HEADER + layout.ip + 13] = packet[layout.ip + 13];
        }
        output.extend_from_slice(&packet[layout.header..]);
        count += 1;
        if payload < size {
            break;
        }
    }
    if count > 1 {
        output[0] = 1; // VIRTIO_NET_HDR_F_NEEDS_CSUM
        output[1] = if layout.protocol == 17 {
            5
        } else if layout.v6 {
            4
        } else {
            1
        };
        output[2..4].copy_from_slice(&(layout.header as u16).to_ne_bytes());
        output[4..6].copy_from_slice(&(size as u16).to_ne_bytes());
        output[6..8].copy_from_slice(&(layout.ip as u16).to_ne_bytes());
        output[8..10].copy_from_slice(&((layout.checksum - layout.ip) as u16).to_ne_bytes());
        let packet = &mut output[HEADER..];
        layout.finish(packet);
        let seed = !fold(layout.pseudo(packet));
        put(packet, layout.checksum, seed);
    }
    (output, count)
}

#[cfg(test)]
mod tests;
