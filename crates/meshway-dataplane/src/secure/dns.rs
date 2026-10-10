//! Bounded DNS request/reply correlations on DNS-purpose circuits. The sole
//! SecureNode owns Noise and nonces; these messages never enter the IP flow table.
use super::*;
use meshway_configuration::DnsTransport;
use meshway_model::EventId;

const MAX_PENDING: usize = 128;
pub const MAX_DNS_BYTES: usize = 4096;
const LIFETIME: Duration = Duration::from_secs(5);

struct Pending {
    source: NodeId,
    metadata: codec::ProbeMetadata,
    guard: Guard,
    question: Option<(DnsTransport, u16)>,
}

#[derive(Default)]
pub(super) struct Exchanges(BTreeMap<([u8; 32], EventId), Pending>);
impl Exchanges {
    fn maintain(&mut self) {
        self.0
            .retain(|_, request| request.guard.ensure_current().is_ok());
    }

    fn admit(&mut self, key: ([u8; 32], EventId), source: NodeId, limit: u16) -> Result<(), Error> {
        self.maintain();
        if self.0.contains_key(&key) {
            return Err(Error::Replay);
        }
        if self.0.len() >= MAX_PENDING
            || self.0.values().filter(|p| p.source == source).count() >= usize::from(limit)
        {
            return Err(Error::Capacity);
        }
        Ok(())
    }
}

/// Authenticated source and full path of one DNS query. Construction is
/// private to SecureNode; callers retain this value through upstream I/O.
pub struct DnsRequest {
    context: Arc<Context>,
    metadata: codec::ProbeMetadata,
    guard: Guard,
    transport: DnsTransport,
    bytes: Vec<u8>,
}

impl std::fmt::Debug for DnsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DnsRequest").finish_non_exhaustive()
    }
}

impl DnsRequest {
    pub fn source(&self) -> NodeId {
        self.context.path[0]
    }
    pub fn path(&self) -> &[NodeId] {
        &self.context.path
    }
    pub fn transport(&self) -> DnsTransport {
        self.transport
    }
    pub fn deadline(&self) -> Instant {
        self.guard.expiry().at
    }
    pub fn snapshot(&self) -> &ExecutionSnapshot {
        self.guard.snapshot()
    }
    pub fn bytes(&self) -> Result<&[u8], Error> {
        self.guard.ensure_current()?;
        Ok(&self.bytes)
    }
    pub async fn invalidated(&self) {
        self.guard.invalidated().await;
    }
}

/// A response to the source's exact pending request, still bound to its
/// original query guard at the consumer's final output boundary.
pub struct DnsReply {
    id: EventId,
    guard: Guard,
    bytes: Vec<u8>,
}
impl std::fmt::Debug for DnsReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DnsReply")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
impl DnsReply {
    pub fn id(&self) -> EventId {
        self.id
    }
    pub fn release<T>(self, sink: impl FnOnce(Vec<u8>) -> T) -> Result<T, Error> {
        self.guard.finish()?;
        Ok(sink(self.bytes))
    }
}

fn question(bytes: &[u8], reply: bool) -> Result<u16, Error> {
    if !(12..=MAX_DNS_BYTES).contains(&bytes.len()) || (bytes[2] & 0x80 != 0) != reply {
        return Err(Error::Wire);
    }
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn encrypt(
    noise: &crypto::Transport,
    sequence: u64,
    transport: DnsTransport,
    bytes: &[u8],
) -> Result<PacketBuffer, Error> {
    let mut body = Vec::with_capacity(bytes.len() + 1);
    body.push(match transport {
        DnsTransport::Udp => 0,
        DnsTransport::Tcp => 1,
    });
    body.extend_from_slice(bytes);
    let mut body = PacketBuffer::from(body);
    noise.encrypt(sequence, &mut body)?;
    Ok(body)
}

fn decrypt(
    noise: &crypto::Transport,
    sequence: u64,
    mut body: PacketBuffer,
    reply: bool,
) -> Result<(DnsTransport, Vec<u8>), Error> {
    noise.decrypt(sequence, &mut body)?;
    let transport = match body.first() {
        Some(0) => DnsTransport::Udp,
        Some(1) => DnsTransport::Tcp,
        _ => return Err(Error::Wire),
    };
    body.trim_front(1);
    question(&body, reply)?;
    Ok((transport, body.into_vec()))
}

impl SecureNode {
    pub fn query_dns(
        &mut self,
        circuit: [u8; 32],
        id: EventId,
        transport: DnsTransport,
        bytes: &[u8],
        deadline: Instant,
    ) -> Result<PendingFrame, Error> {
        let query_id = question(bytes, false)?;
        let snapshot = self.identity.snapshot().clone();
        let reading = View::of(&snapshot).map_err(|_| Error::Authority)?;
        self.maintain_circuits(&reading, false)?;
        let mut owned = self.circuits.remove(&circuit).ok_or(Error::State)?;
        let result = (|| {
            owned.check_at(&reading)?;
            if owned.context.grant.purpose != crate::Purpose::Dns
                || owned.context.path[0] != reading.node()
            {
                return Err(Error::Policy);
            }
            let service = self
                .engine
                .decisions
                .dns_service(reading.netmap(), reading.node())
                .map_err(|_| Error::Policy)?;
            if !service.transports.contains(&transport) {
                return Err(Error::Policy);
            }
            let State::Endpoint(noise) = &owned.state else {
                return Err(Error::State);
            };
            Self::check_outgoing_carrier(&self.carriers, &owned, false, &reading)?;
            let mut guard = owned.guard(&snapshot, false);
            let at = deadline
                .min(reading.instant + LIFETIME)
                .min(guard.expiry().at);
            guard.life.restrict(Some(Cap {
                at,
                ms: reading
                    .now_ms
                    .checked_add(
                        u64::try_from(at.saturating_duration_since(reading.instant).as_millis())
                            .map_err(|_| Error::Limits)?,
                    )
                    .ok_or(Error::Limits)?,
            }));
            guard.finish()?;
            let metadata = codec::ProbeMetadata {
                measurement_id: id,
                expires_at_ms: guard.expiry().ms,
            };
            self.dns
                .admit((circuit, id), reading.node(), service.max_pending)?;
            let sequence = owned.send[0].take()?;
            let frame = Frame {
                context: owned.context.clone(),
                kind: Kind::DnsRequest,
                reply: false,
                sequence,
                probe: Some(metadata),
                body: encrypt(noise, sequence, transport, bytes)?,
                proofs: Vec::new(),
                exchange: None,
                auth: Default::default(),
            };
            let hop = self.handshake_hop(&frame, None, &reading)?;
            let mut output = self.output(&owned, frame, hop, None, &reading)?;
            output.guard = guard.clone();
            guard.finish()?;
            self.dns.0.insert(
                (circuit, id),
                Pending {
                    source: reading.node(),
                    metadata,
                    guard,
                    question: Some((transport, query_id)),
                },
            );
            Ok(output)
        })();
        let result = owned.finish_operation(&snapshot, "dns_query", result);
        self.restore(circuit, owned, &reading);
        result
    }

    pub fn answer_dns(&mut self, request: DnsRequest, bytes: &[u8]) -> Result<PendingFrame, Error> {
        request.guard.ensure_current()?;
        if question(bytes, true)? != question(&request.bytes, false)? {
            return Err(Error::Context);
        }
        let snapshot = self.identity.snapshot().clone();
        let reading = View::of(&snapshot).map_err(|_| Error::Authority)?;
        let id = request.context.circuit;
        let mut owned = self.circuits.remove(&id).ok_or(Error::State)?;
        let result = (|| {
            owned.check_at(&reading)?;
            if owned.context.as_ref() != request.context.as_ref()
                || owned.context.path.last() != Some(&reading.node())
            {
                return Err(Error::Context);
            }
            let key = (id, request.metadata.measurement_id);
            let pending = self.dns.0.get(&key).ok_or(Error::Replay)?;
            if pending.metadata != request.metadata {
                return Err(Error::Context);
            }
            pending.guard.ensure_current()?;
            let State::Endpoint(noise) = &owned.state else {
                return Err(Error::State);
            };
            let sequence = owned.send[1].take()?;
            let frame = Frame {
                context: owned.context.clone(),
                kind: Kind::DnsReply,
                reply: true,
                sequence,
                probe: Some(request.metadata),
                body: encrypt(noise, sequence, request.transport, bytes)?,
                proofs: Vec::new(),
                exchange: None,
                auth: Default::default(),
            };
            let hop = self.handshake_hop(&frame, None, &reading)?;
            let mut output = self.output(&owned, frame, hop, None, &reading)?;
            output.guard = request.guard;
            output.guard.finish()?;
            self.dns.0.remove(&key);
            Ok(output)
        })();
        let result = owned.finish_operation(&snapshot, "dns_reply", result);
        self.restore(id, owned, &reading);
        result
    }

    pub(super) fn receive_dns(
        &mut self,
        circuit: &mut Circuit,
        frame: Frame,
        previous: NodeId,
        reading: &View<'_>,
    ) -> Result<Received, Error> {
        if !circuit.state.established() || circuit.context.grant.purpose != crate::Purpose::Dns {
            return Err(Error::State);
        }
        self.dns.maintain();
        let metadata = frame.probe.ok_or(Error::Wire)?;
        let key = (circuit.context.circuit, metadata.measurement_id);
        let snapshot = self.identity.snapshot().clone();
        circuit.check_at(reading)?;
        if metadata.expires_at_ms <= reading.now_ms
            || metadata.expires_at_ms - reading.now_ms > LIFETIME.as_millis() as u64
        {
            return Err(Error::Expired);
        }
        let hop = self.handshake_hop(&frame, Some(previous), reading)?;
        let index = usize::from(frame.reply);
        let sequence = frame.sequence;
        circuit.receive[index].check(sequence)?;
        let result = if frame.reply {
            let pending = self.dns.0.get(&key).ok_or(Error::Replay)?;
            if pending.metadata != metadata {
                return Err(Error::Context);
            }
            let guard = pending.guard.clone();
            let expected = pending.question;
            guard.ensure_current()?;
            let result = match (&circuit.state, hop) {
                (State::RelayEstablished, Hop::Forward(_)) => {
                    let mut output = self.output(circuit, frame, hop, None, reading)?;
                    output.guard = guard.clone();
                    Received::Forward(output)
                }
                (State::Endpoint(noise), Hop::Deliver) => {
                    let (transport, bytes) = decrypt(noise, sequence, frame.body, true)?;
                    if expected != Some((transport, question(&bytes, true)?)) {
                        return Err(Error::Context);
                    }
                    Received::DnsReply(DnsReply {
                        id: key.1,
                        bytes,
                        guard: guard.clone(),
                    })
                }
                _ => return Err(Error::State),
            };
            guard.finish()?;
            self.dns.0.remove(&key);
            result
        } else {
            let source = circuit.context.path[0];
            let service = self
                .engine
                .decisions
                .dns_service(reading.netmap(), source)
                .map_err(|_| Error::Policy)?;
            self.dns.admit(key, source, service.max_pending)?;
            let mut guard = circuit.guard(&snapshot, false);
            guard.life.restrict(Some(Cap {
                at: reading.instant
                    + Duration::from_millis(metadata.expires_at_ms - reading.now_ms),
                ms: metadata.expires_at_ms,
            }));
            let (result, question) = match (&circuit.state, hop) {
                (State::RelayEstablished, Hop::Forward(_)) => {
                    let mut output = self.output(circuit, frame, hop, None, reading)?;
                    output.guard = guard.clone();
                    (Received::Forward(output), None)
                }
                (State::Endpoint(noise), Hop::Deliver) => {
                    let (transport, bytes) = decrypt(noise, sequence, frame.body, false)?;
                    if !service.transports.contains(&transport) {
                        return Err(Error::Policy);
                    }
                    let id = question(&bytes, false)?;
                    (
                        Received::DnsRequest(DnsRequest {
                            context: circuit.context.clone(),
                            metadata,
                            guard: guard.clone(),
                            transport,
                            bytes,
                        }),
                        Some((transport, id)),
                    )
                }
                _ => return Err(Error::State),
            };
            guard.finish()?;
            self.dns.0.insert(
                key,
                Pending {
                    source,
                    metadata,
                    guard,
                    question,
                },
            );
            result
        };
        circuit.ensure_current(&snapshot)?;
        circuit.receive[index].commit(sequence);
        Ok(result)
    }
}
