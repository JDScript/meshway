//! Generation-owned IPv4 return admission. Relays only carry encrypted envelopes;
//! endpoint state is created after an authenticated delivery receipt or complete
//! device acceptance. Enqueue alone creates only bounded pending receipt state.
//! No fragments, TCP midstream pickup, ICMP errors, or persistent flow state.
mod packet;
use crate::{ReceivedPacket, packet_io::Admission};
use packet::Packet;
use std::{
    collections::BTreeMap,
    io,
    net::Ipv4Addr,
    sync::{Arc, Mutex},
};
use tokio::time::{Duration, Instant};

pub const ENVELOPE_BYTES: usize = 69;
const MAGIC: &[u8; 4] = b"MWR1";
const CAPACITY: usize = 1024;

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
    path: Vec<String>,
    created: Instant,
    touched: Instant,
    tcp: Option<TcpState>,
}

#[derive(Clone, Copy)]
struct TcpState {
    syn: u32,
    peer_syn: Option<u32>,
    end: [u32; 2],
    established: bool,
    closing: bool,
}

impl Flow {
    fn live(&self, now: Instant, protocol: Protocol) -> bool {
        let idle = match protocol {
            Protocol::Tcp if self.tcp.is_some_and(|s| s.closing) => 5,
            Protocol::Tcp => 120,
            Protocol::Udp | Protocol::Echo => 30,
        };
        now.duration_since(self.touched) < Duration::from_secs(idle)
            && now.duration_since(self.created) < Duration::from_secs(300)
    }

    fn advance(&mut self, packet: Packet, reply: bool) -> bool {
        let Some(tcp) = packet.tcp else {
            return packet.tuple.protocol != Protocol::Echo || packet.opener != reply;
        };
        let Some(state) = self.tcp.as_mut() else {
            return false;
        };
        if tcp.flags == 2 {
            return !reply && !state.established && tcp.seq == state.syn && tcp.len == 0;
        }
        if tcp.flags == 0x12 {
            if !reply
                || tcp.ack != state.syn.wrapping_add(1)
                || tcp.len != 0
                || state.peer_syn.is_some_and(|seq| seq != tcp.seq)
                || state.established
            {
                return false;
            }
            state.peer_syn = Some(tcp.seq);
            state.end[1] = tcp.seq.wrapping_add(1);
            return true;
        }
        if reply
            && tcp.flags == 0x14
            && state.peer_syn.is_none()
            && tcp.ack == state.syn.wrapping_add(1)
            && tcp.seq == 0
            && tcp.len == 0
        {
            state.closing = true;
            return true;
        }
        // A bare ACK, RST, or data packet never creates a flow. SYN/URG and
        // unsolicited FIN are rejected; ACKs cannot acknowledge unseen bytes.
        if tcp.flags & 0x10 == 0 || tcp.flags & 0x22 != 0 || state.peer_syn.is_none() {
            return false;
        }
        let direction = usize::from(reply);
        let other = 1 - direction;
        let behind = state.end[direction].wrapping_sub(tcp.seq);
        let ack_behind = state.end[other].wrapping_sub(tcp.ack);
        if behind > 65_535 || ack_behind > 65_535 {
            return false;
        }
        if !state.established {
            if reply || tcp.seq != state.end[0] || tcp.ack != state.end[1] {
                return false;
            }
            state.established = true;
        }
        let consumed = tcp.len + u32::from(tcp.flags & 1 != 0);
        if consumed > behind {
            state.end[direction] = tcp.seq.wrapping_add(consumed);
        }
        state.closing |= tcp.flags & 5 != 0;
        true
    }
}

/// Clone shares one generation's bounded table. A replacement must construct a
/// fresh plan even when Runtime's cryptographic caches are reused.
#[derive(Clone)]
pub struct StatefulIpv4Plan {
    local: Ipv4Addr,
    mtu: u16,
    outgoing: BTreeMap<Ipv4Addr, Vec<String>>,
    incoming: BTreeMap<Vec<String>, Ipv4Addr>,
    returns: BTreeMap<Ipv4Addr, Vec<String>>,
    flows: Arc<Mutex<BTreeMap<Tuple, Flow>>>,
    pending: Arc<Mutex<BTreeMap<[u8; 16], Pending>>>,
    context: [u8; 32],
}

struct Pending {
    path: Vec<String>,
    payload: Vec<u8>,
    expires: Instant,
}

struct Envelope<'a> {
    reply: bool,
    token: [u8; 16],
    packet: Packet,
    bytes: &'a [u8],
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
            .map(|(path, ip)| (*ip, path.iter().rev().cloned().collect()))
            .collect();
        Self {
            local,
            mtu,
            outgoing,
            incoming,
            returns,
            flows: Arc::new(Mutex::new(BTreeMap::new())),
            pending: Arc::new(Mutex::new(BTreeMap::new())),
            context: [0; 32],
        }
    }

    pub fn scoped(mut self, context: [u8; 32]) -> Self {
        self.context = context;
        self
    }

    /// Each activation starts empty even when its immutable plan is reused.
    pub fn fresh(&self) -> Self {
        Self::new(
            self.local,
            self.mtu,
            self.outgoing.clone(),
            self.incoming.clone(),
        )
        .scoped(self.context)
    }

    fn table(&self) -> std::sync::MutexGuard<'_, BTreeMap<Tuple, Flow>> {
        // Poisoning cannot turn a partially advanced flow into an authorization.
        let mut table = self.flows.lock().unwrap_or_else(|e| {
            let mut guard = e.into_inner();
            guard.clear();
            guard
        });
        table.retain(|tuple, flow| flow.live(Instant::now(), tuple.protocol));
        table
    }

    fn envelope<'a>(&self, payload: &'a [u8]) -> Option<Envelope<'a>> {
        if payload.len() < ENVELOPE_BYTES
            || &payload[..4] != MAGIC
            || payload[4] > 1
            || payload[37..69] != self.context
        {
            return None;
        }
        let bytes = &payload[ENVELOPE_BYTES..];
        Some(Envelope {
            reply: payload[4] == 1,
            token: payload[5..21].try_into().ok()?,
            packet: Packet::parse(bytes, self.mtu)?,
            bytes,
        })
    }

    fn candidate(&self, path: &[String], envelope: &Envelope<'_>) -> Option<Flow> {
        self.candidate_in(&self.table(), path, envelope)
    }

    fn candidate_in(
        &self,
        table: &BTreeMap<Tuple, Flow>,
        path: &[String],
        envelope: &Envelope<'_>,
    ) -> Option<Flow> {
        let tuple = if envelope.reply {
            envelope.packet.tuple.reverse()
        } else {
            envelope.packet.tuple
        };
        let forward: Vec<_> = if envelope.reply {
            path.iter().rev().cloned().collect()
        } else {
            path.to_vec()
        };
        let mut flow = if let Some(flow) = table.get(&tuple) {
            if flow.token != envelope.token || flow.path != forward {
                return None;
            }
            flow.clone()
        } else {
            if envelope.reply || !envelope.packet.opener || table.len() >= CAPACITY {
                return None;
            }
            let now = Instant::now();
            Flow {
                token: envelope.token,
                path: forward,
                created: now,
                touched: now,
                tcp: envelope.packet.tcp.map(|tcp| TcpState {
                    syn: tcp.seq,
                    peer_syn: None,
                    end: [tcp.seq.wrapping_add(1), 0],
                    established: false,
                    closing: false,
                }),
            }
        };
        if !flow.advance(envelope.packet, envelope.reply) {
            return None;
        }
        flow.touched = Instant::now();
        Some(flow)
    }

    fn commit(&self, path: &[String], payload: &[u8]) -> io::Result<()> {
        let envelope = self.envelope(payload).ok_or_else(invalid)?;
        let mut table = self.table();
        let tuple = if envelope.reply {
            envelope.packet.tuple.reverse()
        } else {
            envelope.packet.tuple
        };
        if !table.contains_key(&tuple) && table.len() >= CAPACITY {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "stateful IPv4 flow capacity exhausted",
            ));
        }
        let flow = self
            .candidate_in(&table, path, &envelope)
            .ok_or_else(invalid)?;
        // An ICMP echo reply consumes its exact identifier/sequence capability.
        if envelope.reply && tuple.protocol == Protocol::Echo {
            table.remove(&tuple);
        } else {
            table.insert(tuple, flow);
        }
        Ok(())
    }
}

fn invalid() -> io::Error {
    io::Error::other("stateful IPv4 admission changed or invalid")
}

impl Admission for StatefulIpv4Plan {
    fn outgoing(&self, bytes: &[u8]) -> Option<&Vec<String>> {
        let packet = Packet::parse(bytes, self.mtu)?;
        if packet.tuple.src != self.local {
            return None;
        }
        if let Some(path) = self.returns.get(&packet.tuple.dst) {
            let table = self.table();
            if table.get(&packet.tuple.reverse()).is_some_and(|flow| {
                flow.path.iter().rev().eq(path.iter()) && flow.clone().advance(packet, true)
            }) {
                return Some(path);
            }
        }
        let path = self.outgoing.get(&packet.tuple.dst)?;
        let table = self.table();
        match table.get(&packet.tuple) {
            Some(flow) if flow.path == *path && flow.clone().advance(packet, false) => Some(path),
            None if packet.opener => Some(path),
            _ => None,
        }
    }

    fn encapsulate(&self, path: &[String], bytes: &[u8]) -> io::Result<Vec<u8>> {
        let packet = Packet::parse(bytes, self.mtu).ok_or_else(invalid)?;
        let table = self.table();
        let returning = table.get(&packet.tuple.reverse()).filter(|flow| {
            flow.path.iter().rev().eq(path.iter()) && (*flow).clone().advance(packet, true)
        });
        let reply = returning.is_some();
        let token = if let Some(flow) = returning.or_else(|| table.get(&packet.tuple)) {
            flow.token
        } else {
            if !packet.opener || table.len() >= CAPACITY {
                return Err(invalid());
            }
            let mut token = [0; 16];
            getrandom::fill(&mut token).map_err(|_| invalid())?;
            token
        };
        let mut payload = Vec::with_capacity(ENVELOPE_BYTES + bytes.len());
        payload.extend_from_slice(MAGIC);
        payload.push(u8::from(reply));
        payload.extend_from_slice(&token);
        let mut receipt = [0; 16];
        getrandom::fill(&mut receipt).map_err(|_| invalid())?;
        payload.extend_from_slice(&receipt);
        payload.extend_from_slice(&self.context);
        payload.extend_from_slice(bytes);
        Ok(payload)
    }

    fn sent(&self, path: &[String], payload: &[u8]) -> io::Result<()> {
        self.envelope(payload).ok_or_else(invalid)?;
        let mut pending = self.pending.lock().map_err(|_| invalid())?;
        pending.retain(|_, p| p.expires > Instant::now());
        if pending.len() >= CAPACITY {
            return Err(io::Error::other("stateful IPv4 receipt capacity exhausted"));
        }
        let receipt = payload[21..37].try_into().map_err(|_| invalid())?;
        if pending.contains_key(&receipt) {
            return Err(invalid());
        }
        pending.insert(
            receipt,
            Pending {
                path: path.to_vec(),
                payload: payload.to_vec(),
                expires: Instant::now() + Duration::from_secs(5),
            },
        );
        Ok(())
    }

    fn incoming(&self, packet: &ReceivedPacket) -> bool {
        let Some(envelope) = self.envelope(&packet.payload) else {
            return false;
        };
        if packet.path.first() != Some(&packet.source) || envelope.packet.tuple.dst != self.local {
            return false;
        }
        let source = envelope.packet.tuple.src;
        let bound = if envelope.reply {
            self.outgoing
                .get(&source)
                .is_some_and(|forward| forward.iter().rev().eq(packet.path.iter()))
        } else {
            self.incoming.get(&packet.path) == Some(&source)
        };
        bound && self.candidate(&packet.path, &envelope).is_some()
    }

    fn plaintext(&self, packet: &ReceivedPacket) -> io::Result<Vec<u8>> {
        Ok(self
            .envelope(&packet.payload)
            .ok_or_else(invalid)?
            .bytes
            .to_vec())
    }

    fn delivered(&self, packet: &ReceivedPacket) -> io::Result<()> {
        self.commit(&packet.path, &packet.payload)
    }

    fn receipt(&self, packet: &ReceivedPacket) -> Option<Vec<u8>> {
        self.envelope(&packet.payload)?;
        let mut receipt = packet.payload[..ENVELOPE_BYTES].to_vec();
        receipt[4] = 2;
        Some(receipt)
    }

    fn control(&self, packet: &ReceivedPacket) -> io::Result<bool> {
        let bytes = &packet.payload;
        if bytes.len() < 5 || &bytes[..4] != MAGIC || bytes[4] != 2 {
            return Ok(false);
        }
        if bytes.len() != ENVELOPE_BYTES
            || bytes[37..69] != self.context
            || packet.path.first() != Some(&packet.source)
        {
            return Ok(true);
        }
        let id: [u8; 16] = bytes[21..37].try_into().map_err(|_| invalid())?;
        let mut pending = self.pending.lock().map_err(|_| invalid())?;
        pending.retain(|_, p| p.expires > Instant::now());
        let Some(sent) = pending.get(&id) else {
            return Ok(true);
        };
        if !sent.path.iter().rev().eq(packet.path.iter()) || sent.payload[5..21] != bytes[5..21] {
            return Ok(true);
        }
        let sent = pending.remove(&id).ok_or_else(invalid)?;
        // Late/out-of-order receipts cannot roll an advanced flow back. A
        // rejected transition is discarded rather than restarting the flow.
        if let Err(error) = self.commit(&sent.path, &sent.payload)
            && error.kind() == io::ErrorKind::OutOfMemory
        {
            return Err(error);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests;
