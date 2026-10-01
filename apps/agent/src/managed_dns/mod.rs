//! Local authoritative name publication, not OS DNS integration or domain routing.
//! Only a freshly validated and privately persisted controller snapshot can publish.

use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{Arc, RwLock},
    time::Duration,
};

use hickory_proto::{
    op::{Edns, Message, MessageType, OpCode, ResponseCode},
    rr::{
        DNSClass, Name, RData, Record, RecordType,
        rdata::{A, AAAA, PTR},
    },
    serialize::binary::{BinDecodable, BinDecoder},
};
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream, UdpSocket},
    sync::watch,
    task::JoinSet,
    time::{Instant, timeout},
};

use crate::AgentError;

pub(crate) mod log;
mod send;

pub(crate) const MAX_UDP: usize = 1232;
pub(crate) const MAX_REQUEST: usize = 4096;
const MAX_TCP_CLIENTS: usize = 32;
const TCP_TIMEOUT: Duration = Duration::from_secs(3);

/// Parsed DNS-only view. These are internal indexes, never duplicate HTTP DTOs.
pub(crate) struct Zone {
    pub name: Name,
    pub ttl: u32,
    pub forward: BTreeMap<Name, (Ipv4Addr, Ipv6Addr)>,
    pub reverse: BTreeMap<Name, Name>,
}

struct Publication {
    zone: Arc<Zone>,
    deadline: Instant,
    revision: u32,
}

#[derive(Default)]
struct State {
    generation: Arc<()>,
    generation_number: u64,
    active: Option<Publication>,
    disabled: bool,
    expired: bool,
    revision_floor: u32,
    terminated: bool,
    listener: Option<SocketAddr>,
    listener_bound: bool,
    network_id: Option<String>,
    node_id: Option<String>,
    zone: Option<String>,
    ttl: Option<u32>,
}

/// Query-side freshness checks remain independent of any stalled HTTP future.
#[derive(Clone)]
pub(crate) struct Guard(Arc<RwLock<State>>, watch::Sender<()>);

impl Default for Guard {
    fn default() -> Self {
        let (changes, _) = watch::channel(());
        Self(Arc::new(RwLock::new(State::default())), changes)
    }
}

/// A bounded question and publication fence, not a detached serialized answer.
/// Bytes and TTL are generated only under the guard at the nonblocking send.
struct Response {
    query: Message,
    limit: usize,
    tcp: bool,
    generation: Arc<()>,
    deadline: Option<Instant>,
}

impl Guard {
    fn changed(&self, state: &mut State) {
        state.generation = Arc::new(());
        state.generation_number = state.generation_number.saturating_add(1);
        self.1.send_replace(());
    }

    pub fn listener_bound(&self, listener: SocketAddr, node_id: Option<String>) {
        let mut state = self.0.write().expect("DNS guard poisoned");
        state.listener = Some(listener);
        state.listener_bound = true;
        state.node_id = node_id;
        self.changed(&mut state);
    }

    pub fn listener_closed(&self) {
        let mut state = self.0.write().expect("DNS guard poisoned");
        state.listener_bound = false;
        self.changed(&mut state);
    }

    #[cfg(unix)]
    pub fn status(&self) -> crate::diagnostics::dns::Status {
        use crate::diagnostics::dns::{State as StatusState, Status, StatusEvidence};
        let state = self.0.read().expect("DNS guard poisoned");
        let remaining = state.active.as_ref().map_or(Duration::ZERO, |active| {
            active.deadline.saturating_duration_since(Instant::now())
        });
        let status = if state.terminated || !state.listener_bound {
            StatusState::Unavailable
        } else if state.disabled {
            StatusState::Disabled
        } else if state.expired || (state.active.is_some() && remaining.as_secs() == 0) {
            StatusState::Expired
        } else if state.active.is_some() {
            StatusState::Active
        } else {
            StatusState::Unavailable
        };
        Status {
            schema_version: crate::diagnostics::SCHEMA_VERSION,
            state: status,
            generation: state.generation_number,
            revision_floor: state.revision_floor,
            active_revision: (status == StatusState::Active)
                .then(|| state.active.as_ref().expect("active publication").revision),
            zone: state.zone.clone(),
            ttl: state.ttl,
            lease_remaining_ms: if status == StatusState::Active {
                remaining.as_millis().min(u128::from(u64::MAX)) as u64
            } else {
                0
            },
            listener: state.listener,
            listener_bound: state.listener_bound,
            network_id: state.network_id.clone(),
            node_id: state.node_id.clone(),
            evidence: StatusEvidence::CurrentDnsGuard,
        }
    }

    pub fn withdraw(&self) {
        let mut state = self.0.write().expect("DNS guard poisoned");
        state.active = None;
        state.disabled = false;
        state.expired = false;
        self.changed(&mut state);
    }

    #[cfg(test)]
    pub fn disable(&self) {
        let mut state = self.0.write().expect("DNS guard poisoned");
        state.active = None;
        state.disabled = !state.terminated;
        state.expired = false;
        self.changed(&mut state);
    }

    pub fn shutdown(&self) {
        let mut state = self.0.write().expect("DNS guard poisoned");
        state.active = None;
        state.disabled = false;
        state.terminated = true;
        state.expired = false;
        self.changed(&mut state);
    }

    pub fn observe_revision(&self, revision: u32) {
        let mut state = self.0.write().expect("DNS guard poisoned");
        state.revision_floor = state.revision_floor.max(revision);
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.revision < revision)
        {
            state.active = None;
            state.disabled = false;
            state.expired = false;
            self.changed(&mut state);
        }
    }

    pub fn revision_floor(&self) -> u32 {
        self.0.read().expect("DNS guard poisoned").revision_floor
    }

    pub fn expire(&self, now: Instant) -> bool {
        let mut state = self.0.write().expect("DNS guard poisoned");
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.deadline.saturating_duration_since(now).as_secs() == 0)
        {
            state.active = None;
            state.expired = true;
            self.changed(&mut state);
            return true;
        }
        false
    }

    pub fn conflict(&self, revision: u32) {
        let mut state = self.0.write().expect("DNS guard poisoned");
        state.revision_floor = state.revision_floor.max(revision.saturating_add(1));
        state.active = None;
        state.disabled = false;
        state.expired = false;
        self.changed(&mut state);
    }

    #[cfg(test)]
    pub fn publish(&self, zone: Zone, deadline: Instant, revision: u32) -> bool {
        self.publish_snapshot(zone, deadline, revision, true, None, None)
    }

    pub fn publish_snapshot(
        &self,
        zone: Zone,
        deadline: Instant,
        revision: u32,
        enabled: bool,
        network_id: Option<String>,
        node_id: Option<String>,
    ) -> bool {
        let mut state = self.0.write().expect("DNS guard poisoned");
        if state.terminated
            || revision < state.revision_floor
            || deadline.saturating_duration_since(Instant::now()).as_secs() == 0
        {
            return false;
        }
        state.revision_floor = revision;
        state.zone = Some(zone.name.to_string());
        state.ttl = Some(zone.ttl);
        state.network_id = network_id;
        state.node_id = node_id;
        state.active = enabled.then(|| Publication {
            zone: Arc::new(zone),
            deadline,
            revision,
        });
        state.disabled = !enabled;
        state.expired = false;
        self.changed(&mut state);
        true
    }

    #[cfg(test)]
    fn answer(&self, query: &Message, now: Instant) -> Message {
        Self::answer_locked(&self.0.read().expect("DNS guard poisoned"), query, now)
    }

    fn answer_locked(state: &State, query: &Message, now: Instant) -> Message {
        let mut response = Message::response(query.metadata.id, query.metadata.op_code);
        response.metadata.recursion_desired = query.metadata.recursion_desired;
        // RD is echoed, but RA always remains false. No upstream exists.
        response.queries = query.queries.clone();
        if let Some(edns) = &query.edns {
            let mut reply = Edns::new();
            reply.set_max_payload(MAX_UDP as u16);
            response.edns = Some(reply);
            if edns.version() != 0 {
                response.metadata.response_code = ResponseCode::BADVERS;
                return response;
            }
        }
        if query.metadata.op_code != OpCode::Query {
            response.metadata.response_code = ResponseCode::NotImp;
            return response;
        }
        if query.queries.len() != 1
            || !query.answers.is_empty()
            || !query.authorities.is_empty()
            || !query.additionals.is_empty()
            || query.signature.is_some()
            || query.metadata.truncation
        {
            response.queries.truncate(1);
            response.metadata.response_code = ResponseCode::FormErr;
            return response;
        }
        let Some(active) = &state.active else {
            response.metadata.response_code = if state.disabled {
                ResponseCode::Refused
            } else {
                ResponseCode::ServFail
            };
            return response;
        };
        let remaining = active.deadline.saturating_duration_since(now).as_secs();
        if remaining == 0 {
            response.metadata.response_code = ResponseCode::ServFail;
            return response;
        }
        let zone = &active.zone;
        let question = &query.queries[0];
        let name = question.name().to_lowercase();
        let in_zone = zone.name.zone_of(&name);
        let reverse = zone.reverse.get(&name);
        if question.query_class() != DNSClass::IN
            || (!in_zone && reverse.is_none())
            || matches!(
                question.query_type(),
                RecordType::AXFR | RecordType::IXFR | RecordType::ANY
            )
        {
            response.metadata.response_code = ResponseCode::Refused;
            return response;
        }
        response.metadata.authoritative = true;
        let ttl = zone.ttl.min(remaining.min(u64::from(u32::MAX)) as u32);
        let data = if let Some(canonical) = reverse {
            (question.query_type() == RecordType::PTR).then(|| RData::PTR(PTR(canonical.clone())))
        } else if let Some((v4, v6)) = zone.forward.get(&name) {
            match question.query_type() {
                RecordType::A => Some(RData::A(A(*v4))),
                RecordType::AAAA => Some(RData::AAAA(AAAA(*v6))),
                _ => None, // Existing owner with unsupported type: NOERROR/NODATA.
            }
        } else if name == zone.name {
            None
        } else {
            response.metadata.response_code = ResponseCode::NXDomain;
            None
        };
        if let Some(data) = data {
            response.add_answer(Record::from_rdata(question.name().clone(), ttl, data));
        }
        // No SOA negative TTL is published, so negative answers do not outlive a lease.
        response
    }

    fn prepare(&self, bytes: &[u8], tcp: bool) -> Option<Response> {
        if bytes.len() > if tcp { MAX_REQUEST } else { MAX_UDP } {
            return None;
        }
        let mut decoder = BinDecoder::new(bytes);
        let query = Message::read(&mut decoder).ok()?;
        if decoder.index() != bytes.len() || query.metadata.message_type != MessageType::Query {
            return None;
        }
        let limit = if tcp {
            u16::MAX as usize
        } else {
            query.edns.as_ref().map_or(512, |edns| {
                usize::from(edns.max_payload()).clamp(512, MAX_UDP)
            })
        };
        let state = self.0.read().expect("DNS guard poisoned");
        Some(Response {
            query,
            limit,
            tcp,
            generation: state.generation.clone(),
            // New queries after expiry still get a fresh SERVFAIL even before
            // the watchdog retires the publication. Previously prepared
            // positive responses retain their deadline and are canceled.
            deadline: state
                .active
                .as_ref()
                .map(|active| active.deadline)
                .filter(|deadline| {
                    deadline.saturating_duration_since(Instant::now()).as_secs() > 0
                }),
        })
    }

    // Tests inspect point-in-time bytes. Production socket writes exclusively
    // use send's readiness/generation fence and locked nonblocking syscall.
    #[cfg(test)]
    pub fn respond(&self, bytes: &[u8], tcp: bool) -> Option<Vec<u8>> {
        let response = self.prepare(bytes, tcp)?;
        let state = self.0.read().expect("DNS guard poisoned");
        encode_bounded(
            Self::answer_locked(&state, &response.query, Instant::now()),
            response.limit,
        )
    }
}

fn encode_bounded(response: Message, limit: usize) -> Option<Vec<u8>> {
    let bytes = response.to_vec().ok()?;
    if bytes.len() <= limit {
        return Some(bytes);
    }
    // Truncate at whole-message/RR boundaries, never chop serialized bytes.
    let bytes = response.truncate().to_vec().ok()?;
    (bytes.len() <= limit).then_some(bytes)
}

pub fn validate_listen(address: SocketAddr) -> Result<(), AgentError> {
    if !address.ip().is_loopback() || address.port() < 1024 {
        return Err(AgentError::Invalid(
            "DNS listen requires a loopback IP and port >= 1024",
        ));
    }
    Ok(())
}

/// Both protocols bind before readiness. Cancellation drops listeners and every
/// accepted connection; no detached socket tasks can keep stale answers alive.
pub(crate) struct Server {
    udp: UdpSocket,
    tcp: TcpListener,
}

impl Server {
    pub async fn bind(address: SocketAddr) -> Result<Self, AgentError> {
        validate_listen(address)?;
        let udp = UdpSocket::bind(address).await?;
        let tcp = TcpListener::bind(address).await?;
        Ok(Self { udp, tcp })
    }

    pub async fn serve(self, guard: Guard) -> Result<(), AgentError> {
        let mut requests = [0; MAX_UDP + 1];
        let mut clients = JoinSet::new();
        loop {
            tokio::select! {
                packet = self.udp.recv_from(&mut requests) => {
                    let (length, peer) = packet?;
                    if peer.ip().is_loopback() {
                        send::udp(&self.udp, &guard, &requests[..length], peer).await?;
                    }
                }
                client = self.tcp.accept() => {
                    let (stream, peer) = client?;
                    if peer.ip().is_loopback() && clients.len() < MAX_TCP_CLIENTS {
                        clients.spawn(serve_tcp(stream, guard.clone()));
                    }
                }
                _ = clients.join_next(), if !clients.is_empty() => {}
            }
        }
    }
}

async fn serve_tcp(mut stream: TcpStream, guard: Guard) {
    // One transaction per connection keeps client work and memory strictly bounded.
    let _ = timeout(TCP_TIMEOUT, async {
        let length = usize::from(stream.read_u16().await?);
        if !(12..=MAX_REQUEST).contains(&length) {
            return Ok::<_, std::io::Error>(());
        }
        let mut request = vec![0; length];
        stream.read_exact(&mut request).await?;
        send::tcp(&stream, &guard, &request).await?;
        Ok(())
    })
    .await;
}

#[cfg(test)]
mod tests;
