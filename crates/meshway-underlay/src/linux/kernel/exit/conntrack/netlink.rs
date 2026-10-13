//! Minimal nf_conntrack netlink (`NFNL_SUBSYS_CTNETLINK`) client: exact
//! default-zone IPv4 GET/DELETE by original tuple and a mark-filtered dump.
//! Every request names its tuple; this codec cannot express a flush. Replies
//! are decoded from one bounded receive buffer and never trusted to be
//! well formed.
use super::{Entry, Tuple};
use meshway_model::exit::ExitUnavailableReason as Reason;
use rtnetlink::sys::{Socket, SocketAddr, protocols::NETLINK_NETFILTER};
use std::{
    net::Ipv4Addr,
    time::{Duration, Instant},
};
use tokio::io::unix::AsyncFd;

/// The same per-operation budget as the kernel tools.
const BUDGET: Duration = super::super::command::BUDGET;
/// One datagram, as for the routing monitor; a full buffer is a truncation.
const BUFFER: usize = 65_536;
/// Bounded entries examined by one dump before it is declared unusable.
const DUMP_LIMIT: usize = 65_536;

const NLMSG_HEADER: usize = 16;
const NLMSG_NOOP: u16 = 1;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const NLM_F_REQUEST: u16 = 0x1;
const NLM_F_ACK: u16 = 0x4;
const NLM_F_DUMP: u16 = 0x300;
#[cfg(test)]
const NLM_F_EXCL: u16 = 0x200;
#[cfg(test)]
const NLM_F_CREATE: u16 = 0x400;
const NLA_F_NESTED: u16 = 0x8000;
const NLA_TYPE_MASK: u16 = 0x3fff;

const NFNL_SUBSYS_CTNETLINK: u16 = 1;
const IPCTNL_MSG_CT_NEW: u16 = 0;
const IPCTNL_MSG_CT_GET: u16 = 1;
const IPCTNL_MSG_CT_DELETE: u16 = 2;
const AF_INET: u8 = 2;

const CTA_TUPLE_ORIG: u16 = 1;
const CTA_TUPLE_REPLY: u16 = 2;
const CTA_STATUS: u16 = 3;
const CTA_HELP: u16 = 5;
#[cfg(test)]
const CTA_TIMEOUT: u16 = 7;
const CTA_MARK: u16 = 8;
const CTA_ID: u16 = 12;
const CTA_ZONE: u16 = 18;
const CTA_MARK_MASK: u16 = 21;
const CTA_TUPLE_IP: u16 = 1;
const CTA_TUPLE_PROTO: u16 = 2;
const CTA_IP_V4_SRC: u16 = 1;
const CTA_IP_V4_DST: u16 = 2;
const CTA_PROTO_NUM: u16 = 1;
const CTA_PROTO_SRC_PORT: u16 = 2;
const CTA_PROTO_DST_PORT: u16 = 3;
const CTA_PROTO_ICMP_ID: u16 = 4;
const CTA_PROTO_ICMP_TYPE: u16 = 5;
const CTA_PROTO_ICMP_CODE: u16 = 6;
/// `IPS_OFFLOAD` and `IPS_HW_OFFLOAD`: a flowtable owns the forwarding.
const IPS_OFFLOAD: u32 = (1 << 14) | (1 << 15);
#[cfg(test)]
const IPS_SEEN_REPLY: u32 = 1 << 1;
/// A ctnetlink-created entry is inserted confirmed; omitting the bit from
/// `CTA_STATUS` asks to clear it and the kernel answers `EBUSY`.
#[cfg(test)]
const IPS_CONFIRMED: u32 = 1 << 3;

const EPERM: i32 = 1;
const ENOENT: i32 = 2;
const EACCES: i32 = 13;

/// A kernel answer that is not an entry: absence, or why ctnetlink is unusable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Failure {
    /// The kernel refused the operation for missing `CAP_NET_ADMIN`.
    Permission,
    /// No ctnetlink, an unsupported filter, a truncated or malformed reply,
    /// or the budget elapsed.
    Unavailable,
    /// The reply names an entry this owner cannot prove it exclusively owns
    /// (offload, helper, a non-default zone).
    Ownership,
}

impl From<Failure> for Reason {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Permission => Self::NetAdminMissing,
            Failure::Unavailable => Self::ConntrackUnavailable,
            Failure::Ownership => Self::OwnershipConflict,
        }
    }
}

/// Outcome of an exact delete. `Absent` also covers an id mismatch: the
/// entry the caller identified no longer exists under its original tuple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Deleted {
    Deleted,
    Absent,
}

fn errno(code: i32) -> Failure {
    match code {
        EPERM | EACCES => Failure::Permission,
        _ => Failure::Unavailable,
    }
}

fn io(error: &std::io::Error) -> Failure {
    error.raw_os_error().map_or(Failure::Unavailable, errno)
}

/// Exact entry by original tuple in the default zone; `None` if absent.
pub(super) async fn get(tuple: &Tuple) -> Result<Option<Entry>, Failure> {
    let request = Request::new(IPCTNL_MSG_CT_GET, NLM_F_REQUEST | NLM_F_ACK)
        .tuple(CTA_TUPLE_ORIG, tuple)
        .u16(CTA_ZONE, 0);
    let mut found = None;
    match exchange(request, |payload| {
        if found.replace(parse_entry(payload)?).is_some() {
            return Err(Failure::Unavailable);
        }
        Ok(true)
    })
    .await?
    {
        Answer::Ack => {}
        Answer::Errno(ENOENT) if found.is_none() => return Ok(None),
        Answer::Errno(code) => return Err(errno(code)),
    }
    found.map(Some).ok_or(Failure::Unavailable)
}

/// Delete exactly the entry with this original tuple, default zone and id.
pub(super) async fn delete(tuple: &Tuple, id: u32) -> Result<Deleted, Failure> {
    let request = Request::new(IPCTNL_MSG_CT_DELETE, NLM_F_REQUEST | NLM_F_ACK)
        .tuple(CTA_TUPLE_ORIG, tuple)
        .u16(CTA_ZONE, 0)
        .u32(CTA_ID, id);
    match exchange(request, |_| Err(Failure::Unavailable)).await? {
        Answer::Ack => Ok(Deleted::Deleted),
        Answer::Errno(ENOENT) => Ok(Deleted::Absent),
        Answer::Errno(code) => Err(errno(code)),
    }
}

/// Whether any IPv4 entry carries all bits of `mask` as in `value`. The
/// kernel filters the dump; each entry is checked again regardless.
pub(super) async fn any_marked(value: u32, mask: u32) -> Result<bool, Failure> {
    let request = Request::new(IPCTNL_MSG_CT_GET, NLM_F_REQUEST | NLM_F_DUMP)
        .u32(CTA_MARK, value)
        .u32(CTA_MARK_MASK, mask);
    let mut seen = 0;
    let mut marked = false;
    let result = exchange(request, |payload| {
        seen += 1;
        if seen > DUMP_LIMIT {
            return Err(Failure::Unavailable);
        }
        // Only the mark is examined: other families' and protocols' entries
        // need not have a tuple shape this owner understands.
        marked = parse_mark(payload)? & mask == value & mask;
        Ok(!marked)
    })
    .await;
    match result {
        // Stopped at the first marked entry; the rest of the dump is dropped.
        _ if marked => Ok(true),
        Ok(Answer::Ack) => Ok(false),
        Ok(Answer::Errno(code)) => Err(errno(code)),
        Err(failure) => Err(failure),
    }
}

/// Create an unreplied-then-seen entry with its inverse reply (test setup).
#[cfg(test)]
pub(super) async fn insert(tuple: &Tuple, mark: u32, timeout: u32) -> Result<(), Failure> {
    let reply = Tuple {
        source: tuple.destination,
        destination: tuple.source,
        protocol: tuple.protocol,
        source_key: if tuple.protocol == 1 {
            tuple.source_key
        } else {
            tuple.destination_key
        },
        destination_key: if tuple.protocol == 1 {
            0
        } else {
            tuple.source_key
        },
    };
    let request = Request::new(
        IPCTNL_MSG_CT_NEW,
        NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL,
    )
    .tuple(CTA_TUPLE_ORIG, tuple)
    .tuple(CTA_TUPLE_REPLY, &reply)
    .u32(CTA_STATUS, IPS_SEEN_REPLY | IPS_CONFIRMED)
    .u32(CTA_TIMEOUT, timeout)
    .u32(CTA_MARK, mark);
    match exchange(request, |_| Err(Failure::Unavailable)).await? {
        Answer::Ack => Ok(()),
        Answer::Errno(code) => Err(errno(code)),
    }
}

enum Answer {
    /// ACK (`NLMSG_ERROR` 0) or the end of a dump.
    Ack,
    /// A negative kernel answer, as a positive errno.
    Errno(i32),
}

async fn exchange(
    request: Request,
    mut entry: impl FnMut(&[u8]) -> Result<bool, Failure>,
) -> Result<Answer, Failure> {
    let end = Instant::now() + BUDGET;
    tokio::time::timeout_at(end.into(), async {
        let mut socket = Socket::new(NETLINK_NETFILTER).map_err(|error| io(&error))?;
        socket.set_non_blocking(true).map_err(|error| io(&error))?;
        socket.bind_auto().map_err(|error| io(&error))?;
        let _ = socket.set_cap_ack(true);
        let socket = AsyncFd::new(socket).map_err(|error| io(&error))?;
        let sequence = request.sequence;
        let bytes = request.finish()?;
        let sent = socket
            .get_ref()
            .send_to(&bytes, &SocketAddr::new(0, 0), 0)
            .map_err(|error| io(&error))?;
        if sent != bytes.len() {
            return Err(Failure::Unavailable);
        }
        let mut buffer = vec![0; BUFFER];
        loop {
            let mut ready = socket.readable().await.map_err(|error| io(&error))?;
            let (size, source) = match socket.get_ref().recv_from(&mut &mut buffer[..], 0) {
                Ok(received) => received,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    ready.clear_ready();
                    continue;
                }
                Err(error) => return Err(io(&error)),
            };
            // Only the kernel answers a unicast netlink request.
            if source.port_number() != 0 {
                continue;
            }
            if size == 0 || size >= buffer.len() {
                return Err(Failure::Unavailable);
            }
            match decode(&buffer[..size], sequence, &mut entry)? {
                Step::More => {}
                Step::Stop => return Ok(Answer::Ack),
                Step::Done(answer) => return Ok(answer),
            }
        }
    })
    .await
    .map_err(|_| Failure::Unavailable)?
}

enum Step {
    More,
    /// The consumer has what it needs; remaining messages are not read.
    Stop,
    Done(Answer),
}

/// Decode one datagram of answers to `sequence`. No input panics.
fn decode(
    mut datagram: &[u8],
    sequence: u32,
    entry: &mut impl FnMut(&[u8]) -> Result<bool, Failure>,
) -> Result<Step, Failure> {
    while !datagram.is_empty() {
        let header = datagram.get(..NLMSG_HEADER).ok_or(Failure::Unavailable)?;
        let length = usize::try_from(u32::from_ne_bytes(word(header, 0)?))
            .map_err(|_| Failure::Unavailable)?;
        if length < NLMSG_HEADER || length > datagram.len() {
            return Err(Failure::Unavailable);
        }
        let kind = u16::from_ne_bytes([header[4], header[5]]);
        if u32::from_ne_bytes(word(header, 8)?) != sequence {
            return Err(Failure::Unavailable);
        }
        let payload = &datagram[NLMSG_HEADER..length];
        match kind {
            NLMSG_NOOP => {}
            NLMSG_ERROR | NLMSG_DONE => {
                let code = match payload.get(..4) {
                    Some(code) => i32::from_ne_bytes(word(code, 0)?),
                    None if kind == NLMSG_DONE => 0,
                    None => return Err(Failure::Unavailable),
                };
                return Ok(Step::Done(match code {
                    0 => Answer::Ack,
                    code if code < 0 => Answer::Errno(code.checked_neg().unwrap_or(i32::MAX)),
                    _ => return Err(Failure::Unavailable),
                }));
            }
            kind if kind == (NFNL_SUBSYS_CTNETLINK << 8 | IPCTNL_MSG_CT_NEW) => {
                if !entry(payload)? {
                    return Ok(Step::Stop);
                }
            }
            _ => return Err(Failure::Unavailable),
        }
        datagram = datagram.get(align(length)..).unwrap_or_default();
    }
    Ok(Step::More)
}

fn word(bytes: &[u8], at: usize) -> Result<[u8; 4], Failure> {
    bytes
        .get(at..at + 4)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(Failure::Unavailable)
}

fn align(length: usize) -> usize {
    length.saturating_add(3) & !3
}

/// Attributes of one level; strict lengths, duplicates rejected by callers.
fn attributes(mut bytes: &[u8]) -> Result<Vec<(u16, &[u8])>, Failure> {
    let mut result = Vec::new();
    while !bytes.is_empty() {
        let header = bytes.get(..4).ok_or(Failure::Unavailable)?;
        let length = usize::from(u16::from_ne_bytes([header[0], header[1]]));
        let kind = u16::from_ne_bytes([header[2], header[3]]) & NLA_TYPE_MASK;
        if length < 4 || length > bytes.len() || result.len() >= 64 {
            return Err(Failure::Unavailable);
        }
        result.push((kind, &bytes[4..length]));
        bytes = bytes.get(align(length)..).unwrap_or_default();
    }
    Ok(result)
}

fn unique<'a>(attributes: &[(u16, &'a [u8])], kind: u16) -> Result<Option<&'a [u8]>, Failure> {
    let mut found = attributes.iter().filter(|(found, _)| *found == kind);
    let value = found.next().map(|(_, value)| *value);
    if found.next().is_some() {
        return Err(Failure::Unavailable);
    }
    Ok(value)
}

fn be32(value: Option<&[u8]>) -> Result<u32, Failure> {
    Ok(u32::from_be_bytes(
        value
            .and_then(|value| value.try_into().ok())
            .ok_or(Failure::Unavailable)?,
    ))
}

fn be16(value: Option<&[u8]>) -> Result<u16, Failure> {
    Ok(u16::from_be_bytes(
        value
            .and_then(|value| value.try_into().ok())
            .ok_or(Failure::Unavailable)?,
    ))
}

fn byte(value: Option<&[u8]>) -> Result<u8, Failure> {
    match value {
        Some([value]) => Ok(*value),
        _ => Err(Failure::Unavailable),
    }
}

/// Top-level attributes after the nfgenmsg (family, version, resource id).
fn message(payload: &[u8]) -> Result<Vec<(u16, &[u8])>, Failure> {
    match payload {
        [AF_INET, 0, _, _, attributes @ ..] => self::attributes(attributes),
        _ => Err(Failure::Unavailable),
    }
}

/// A dumped entry's mark; the kernel omits a zero mark on some versions.
fn parse_mark(payload: &[u8]) -> Result<u32, Failure> {
    match unique(&message(payload)?, CTA_MARK)? {
        None => Ok(0),
        mark => be32(mark),
    }
}

fn parse_entry(payload: &[u8]) -> Result<Entry, Failure> {
    let attributes = message(payload)?;
    for kind in [CTA_STATUS, CTA_HELP, CTA_ZONE, CTA_MARK, CTA_ID] {
        unique(&attributes, kind)?;
    }
    if unique(&attributes, CTA_HELP)?.is_some()
        || unique(&attributes, CTA_ZONE)?.is_some_and(|zone| zone != [0, 0])
        || be32(unique(&attributes, CTA_STATUS)?)? & IPS_OFFLOAD != 0
    {
        return Err(Failure::Ownership);
    }
    let original = parse_tuple(unique(&attributes, CTA_TUPLE_ORIG)?, 8)?;
    let reply = parse_tuple(unique(&attributes, CTA_TUPLE_REPLY)?, 0)?;
    if original.protocol != reply.protocol {
        return Err(Failure::Unavailable);
    }
    Ok(Entry {
        id: be32(unique(&attributes, CTA_ID)?)?,
        mark: be32(unique(&attributes, CTA_MARK)?)?,
        original,
        reply,
    })
}

/// `echo`: the ICMP type the direction must carry (8 request, 0 reply).
fn parse_tuple(value: Option<&[u8]>, echo: u8) -> Result<Tuple, Failure> {
    let tuple = attributes(value.ok_or(Failure::Unavailable)?)?;
    // A directional zone (CTA_TUPLE_ZONE) or any other member is not exact.
    if tuple
        .iter()
        .any(|(kind, _)| !matches!(*kind, CTA_TUPLE_IP | CTA_TUPLE_PROTO))
    {
        return Err(Failure::Ownership);
    }
    let ip = attributes(unique(&tuple, CTA_TUPLE_IP)?.ok_or(Failure::Unavailable)?)?;
    if ip.len() != 2 {
        return Err(Failure::Unavailable);
    }
    let address = |kind| -> Result<Ipv4Addr, Failure> {
        let octets: [u8; 4] = unique(&ip, kind)?
            .and_then(|value| value.try_into().ok())
            .ok_or(Failure::Unavailable)?;
        Ok(Ipv4Addr::from(octets))
    };
    let proto = attributes(unique(&tuple, CTA_TUPLE_PROTO)?.ok_or(Failure::Unavailable)?)?;
    let protocol = byte(unique(&proto, CTA_PROTO_NUM)?)?;
    let (source_key, destination_key) = match protocol {
        6 | 17 if proto.len() == 3 => (
            be16(unique(&proto, CTA_PROTO_SRC_PORT)?)?,
            be16(unique(&proto, CTA_PROTO_DST_PORT)?)?,
        ),
        1 if proto.len() == 4 => {
            let kind = byte(unique(&proto, CTA_PROTO_ICMP_TYPE)?)?;
            if kind != echo || byte(unique(&proto, CTA_PROTO_ICMP_CODE)?)? != 0 {
                return Err(Failure::Unavailable);
            }
            (
                be16(unique(&proto, CTA_PROTO_ICMP_ID)?)?,
                u16::from(kind) << 8,
            )
        }
        _ => return Err(Failure::Unavailable),
    };
    Ok(Tuple {
        source: address(CTA_IP_V4_SRC)?,
        destination: address(CTA_IP_V4_DST)?,
        protocol,
        source_key,
        destination_key,
    })
}

struct Request {
    sequence: u32,
    bytes: Vec<u8>,
}

impl Request {
    fn new(message: u16, flags: u16) -> Self {
        let mut random = [0; 4];
        // A predictable sequence only weakens stale-answer detection on this
        // private, freshly bound socket; it never authorizes an entry.
        let _ = getrandom::fill(&mut random);
        let sequence = u32::from_ne_bytes(random);
        let mut bytes = Vec::with_capacity(128);
        bytes.extend(0u32.to_ne_bytes());
        bytes.extend((NFNL_SUBSYS_CTNETLINK << 8 | message).to_ne_bytes());
        bytes.extend(flags.to_ne_bytes());
        bytes.extend(sequence.to_ne_bytes());
        bytes.extend(0u32.to_ne_bytes());
        bytes.extend([AF_INET, 0, 0, 0]);
        Self { sequence, bytes }
    }

    fn attribute(mut self, kind: u16, value: &[u8]) -> Self {
        put(&mut self.bytes, kind, value);
        self
    }

    fn u32(self, kind: u16, value: u32) -> Self {
        self.attribute(kind, &value.to_be_bytes())
    }

    fn u16(self, kind: u16, value: u16) -> Self {
        self.attribute(kind, &value.to_be_bytes())
    }

    fn tuple(self, kind: u16, tuple: &Tuple) -> Self {
        let mut ip = Vec::new();
        put(&mut ip, CTA_IP_V4_SRC, &tuple.source.octets());
        put(&mut ip, CTA_IP_V4_DST, &tuple.destination.octets());
        let mut proto = Vec::new();
        put(&mut proto, CTA_PROTO_NUM, &[tuple.protocol]);
        if tuple.protocol == 1 {
            put(
                &mut proto,
                CTA_PROTO_ICMP_ID,
                &tuple.source_key.to_be_bytes(),
            );
            put(
                &mut proto,
                CTA_PROTO_ICMP_TYPE,
                &[(tuple.destination_key >> 8) as u8],
            );
            put(&mut proto, CTA_PROTO_ICMP_CODE, &[0]);
        } else {
            put(
                &mut proto,
                CTA_PROTO_SRC_PORT,
                &tuple.source_key.to_be_bytes(),
            );
            put(
                &mut proto,
                CTA_PROTO_DST_PORT,
                &tuple.destination_key.to_be_bytes(),
            );
        }
        let mut nested = Vec::new();
        put(&mut nested, CTA_TUPLE_IP | NLA_F_NESTED, &ip);
        put(&mut nested, CTA_TUPLE_PROTO | NLA_F_NESTED, &proto);
        self.attribute(kind | NLA_F_NESTED, &nested)
    }

    fn finish(mut self) -> Result<Vec<u8>, Failure> {
        let length = u32::try_from(self.bytes.len()).map_err(|_| Failure::Unavailable)?;
        self.bytes[..4].copy_from_slice(&length.to_ne_bytes());
        Ok(self.bytes)
    }
}

fn put(bytes: &mut Vec<u8>, kind: u16, value: &[u8]) {
    // Every value here is a few dozen bytes; u16 lengths cannot overflow.
    let length = (4 + value.len()) as u16;
    bytes.extend(length.to_ne_bytes());
    bytes.extend(kind.to_ne_bytes());
    bytes.extend(value);
    bytes.resize(align(bytes.len()), 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn udp(source_port: u16) -> Tuple {
        Tuple {
            source: Ipv4Addr::new(100, 96, 0, 1),
            destination: Ipv4Addr::new(192, 0, 2, 2),
            protocol: 17,
            source_key: source_port,
            destination_key: 53,
        }
    }

    fn reply_of(tuple: &Tuple) -> Tuple {
        Tuple {
            source: tuple.destination,
            destination: Ipv4Addr::new(198, 51, 100, 1),
            protocol: tuple.protocol,
            source_key: if tuple.protocol == 1 {
                456
            } else {
                tuple.destination_key
            },
            destination_key: if tuple.protocol == 1 { 0 } else { 23456 },
        }
    }

    /// A kernel-shaped answer: one CT message, then the ACK.
    fn answer(sequence: u32, build: impl FnOnce(Request) -> Request) -> Vec<u8> {
        let mut message = build(Request::new(IPCTNL_MSG_CT_NEW, 0)).finish().unwrap();
        message[8..12].copy_from_slice(&sequence.to_ne_bytes());
        let mut ack = Vec::new();
        ack.extend(36u32.to_ne_bytes());
        ack.extend(NLMSG_ERROR.to_ne_bytes());
        ack.extend(0u16.to_ne_bytes());
        ack.extend(sequence.to_ne_bytes());
        ack.extend(0u32.to_ne_bytes());
        ack.extend(0i32.to_ne_bytes());
        ack.extend([0; 16]);
        message.extend(ack);
        message
    }

    fn entry(tuple: &Tuple) -> impl FnOnce(Request) -> Request {
        status(tuple, IPS_SEEN_REPLY)
    }

    fn status(tuple: &Tuple, status: u32) -> impl FnOnce(Request) -> Request {
        let tuple = tuple.clone();
        move |request| {
            request
                .tuple(CTA_TUPLE_ORIG, &tuple)
                .tuple(CTA_TUPLE_REPLY, &reply_of(&tuple))
                .u32(CTA_STATUS, status)
                .u32(CTA_TIMEOUT, 29)
                .u32(CTA_MARK, 8)
                .u32(CTA_ID, 123)
        }
    }

    fn collect(datagram: &[u8], sequence: u32) -> Result<(Vec<Entry>, bool), Failure> {
        let mut entries = Vec::new();
        let step = decode(datagram, sequence, &mut |payload: &[u8]| {
            entries.push(parse_entry(payload)?);
            Ok(true)
        })?;
        Ok((entries, matches!(step, Step::Done(Answer::Ack))))
    }

    #[test]
    fn ctnetlink_requests_always_name_one_exact_tuple_zone_and_id() {
        let tuple = udp(12345);
        let request = Request::new(IPCTNL_MSG_CT_DELETE, NLM_F_REQUEST | NLM_F_ACK)
            .tuple(CTA_TUPLE_ORIG, &tuple)
            .u16(CTA_ZONE, 0)
            .u32(CTA_ID, 77);
        let sequence = request.sequence;
        let bytes = request.finish().unwrap();
        assert_eq!(
            u32::from_ne_bytes(word(&bytes, 0).unwrap()) as usize,
            bytes.len()
        );
        assert_eq!(
            u16::from_ne_bytes([bytes[4], bytes[5]]),
            NFNL_SUBSYS_CTNETLINK << 8 | IPCTNL_MSG_CT_DELETE
        );
        assert_eq!(u32::from_ne_bytes(word(&bytes, 8).unwrap()), sequence);
        let top = message(&bytes[16..]).unwrap();
        assert_eq!(
            top.iter().map(|(kind, _)| *kind).collect::<Vec<_>>(),
            [CTA_TUPLE_ORIG, CTA_ZONE, CTA_ID]
        );
        assert_eq!(be32(unique(&top, CTA_ID).unwrap()).unwrap(), 77);
        assert_eq!(
            parse_tuple(unique(&top, CTA_TUPLE_ORIG).unwrap(), 8).unwrap(),
            tuple
        );
        let echo = Tuple {
            protocol: 1,
            source_key: 321,
            destination_key: 0x0800,
            ..udp(0)
        };
        let bytes = Request::new(IPCTNL_MSG_CT_GET, NLM_F_REQUEST)
            .tuple(CTA_TUPLE_ORIG, &echo)
            .finish()
            .unwrap();
        let top = message(&bytes[16..]).unwrap();
        assert_eq!(
            parse_tuple(unique(&top, CTA_TUPLE_ORIG).unwrap(), 8).unwrap(),
            echo
        );
    }

    #[test]
    fn ctnetlink_entry_identity_is_exact_and_rejects_unprovable_ownership() {
        let tuple = udp(12345);
        let (entries, acked) = collect(&answer(9, entry(&tuple)), 9).unwrap();
        assert!(acked);
        assert_eq!(
            entries,
            [Entry {
                id: 123,
                mark: 8,
                original: tuple.clone(),
                reply: reply_of(&tuple),
            }]
        );
        let echo = Tuple {
            protocol: 1,
            source_key: 321,
            destination_key: 0x0800,
            ..udp(0)
        };
        let (entries, _) = collect(&answer(9, entry(&echo)), 9).unwrap();
        assert_eq!(entries[0].reply.source_key, 456);
        assert_eq!(entries[0].reply.destination_key, 0);
        // A stale or foreign sequence is never an answer to this request.
        assert_eq!(
            collect(&answer(9, entry(&tuple)), 10).unwrap_err(),
            Failure::Unavailable
        );
        type Mutation = fn(Request) -> Request;
        for offload in [1 << 14, 1 << 15] {
            assert_eq!(
                collect(&answer(9, status(&tuple, IPS_SEEN_REPLY | offload)), 9).unwrap_err(),
                Failure::Ownership
            );
        }
        let ownership: [Mutation; 2] = [
            |request| request.u16(CTA_ZONE, 1),
            |request| request.attribute(CTA_HELP | NLA_F_NESTED, &[]),
        ];
        for mutation in ownership {
            let base = entry(&tuple);
            assert_eq!(
                collect(&answer(9, |request| mutation(base(request))), 9).unwrap_err(),
                Failure::Ownership
            );
        }
        let malformed: [Mutation; 2] = [
            |request| request.u32(CTA_MARK, 8),
            |request| request.u32(CTA_ID, 124),
        ];
        for mutation in malformed {
            let base = entry(&tuple);
            assert_eq!(
                collect(&answer(9, |request| mutation(base(request))), 9).unwrap_err(),
                Failure::Unavailable
            );
        }
        // A default zone named explicitly is still the default zone.
        let base = entry(&tuple);
        assert!(collect(&answer(9, |request| base(request).u16(CTA_ZONE, 0)), 9).is_ok());
        // A directional zone inside a tuple is not exact.
        let mut ip = Vec::new();
        put(&mut ip, CTA_IP_V4_SRC, &[1, 2, 3, 4]);
        put(&mut ip, CTA_IP_V4_DST, &[1, 2, 3, 5]);
        let mut tuple_zone = Vec::new();
        put(&mut tuple_zone, CTA_TUPLE_IP | NLA_F_NESTED, &ip);
        put(&mut tuple_zone, 3, &0u16.to_be_bytes());
        assert_eq!(parse_tuple(Some(&tuple_zone), 8), Err(Failure::Ownership));
        // Missing mark or id is not an identity.
        let bare = |request: Request| {
            request
                .tuple(CTA_TUPLE_ORIG, &udp(1))
                .tuple(CTA_TUPLE_REPLY, &reply_of(&udp(1)))
                .u32(CTA_STATUS, 0)
        };
        assert!(collect(&answer(9, bare), 9).is_err());
        // A dump reads only the mark; an absent mark is the kernel's zero.
        let bytes = Request::new(IPCTNL_MSG_CT_NEW, 0).finish().unwrap();
        assert_eq!(parse_mark(&bytes[16..]), Ok(0));
    }

    #[test]
    fn ctnetlink_answers_map_errno_and_end_of_dump() {
        for (code, answer) in [(0, None), (-2, Some(2)), (-1, Some(1)), (-95, Some(95))] {
            let mut bytes = Vec::new();
            bytes.extend(20u32.to_ne_bytes());
            bytes.extend(NLMSG_ERROR.to_ne_bytes());
            bytes.extend(0u16.to_ne_bytes());
            bytes.extend(5u32.to_ne_bytes());
            bytes.extend(0u32.to_ne_bytes());
            bytes.extend(i32::to_ne_bytes(code));
            match decode(&bytes, 5, &mut |_: &[u8]| Ok(true)).unwrap() {
                Step::Done(Answer::Ack) => assert!(answer.is_none()),
                Step::Done(Answer::Errno(found)) => assert_eq!(Some(found), answer),
                _ => panic!("no answer"),
            }
        }
        let mut done = Vec::new();
        done.extend(16u32.to_ne_bytes());
        done.extend(NLMSG_DONE.to_ne_bytes());
        done.extend(0u16.to_ne_bytes());
        done.extend(5u32.to_ne_bytes());
        done.extend(0u32.to_ne_bytes());
        assert!(matches!(
            decode(&done, 5, &mut |_: &[u8]| Ok(true)),
            Ok(Step::Done(Answer::Ack))
        ));
        assert_eq!(errno(EPERM), Failure::Permission);
        assert_eq!(errno(EACCES), Failure::Permission);
        assert_eq!(errno(95), Failure::Unavailable);
        assert_eq!(errno(93), Failure::Unavailable);
        assert_eq!(Reason::from(Failure::Permission), Reason::NetAdminMissing);
        assert_eq!(
            Reason::from(Failure::Unavailable),
            Reason::ConntrackUnavailable
        );
        assert_eq!(Reason::from(Failure::Ownership), Reason::OwnershipConflict);
        assert_eq!(
            io(&std::io::Error::from_raw_os_error(EPERM)),
            Failure::Permission
        );
        assert_eq!(
            io(&std::io::Error::from_raw_os_error(93)),
            Failure::Unavailable
        );
    }

    #[test]
    fn ctnetlink_malformed_answers_never_panic() {
        let valid = answer(9, entry(&udp(12345)));
        // Every truncation and every single-byte corruption fails or decodes,
        // never panics.
        for end in 0..valid.len() {
            let _ = collect(&valid[..end], 9);
        }
        for at in 0..valid.len() {
            for value in [0, 1, 0x7f, 0x80, 0xff] {
                let mut corrupt = valid.clone();
                corrupt[at] = value;
                let _ = collect(&corrupt, 9);
            }
        }
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        for _ in 0..20_000 {
            let mut bytes = valid.clone();
            for _ in 0..4 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let at = (state as usize) % bytes.len();
                bytes[at] = (state >> 32) as u8;
            }
            let _ = collect(&bytes, 9);
            let _ = parse_mark(bytes.get(16..).unwrap_or_default());
        }
        assert!(collect(&[], 9).unwrap().0.is_empty());
        assert_eq!(collect(&[0; 15], 9).unwrap_err(), Failure::Unavailable);
    }

    fn isolated() {
        assert_eq!(
            std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
            Ok("1")
        );
    }

    #[tokio::test]
    #[ignore = "requires isolated Linux namespace with NET_ADMIN and nf_conntrack_netlink"]
    async fn ctnetlink_exact_delete_reports_absent_and_keeps_a_replacement() {
        isolated();
        for tuple in [
            udp(41000),
            Tuple {
                protocol: 6,
                ..udp(41001)
            },
            Tuple {
                protocol: 1,
                source_key: 41002,
                destination_key: 0x0800,
                ..udp(0)
            },
        ] {
            assert_eq!(get(&tuple).await, Ok(None));
            assert_eq!(delete(&tuple, 1).await, Ok(Deleted::Absent));
            insert(&tuple, 8, 60).await.unwrap();
            let entry = get(&tuple).await.unwrap().unwrap();
            assert_eq!(entry.original, tuple);
            assert_eq!(entry.mark, 8);
            assert!(any_marked(8, 8).await.unwrap());
            // Another id under the same tuple is a replacement: untouched.
            assert_eq!(
                delete(&tuple, entry.id.wrapping_add(1)).await,
                Ok(Deleted::Absent)
            );
            assert_eq!(get(&tuple).await, Ok(Some(entry.clone())));
            assert_eq!(delete(&tuple, entry.id).await, Ok(Deleted::Deleted));
            assert_eq!(get(&tuple).await, Ok(None));
            assert_eq!(delete(&tuple, entry.id).await, Ok(Deleted::Absent));
        }
        assert!(!any_marked(8, 8).await.unwrap());
        any_marked(u32::MAX, u32::MAX).await.unwrap();
        println!(
            "M3_CTNETLINK=get,absent-delete,id-mismatch-kept,exact-delete,repeat-absent,mark-dump"
        );
    }

    #[tokio::test]
    #[ignore = "requires isolated Linux namespace without NET_ADMIN"]
    async fn ctnetlink_without_net_admin_is_a_permission_failure() {
        isolated();
        assert_eq!(get(&udp(41010)).await, Err(Failure::Permission));
        assert_eq!(delete(&udp(41010), 1).await, Err(Failure::Permission));
        assert_eq!(
            any_marked(u32::MAX, u32::MAX).await,
            Err(Failure::Permission)
        );
        assert_eq!(
            super::super::available().await.unwrap_err().reason,
            Reason::NetAdminMissing
        );
        println!("M3_CTNETLINK_PERMISSION=netAdminMissing");
    }
}
