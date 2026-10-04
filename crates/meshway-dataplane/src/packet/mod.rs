//! Strict, allocation-free inspection of an owned IP datagram.
//!
//! Accepts IPv4 without options and non-jumbo IPv6 with at most one leading
//! hop-by-hop and one destination-options header, containing padding only.
//! Routing, fragmentation (including atomic fragments), AH/ESP and other
//! extensions require separate implementations and are rejected. Transport
//! checksums must already be materialized by the ingress adapter: checksum
//! offload placeholders are not accepted (IPv4 UDP's optional zero is valid).
//! TCP/UDP port zero is outside the business Port contract. TCP options are
//! limited to bounded MSS, window scale, SACK-permitted/SACK and timestamps,
//! plus padding. ICMP accepts echo request/reply with code zero only.
use meshway_configuration::execution::BusinessProtocol;
use meshway_policy::admission::Traffic;
use meshway_transport_spec::Port;
use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PacketError {
    #[error("truncated or inconsistent IP/transport length")]
    Length,
    #[error("unsupported IP version, protocol, option or extension")]
    Unsupported,
    #[error("fragment reassembly is not implemented")]
    Fragment,
    #[error("invalid IP or transport header")]
    Header,
    #[error("invalid IP or transport checksum")]
    Checksum,
}

/// A validated packet. Debug never includes the payload; ownership conversion
/// preserves the original allocation and bytes. This is plaintext, not an
/// authenticated or encrypted wire envelope.
pub struct Packet {
    bytes: Vec<u8>,
    traffic: Traffic,
    transport: Transport,
}

/// Payload-free packet metadata. Shape validation does not authenticate these
/// facts or prove that they describe an encrypted inner packet. Endpoints must
/// compare them with the facts produced by the one plaintext packet parser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketFacts {
    pub traffic: Traffic,
    pub transport: Transport,
    pub len: usize,
}

impl PacketFacts {
    pub fn validate(&self) -> Result<(), PacketError> {
        if self.traffic.source.is_ipv4() != self.traffic.destination.is_ipv4() {
            return Err(PacketError::Header);
        }
        let ip_header = if self.traffic.source.is_ipv4() {
            20
        } else {
            40
        };
        let max_len = if self.traffic.source.is_ipv4() {
            usize::from(u16::MAX)
        } else {
            Packet::MAX_LEN
        };
        let transport_header = match (self.traffic.protocol, self.transport) {
            (BusinessProtocol::Tcp, Transport::Tcp(_)) => 20,
            (BusinessProtocol::Udp, Transport::Udp)
            | (BusinessProtocol::IcmpEcho, Transport::Echo { .. }) => 8,
            _ => return Err(PacketError::Header),
        };
        if self.len > max_len || self.len < ip_header + transport_header {
            return Err(PacketError::Length);
        }
        let ports = [self.traffic.source_port, self.traffic.destination_port];
        if match self.transport {
            Transport::Echo { .. } => ports.iter().any(Option::is_some),
            _ => ports
                .iter()
                .any(|port| port.is_none_or(|port| port.get() == 0)),
        } {
            return Err(PacketError::Header);
        }
        if let Transport::Tcp(segment) = self.transport {
            if usize::from(segment.payload_len) > self.len - ip_header - transport_header {
                return Err(PacketError::Length);
            }
            if segment.flags & 0x03 == 0x03
                || segment.flags & 0x06 == 0x06
                || segment.flags & 0x05 == 0x05
            {
                return Err(PacketError::Header);
            }
        }
        Ok(())
    }

    pub(crate) fn check_packet(&self, packet: &Packet) -> Result<(), crate::Error> {
        if *self != packet.facts() {
            return Err(crate::Error::FactsMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Tcp(TcpSegment),
    Udp,
    Echo {
        reply: bool,
        identifier: u16,
        sequence: u16,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TcpSegment {
    pub sequence: u32,
    pub acknowledgement: u32,
    pub flags: u8,
    pub payload_len: u16,
}

impl TcpSegment {
    pub(crate) fn syn(self) -> bool {
        self.flags & 0x02 != 0
    }
    pub(crate) fn ack(self) -> bool {
        self.flags & 0x10 != 0
    }
    pub(crate) fn fin(self) -> bool {
        self.flags & 0x01 != 0
    }
    pub(crate) fn rst(self) -> bool {
        self.flags & 0x04 != 0
    }
    pub(crate) fn end(self) -> u32 {
        self.sequence
            .wrapping_add(u32::from(self.payload_len))
            .wrapping_add(u32::from(self.syn()))
            .wrapping_add(u32::from(self.fin()))
    }
}

impl fmt::Debug for Packet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Packet")
            .field("length", &self.bytes.len())
            .field("traffic", &self.traffic)
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}

impl Packet {
    /// IPv6's fixed header plus its largest non-jumbo payload.
    pub const MAX_LEN: usize = 40 + u16::MAX as usize;

    pub fn parse(bytes: Vec<u8>) -> Result<Self, PacketError> {
        let (source, destination, protocol, offset) = match bytes.first().map(|b| b >> 4) {
            Some(4) => ipv4(&bytes)?,
            Some(6) => ipv6(&bytes)?,
            Some(_) => return Err(PacketError::Unsupported),
            None => return Err(PacketError::Length),
        };
        let body = &bytes[offset..];
        let (business, source_port, destination_port, transport) = match protocol {
            6 => {
                if body.len() < 20 {
                    return Err(PacketError::Length);
                }
                let header = usize::from(body[12] >> 4) * 4;
                if header < 20 || header > body.len() {
                    return Err(PacketError::Length);
                }
                let flags = body[13];
                if body[12] & 0x0f != 0
                    || flags & 0x03 == 0x03
                    || flags & 0x06 == 0x06
                    || flags & 0x05 == 0x05
                {
                    return Err(PacketError::Header);
                }
                tcp_options(&body[20..header], flags)?;
                check_transport(source, destination, protocol, body)?;
                let (src, dst) = ports(body)?;
                (
                    BusinessProtocol::Tcp,
                    Some(src),
                    Some(dst),
                    Transport::Tcp(TcpSegment {
                        sequence: word32(body, 4),
                        acknowledgement: word32(body, 8),
                        flags,
                        payload_len: u16::try_from(body.len() - header)
                            .map_err(|_| PacketError::Length)?,
                    }),
                )
            }
            17 => {
                if body.len() < 8 || usize::from(word(body, 4)) != body.len() {
                    return Err(PacketError::Length);
                }
                if word(body, 6) == 0 {
                    if source.is_ipv6() {
                        return Err(PacketError::Checksum);
                    }
                } else {
                    check_transport(source, destination, protocol, body)?;
                }
                let (src, dst) = ports(body)?;
                (BusinessProtocol::Udp, Some(src), Some(dst), Transport::Udp)
            }
            1 | 58 => {
                if (protocol == 1) != source.is_ipv4() {
                    return Err(PacketError::Unsupported);
                }
                if body.len() < 8 {
                    return Err(PacketError::Length);
                }
                let reply = match (protocol, body[0], body[1]) {
                    (1, 8, 0) | (58, 128, 0) => false,
                    (1, 0, 0) | (58, 129, 0) => true,
                    _ => return Err(PacketError::Unsupported),
                };
                if protocol == 1 {
                    if finish(sum(0, body)) != 0 {
                        return Err(PacketError::Checksum);
                    }
                } else {
                    check_transport(source, destination, protocol, body)?;
                }
                (
                    BusinessProtocol::IcmpEcho,
                    None,
                    None,
                    Transport::Echo {
                        reply,
                        identifier: word(body, 4),
                        sequence: word(body, 6),
                    },
                )
            }
            _ => return Err(PacketError::Unsupported),
        };
        Ok(Self {
            bytes,
            traffic: Traffic {
                source,
                destination,
                protocol: business,
                source_port,
                destination_port,
            },
            transport,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
    pub fn traffic(&self) -> Traffic {
        self.traffic
    }
    pub fn transport(&self) -> Transport {
        self.transport
    }
    pub fn facts(&self) -> PacketFacts {
        PacketFacts {
            traffic: self.traffic,
            transport: self.transport,
            len: self.bytes.len(),
        }
    }
}

fn ipv4(bytes: &[u8]) -> Result<(IpAddr, IpAddr, u8, usize), PacketError> {
    if bytes.len() < 20 {
        return Err(PacketError::Length);
    }
    let header = usize::from(bytes[0] & 0x0f) * 4;
    if header < 20 || header > bytes.len() || usize::from(word(bytes, 2)) != bytes.len() {
        return Err(PacketError::Length);
    }
    if word(bytes, 6) & 0x3fff != 0 {
        return Err(PacketError::Fragment);
    }
    if header != 20 {
        return Err(PacketError::Unsupported);
    }
    if word(bytes, 6) & 0x8000 != 0 || bytes[8] == 0 {
        return Err(PacketError::Header);
    }
    if finish(sum(0, &bytes[..header])) != 0 {
        return Err(PacketError::Checksum);
    }
    Ok((
        Ipv4Addr::new(bytes[12], bytes[13], bytes[14], bytes[15]).into(),
        Ipv4Addr::new(bytes[16], bytes[17], bytes[18], bytes[19]).into(),
        bytes[9],
        header,
    ))
}

fn ipv6(bytes: &[u8]) -> Result<(IpAddr, IpAddr, u8, usize), PacketError> {
    if bytes.len() < 40 {
        return Err(PacketError::Length);
    }
    let length = usize::from(word(bytes, 4));
    if length == 0 {
        return Err(PacketError::Unsupported);
    }
    if length + 40 != bytes.len() {
        return Err(PacketError::Length);
    }
    if bytes[7] == 0 {
        return Err(PacketError::Header);
    }
    let mut source = [0; 16];
    let mut destination = [0; 16];
    source.copy_from_slice(&bytes[8..24]);
    destination.copy_from_slice(&bytes[24..40]);
    let mut next = bytes[6];
    let mut offset = 40;
    let mut count = 0;
    let mut destination_options = false;
    while matches!(next, 0 | 60) {
        if count == 2 || (next == 0 && offset != 40) || destination_options {
            return Err(PacketError::Unsupported);
        }
        destination_options = next == 60;
        if bytes.len() - offset < 2 {
            return Err(PacketError::Length);
        }
        let length = (usize::from(bytes[offset + 1]) + 1) * 8;
        let end = offset + length;
        if end > bytes.len() {
            return Err(PacketError::Length);
        }
        padding(&bytes[offset + 2..end])?;
        next = bytes[offset];
        offset = end;
        count += 1;
    }
    if next == 44 {
        return Err(PacketError::Fragment);
    }
    Ok((
        Ipv6Addr::from(source).into(),
        Ipv6Addr::from(destination).into(),
        next,
        offset,
    ))
}

fn padding(mut bytes: &[u8]) -> Result<(), PacketError> {
    while let Some(&kind) = bytes.first() {
        if kind == 0 {
            bytes = &bytes[1..];
            continue;
        }
        if kind != 1 {
            return Err(PacketError::Unsupported);
        }
        if bytes.len() < 2 {
            return Err(PacketError::Length);
        }
        let end = usize::from(bytes[1]) + 2;
        if end > bytes.len() {
            return Err(PacketError::Length);
        }
        if bytes[2..end].iter().any(|b| *b != 0) {
            return Err(PacketError::Header);
        }
        bytes = &bytes[end..];
    }
    Ok(())
}

fn tcp_options(mut bytes: &[u8], flags: u8) -> Result<(), PacketError> {
    while let Some(&kind) = bytes.first() {
        if kind == 0 {
            return if bytes.iter().all(|b| *b == 0) {
                Ok(())
            } else {
                Err(PacketError::Header)
            };
        }
        if kind == 1 {
            bytes = &bytes[1..];
            continue;
        }
        if bytes.len() < 2 {
            return Err(PacketError::Length);
        }
        let length = usize::from(bytes[1]);
        if length < 2 || length > bytes.len() {
            return Err(PacketError::Length);
        }
        match (kind, length) {
            (2, 4) | (4, 2) if flags & 2 != 0 => {}
            (3, 3) if flags & 2 != 0 && bytes[2] <= 14 => {}
            (5, n) if flags & 0x10 != 0 && n >= 10 && (n - 2).is_multiple_of(8) => {}
            (8, 10) => {}
            _ => return Err(PacketError::Unsupported),
        }
        bytes = &bytes[length..];
    }
    Ok(())
}

fn ports(body: &[u8]) -> Result<(Port, Port), PacketError> {
    Ok((
        Port::new(word(body, 0)).map_err(|_| PacketError::Header)?,
        Port::new(word(body, 2)).map_err(|_| PacketError::Header)?,
    ))
}

fn word(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}
fn word32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

// RFC 1071 one's-complement Internet checksum, not a cryptographic primitive.
fn sum(mut value: u32, bytes: &[u8]) -> u32 {
    let (chunks, remaining) = bytes.as_chunks::<2>();
    for pair in chunks {
        value += u32::from(u16::from_be_bytes([pair[0], pair[1]]));
    }
    if let Some(&last) = remaining.first() {
        value += u32::from(last) << 8;
    }
    value
}
fn finish(mut value: u32) -> u16 {
    while value >> 16 != 0 {
        value = (value & 0xffff) + (value >> 16);
    }
    !(value as u16)
}
fn pseudo(source: IpAddr, destination: IpAddr, protocol: u8, length: usize) -> u32 {
    let value = match (source, destination) {
        (IpAddr::V4(src), IpAddr::V4(dst)) => sum(sum(0, &src.octets()), &dst.octets()),
        (IpAddr::V6(src), IpAddr::V6(dst)) => sum(sum(0, &src.octets()), &dst.octets()),
        _ => unreachable!("parser always pairs addresses from one IP header"),
    };
    value + u32::from(protocol) + length as u32
}
fn check_transport(
    source: IpAddr,
    destination: IpAddr,
    protocol: u8,
    body: &[u8],
) -> Result<(), PacketError> {
    if finish(sum(pseudo(source, destination, protocol, body.len()), body)) != 0 {
        return Err(PacketError::Checksum);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
