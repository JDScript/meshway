//! Bounded generation-owned IPv4 flows. A stable unpredictable challenge binds
//! an attempt to its exact tuple/path/context. An authenticated delivery receipt
//! OR a reply carrying that challenge proves the forward packet reached its
//! endpoint. Queue acceptance only advances provisional sending progress.
mod packet;
mod tcp;
use crate::{ReceivedPacket, packet_io::Admission};
use packet::Packet;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    net::Ipv4Addr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::time::{Duration, Instant};

pub const ENVELOPE_BYTES: usize = 69;
const MAGIC: &[u8; 4] = b"MWR2";
const CAPACITY: usize = 1024;
const PEER_CAPACITY: usize = 128;
const FORWARD: u8 = 0;
const REPLY: u8 = 1;
const RECEIPT: u8 = 2;
const CONFIRMED: u8 = 3;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, serde::Serialize)]
pub struct FlowStatistics {
    pub capacity_drops: u64,
    pub invalid_drops: u64,
    pub unproven_drops: u64,
    pub expired_flows: u64,
    pub control_attempts: u64,
    pub control_failures: u64,
}
#[derive(Default)]
struct Counters {
    capacity: AtomicU64,
    invalid: AtomicU64,
    unproven: AtomicU64,
    expired: AtomicU64,
    attempts: AtomicU64,
    failures: AtomicU64,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Protocol {
    Tcp,
    Udp,
    Echo,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Tuple {
    src: Ipv4Addr,
    dst: Ipv4Addr,
    protocol: Protocol,
    a: u16,
    b: u16,
}
impl Tuple {
    fn reverse(self) -> Self {
        Self {
            src: self.dst,
            dst: self.src,
            a: if self.protocol == Protocol::Echo {
                self.a
            } else {
                self.b
            },
            b: if self.protocol == Protocol::Echo {
                self.b
            } else {
                self.a
            },
            ..self
        }
    }
}
#[derive(Clone)]
struct Flow {
    token: [u8; 16],
    challenge: [u8; 16],
    path: Vec<String>,
    peer: Ipv4Addr,
    initiator: bool,
    active: bool,
    sent: bool,
    prepared: bool,
    delivering: bool,
    retire: bool,
    expires: Instant,
    tcp: Option<tcp::State>,
    receipt_due: Option<Instant>,
    receipt_attempts: u8,
}
impl Flow {
    fn advance(&mut self, packet: Packet, reply: bool, sending: bool) -> bool {
        match packet.tcp {
            Some(tcp) => self
                .tcp
                .as_mut()
                .is_some_and(|state| state.advance(tcp, reply, sending)),
            None => packet.tuple.protocol != Protocol::Echo || packet.opener != reply,
        }
    }
    fn idle(&self, protocol: Protocol) -> Duration {
        Duration::from_secs(match protocol {
            Protocol::Tcp if self.tcp.is_some_and(|tcp| tcp.terminal()) => 5,
            Protocol::Tcp => 120,
            Protocol::Udp | Protocol::Echo => 30,
        })
    }
}

#[derive(Default)]
struct Table {
    flows: BTreeMap<Tuple, Flow>,
    tokens: BTreeMap<[u8; 16], Tuple>,
    expiry: BTreeSet<(Instant, Tuple)>,
    controls: BTreeSet<(Instant, Tuple)>,
    peers: BTreeMap<Ipv4Addr, usize>,
}
impl Table {
    fn remove(&mut self, tuple: Tuple) {
        if let Some(flow) = self.flows.remove(&tuple) {
            self.tokens.remove(&flow.token);
            self.expiry.remove(&(flow.expires, tuple));
            if let Some(due) = flow.receipt_due {
                self.controls.remove(&(due, tuple));
            }
            let count = self.peers.get_mut(&flow.peer).expect("flow peer quota");
            *count -= 1;
            if *count == 0 {
                self.peers.remove(&flow.peer);
            }
        }
    }
    fn put(&mut self, tuple: Tuple, flow: Flow) {
        self.remove(tuple);
        self.tokens.insert(flow.token, tuple);
        if !flow.prepared && !flow.delivering {
            self.expiry.insert((flow.expires, tuple));
        }
        if let Some(due) = flow.receipt_due {
            self.controls.insert((due, tuple));
        }
        *self.peers.entry(flow.peer).or_default() += 1;
        self.flows.insert(tuple, flow);
    }
    fn room(&self, peer: Ipv4Addr) -> bool {
        self.flows.len() < CAPACITY && self.peers.get(&peer).copied().unwrap_or(0) < PEER_CAPACITY
    }
}

#[derive(Clone)]
pub struct StatefulIpv4Plan {
    local: Ipv4Addr,
    mtu: u16,
    outgoing: BTreeMap<Ipv4Addr, Vec<String>>,
    incoming: BTreeMap<Vec<String>, Ipv4Addr>,
    returns: BTreeMap<Ipv4Addr, Vec<String>>,
    flows: Arc<Mutex<Table>>,
    counters: Arc<Counters>,
    context: [u8; 32],
}
struct Envelope<'a> {
    kind: u8,
    token: [u8; 16],
    challenge: [u8; 16],
    packet: Packet,
    bytes: &'a [u8],
}
impl Envelope<'_> {
    fn reply(&self) -> bool {
        self.kind == REPLY
    }
    fn tuple(&self) -> Tuple {
        if self.reply() {
            self.packet.tuple.reverse()
        } else {
            self.packet.tuple
        }
    }
}

impl StatefulIpv4Plan {
    pub fn new(
        local: Ipv4Addr,
        mtu: u16,
        outgoing: BTreeMap<Ipv4Addr, Vec<String>>,
        incoming: BTreeMap<Vec<String>, Ipv4Addr>,
    ) -> Self {
        let returns = incoming
            .iter()
            .map(|(p, ip)| (*ip, p.iter().rev().cloned().collect()))
            .collect();
        Self {
            local,
            mtu,
            outgoing,
            incoming,
            returns,
            flows: Arc::new(Mutex::new(Table::default())),
            counters: Arc::default(),
            context: [0; 32],
        }
    }
    pub fn scoped(mut self, context: [u8; 32]) -> Self {
        self.context = context;
        self
    }
    pub fn fresh(&self) -> Self {
        let mut fresh = Self::new(
            self.local,
            self.mtu,
            self.outgoing.clone(),
            self.incoming.clone(),
        )
        .scoped(self.context);
        fresh.counters = self.counters.clone(); // observations carry no authority
        fresh
    }
    pub fn statistics(&self) -> FlowStatistics {
        let c = &self.counters;
        FlowStatistics {
            capacity_drops: c.capacity.load(Ordering::Relaxed),
            invalid_drops: c.invalid.load(Ordering::Relaxed),
            unproven_drops: c.unproven.load(Ordering::Relaxed),
            expired_flows: c.expired.load(Ordering::Relaxed),
            control_attempts: c.attempts.load(Ordering::Relaxed),
            control_failures: c.failures.load(Ordering::Relaxed),
        }
    }
    fn table(&self) -> std::sync::MutexGuard<'_, Table> {
        let mut table = self.flows.lock().unwrap_or_else(|error| {
            let mut table = error.into_inner();
            *table = Table::default();
            table
        });
        let now = Instant::now();
        while let Some(&(until, tuple)) = table.expiry.first() {
            if until > now {
                break;
            }
            table.remove(tuple);
            self.counters.expired.fetch_add(1, Ordering::Relaxed);
        }
        table
    }
    fn envelope<'a>(&self, bytes: &'a [u8]) -> Option<Envelope<'a>> {
        if bytes.len() < ENVELOPE_BYTES
            || &bytes[..4] != MAGIC
            || !matches!(bytes[4], FORWARD | REPLY | CONFIRMED)
            || bytes[37..69] != self.context
        {
            return None;
        }
        Some(Envelope {
            kind: bytes[4],
            token: bytes[5..21].try_into().ok()?,
            challenge: bytes[21..37].try_into().ok()?,
            packet: Packet::parse(&bytes[ENVELOPE_BYTES..], self.mtu)?,
            bytes: &bytes[ENVELOPE_BYTES..],
        })
    }
    fn header(&self, kind: u8, flow: &Flow) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(ENVELOPE_BYTES);
        bytes.extend_from_slice(MAGIC);
        bytes.push(kind);
        bytes.extend_from_slice(&flow.token);
        bytes.extend_from_slice(&flow.challenge);
        bytes.extend_from_slice(&self.context);
        bytes
    }
    fn invalid(&self) -> bool {
        self.counters.invalid.fetch_add(1, Ordering::Relaxed);
        false
    }
    fn capacity(&self) -> io::Error {
        self.counters.capacity.fetch_add(1, Ordering::Relaxed);
        io::Error::new(io::ErrorKind::WouldBlock, "stateful flow quota exhausted")
    }
    fn matches(flow: &Flow, path: &[String], envelope: &Envelope<'_>) -> bool {
        flow.token == envelope.token
            && flow.challenge == envelope.challenge
            && if envelope.reply() {
                flow.path.iter().rev().eq(path.iter())
            } else {
                flow.path == path
            }
    }
    fn can_receive(&self, flow: &Flow, path: &[String], envelope: &Envelope<'_>) -> bool {
        if flow.retire {
            return self.invalid();
        }
        if !Self::matches(flow, path, envelope) {
            return self.invalid();
        }
        if envelope.reply() && (!flow.initiator || !flow.sent) {
            self.counters.unproven.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        if !envelope.reply() && flow.initiator {
            return self.invalid();
        }
        flow.clone()
            .advance(envelope.packet, envelope.reply(), false)
    }
}

impl Admission for StatefulIpv4Plan {
    fn outgoing(&self, bytes: &[u8]) -> Option<&Vec<String>> {
        let packet = Packet::parse(bytes, self.mtu)?;
        if packet.tuple.src != self.local {
            self.invalid();
            return None;
        }
        let table = self.table();
        if let Some(flow) = table.flows.get(&packet.tuple.reverse())
            && !flow.initiator
        {
            if flow.active && !flow.retire && flow.clone().advance(packet, true, true) {
                return self
                    .returns
                    .get(&packet.tuple.dst)
                    .filter(|p| flow.path.iter().rev().eq(p.iter()));
            }
            self.invalid();
            return None;
        }
        let path = self.outgoing.get(&packet.tuple.dst)?;
        match table.flows.get(&packet.tuple) {
            Some(flow)
                if flow.initiator
                    && !flow.retire
                    && flow.path == *path
                    && flow.clone().advance(packet, false, true) =>
            {
                Some(path)
            }
            None if packet.opener => Some(path),
            _ => {
                self.invalid();
                None
            }
        }
    }
    fn encapsulate(&self, path: &[String], bytes: &[u8]) -> io::Result<Vec<u8>> {
        let packet = Packet::parse(bytes, self.mtu).ok_or_else(invalid)?;
        let mut table = self.table();
        let reply = table
            .flows
            .get(&packet.tuple.reverse())
            .is_some_and(|flow| {
                !flow.initiator
                    && flow.active
                    && flow.path.iter().rev().eq(path.iter())
                    && flow.clone().advance(packet, true, true)
            });
        let tuple = if reply {
            packet.tuple.reverse()
        } else {
            packet.tuple
        };
        let mut flow = if let Some(flow) = table.flows.get(&tuple) {
            flow.clone()
        } else {
            if !packet.opener
                || self
                    .outgoing
                    .get(&packet.tuple.dst)
                    .is_none_or(|p| p != path)
            {
                return Err(invalid());
            }
            if !table.room(packet.tuple.dst) {
                return Err(self.capacity());
            }
            let mut token = [0; 16];
            let mut challenge = [0; 16];
            getrandom::fill(&mut token).map_err(|_| invalid())?;
            getrandom::fill(&mut challenge).map_err(|_| invalid())?;
            if table.tokens.contains_key(&token) {
                return Err(invalid());
            }
            Flow {
                token,
                challenge,
                path: path.to_vec(),
                peer: packet.tuple.dst,
                initiator: true,
                active: false,
                sent: false,
                prepared: false,
                delivering: false,
                retire: false,
                expires: Instant::now() + Duration::from_secs(30),
                tcp: packet.tcp.map(tcp::State::new),
                receipt_due: None,
                receipt_attempts: 0,
            }
        };
        if flow.prepared {
            return Err(self.capacity());
        }
        if !flow.clone().advance(packet, reply, true) {
            return Err(invalid());
        }
        let kind = if reply {
            REPLY
        } else if flow.active {
            CONFIRMED
        } else {
            FORWARD
        };
        let mut payload = self.header(kind, &flow);
        payload.extend_from_slice(bytes);
        flow.prepared = true;
        table.put(tuple, flow); // quota reserved before any Runtime enqueue
        Ok(payload)
    }
    fn sent(&self, path: &[String], payload: &[u8]) -> io::Result<()> {
        let envelope = self.envelope(payload).ok_or_else(invalid)?;
        let tuple = envelope.tuple();
        let mut table = self.table();
        let Some(mut flow) = table.flows.get(&tuple).cloned() else {
            return Err(invalid());
        };
        if !Self::matches(&flow, path, &envelope) {
            return Err(invalid());
        }
        if flow.retire {
            flow.prepared = false;
            if flow.delivering {
                table.put(tuple, flow);
            } else {
                table.remove(tuple);
            }
            return Ok(());
        }
        if !flow.advance(envelope.packet, envelope.reply(), true) {
            flow.prepared = false;
            table.put(tuple, flow);
            self.invalid();
            return Ok(());
        }
        flow.prepared = false;
        flow.sent = true;
        flow.expires = Instant::now() + flow.idle(tuple.protocol);
        if envelope.reply() && tuple.protocol == Protocol::Echo {
            flow.retire = true;
            if flow.delivering {
                table.put(tuple, flow);
            } else {
                table.remove(tuple);
            }
        } else {
            table.put(tuple, flow);
        }
        Ok(())
    }
    fn cancelled(&self, path: &[String], payload: &[u8]) {
        if let Some(envelope) = self.envelope(payload) {
            let tuple = envelope.tuple();
            let mut table = self.table();
            if let Some(mut flow) = table.flows.get(&tuple).cloned()
                && Self::matches(&flow, path, &envelope)
            {
                flow.prepared = false;
                if flow.retire && !flow.delivering {
                    table.remove(tuple);
                    return;
                }
                flow.expires = Instant::now() + flow.idle(tuple.protocol);
                table.put(tuple, flow); // retry reuses the same attempt, not active authority
            }
        }
    }
    fn incoming(&self, packet: &ReceivedPacket) -> bool {
        let Some(envelope) = self.envelope(&packet.payload) else {
            return self.invalid();
        };
        if packet.path.first() != Some(&packet.source) || envelope.packet.tuple.dst != self.local {
            return self.invalid();
        }
        let source = envelope.packet.tuple.src;
        let bound = if envelope.reply() {
            self.outgoing
                .get(&source)
                .is_some_and(|p| p.iter().rev().eq(packet.path.iter()))
        } else {
            self.incoming.get(&packet.path) == Some(&source)
        };
        if !bound {
            return self.invalid();
        }
        let tuple = envelope.tuple();
        let mut table = self.table();
        if let Some(mut flow) = table.flows.get(&tuple).cloned() {
            if !self.can_receive(&flow, &packet.path, &envelope) {
                return false;
            }
            flow.delivering = true;
            table.put(tuple, flow);
            return true;
        }
        if envelope.reply() || !envelope.packet.opener {
            self.counters.unproven.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        if !table.room(source) {
            self.counters.capacity.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        if table.tokens.contains_key(&envelope.token) {
            return self.invalid();
        }
        let flow = Flow {
            token: envelope.token,
            challenge: envelope.challenge,
            path: packet.path.clone(),
            peer: source,
            initiator: false,
            active: false,
            sent: false,
            prepared: false,
            delivering: true,
            retire: false,
            expires: Instant::now() + Duration::from_secs(5),
            tcp: envelope.packet.tcp.map(tcp::State::new),
            receipt_due: None,
            receipt_attempts: 0,
        };
        table.put(tuple, flow);
        true // reserved only; no reverse permission before Device success
    }
    fn plaintext(&self, packet: &ReceivedPacket) -> io::Result<Vec<u8>> {
        Ok(self
            .envelope(&packet.payload)
            .ok_or_else(invalid)?
            .bytes
            .to_vec())
    }
    fn delivered(&self, packet: &ReceivedPacket) -> io::Result<()> {
        let envelope = self.envelope(&packet.payload).ok_or_else(invalid)?;
        let tuple = envelope.tuple();
        let mut table = self.table();
        let Some(mut flow) = table.flows.get(&tuple).cloned() else {
            return Err(invalid());
        };
        if !Self::matches(&flow, &packet.path, &envelope)
            || !flow.advance(envelope.packet, envelope.reply(), false)
        {
            return Err(invalid());
        }
        flow.delivering = false;
        flow.retire = false;
        flow.active = true;
        flow.expires = Instant::now() + flow.idle(tuple.protocol);
        if envelope.kind == FORWARD && flow.receipt_due.is_none() {
            flow.receipt_due = Some(Instant::now());
            flow.receipt_attempts = 0;
        } else if envelope.kind == CONFIRMED {
            flow.receipt_due = None;
        }
        if envelope.reply() && tuple.protocol == Protocol::Echo {
            flow.retire = true;
            if flow.prepared {
                table.put(tuple, flow);
            } else {
                table.remove(tuple);
            }
        } else {
            table.put(tuple, flow);
        }
        Ok(())
    }
    fn receipt(&self, packet: &ReceivedPacket) -> Option<Vec<u8>> {
        let envelope = self.envelope(&packet.payload)?;
        if envelope.reply() {
            return None;
        }
        let table = self.table();
        let flow = table.flows.get(&envelope.tuple())?;
        (flow.active && !flow.initiator && Self::matches(flow, &packet.path, &envelope))
            .then(|| self.header(RECEIPT, flow))
    }
    fn control(&self, packet: &ReceivedPacket) -> io::Result<bool> {
        let bytes = &packet.payload;
        if bytes.len() < 5 || &bytes[..4] != MAGIC || bytes[4] != RECEIPT {
            return Ok(false);
        }
        if bytes.len() != ENVELOPE_BYTES
            || bytes[37..69] != self.context
            || packet.path.first() != Some(&packet.source)
        {
            self.invalid();
            return Ok(true);
        }
        let token: [u8; 16] = bytes[5..21].try_into().map_err(|_| invalid())?;
        let mut table = self.table();
        let Some(tuple) = table.tokens.get(&token).copied() else {
            self.counters.unproven.fetch_add(1, Ordering::Relaxed);
            return Ok(true);
        };
        let mut flow = table.flows[&tuple].clone();
        if !flow.initiator
            || !flow.sent
            || bytes[21..37] != flow.challenge
            || !flow.path.iter().rev().eq(packet.path.iter())
        {
            self.counters.unproven.fetch_add(1, Ordering::Relaxed);
            return Ok(true);
        }
        flow.active = true; // replay does not refresh idle expiry or TCP cursors
        table.put(tuple, flow);
        Ok(true)
    }
    fn next_control(&self) -> Option<(Vec<String>, Vec<u8>)> {
        let mut table = self.table();
        let &(due, tuple) = table.controls.first()?;
        if due > Instant::now() {
            return None;
        }
        let mut flow = table.flows[&tuple].clone();
        let packet = self.header(RECEIPT, &flow);
        let path = flow.path.iter().rev().cloned().collect();
        flow.receipt_attempts += 1;
        flow.receipt_due = (flow.receipt_attempts < 3).then(|| {
            Instant::now() + Duration::from_millis(250 * u64::from(flow.receipt_attempts))
        });
        table.put(tuple, flow);
        self.counters.attempts.fetch_add(1, Ordering::Relaxed);
        Some((path, packet))
    }
    fn control_failed(&self) {
        self.counters.failures.fetch_add(1, Ordering::Relaxed);
    }
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid stateful packet transition",
    )
}

#[cfg(test)]
mod tests;
