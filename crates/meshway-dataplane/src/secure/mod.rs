//! Versioned candidate Mesh circuits, not a stable wire ABI.
//!
//! Noise IK protects IP payloads end to end between the original source and the
//! terminal node. Relays see authenticated five-tuples, TCP flags/sequence facts,
//! echo identifiers and packet lengths, plus ciphertext. Every forwarding node
//! verifies an ordered Ed25519 proof chain over the original full path, frame,
//! ciphertext and actual carrier descriptors. Replies use that exact path in
//! reverse; carrier changes never authorize another logical path.
//!
//! This synchronous owner opens no sockets and creates no background tasks.
//! Hosts retain guarded outputs through I/O readiness and release at their final
//! synchronous sink. Closing a circuit cancels its queued wire/plaintext outputs;
//! dropping the node cancels every circuit, independently of Control's lease.
//! Per-hop signatures prioritize correctness; no performance
//! acceptance or transport implementation is implied.
mod codec;
mod crypto;
mod replay;

use crate::{
    AuthenticatedPeer, Direction, Engine, Hop, Ingress, Packet, PendingAction, PendingRoute,
};
use codec::{Context, Frame, Kind};
use meshway_control::{DataPlaneIdentity, ExecutionSnapshot};
use meshway_model::{NodeId, RequestId};
pub use meshway_policy::admission::Carrier;
use meshway_policy::admission::{admit_carrier, admit_handshake};
use replay::{ReplayWindow, SendNonce};
use std::{
    collections::BTreeMap,
    fmt,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub const MAX_FRAME: usize = 128 * 1024;
/// Noise's 65535-byte message limit minus its 16-byte authentication tag.
pub const MAX_PLAINTEXT: usize = 65_519;

/// Bounded static labels: no raw keys, packet contents or configuration errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid candidate circuit frame")]
    Wire,
    #[error("unsupported circuit configuration or packet length")]
    Unsupported,
    #[error("execution authority is no longer current")]
    Authority,
    #[error("circuit does not match the installed context")]
    Context,
    #[error("circuit policy rejected the operation")]
    Policy,
    #[error("invalid circuit proof chain")]
    Proof,
    #[error("circuit cryptography failed")]
    Crypto,
    #[error("circuit replay rejected")]
    Replay,
    #[error("circuit nonce exhausted")]
    Nonce,
    #[error("circuit is absent or not established")]
    State,
    #[error("circuit lifetime expired")]
    Expired,
    #[error("circuit capacity reached")]
    Capacity,
    #[error("invalid circuit limits")]
    Limits,
    #[error("packet or authenticated packet facts rejected")]
    Packet,
    #[error("packet has no valid unambiguous local flow role")]
    Flow,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_circuits: usize,
    pub handshake_timeout: Duration,
    pub flows: crate::Limits,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_circuits: 1024,
            handshake_timeout: Duration::from_secs(10),
            flows: crate::Limits::default(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct Guard {
    snapshot: ExecutionSnapshot,
    expires_at: Instant,
    expires_at_ms: u64,
    cancellation: CancellationToken,
}
impl Guard {
    pub(crate) fn ensure_current(&self) -> Result<(), Error> {
        if self.cancellation.is_cancelled() {
            return Err(Error::Expired);
        }
        self.snapshot
            .ensure_current()
            .map_err(|_| Error::Authority)?;
        if Instant::now() >= self.expires_at
            || self.snapshot.now_ms().map_err(|_| Error::Authority)? >= self.expires_at_ms
        {
            return Err(Error::Expired);
        }
        self.snapshot
            .ensure_current()
            .map_err(|_| Error::Authority)?;
        if self.cancellation.is_cancelled() {
            return Err(Error::Expired);
        }
        Ok(())
    }
    pub(crate) async fn invalidated(&self) {
        if self.ensure_current().is_err() {
            return;
        }
        tokio::select! { biased;
            _ = self.cancellation.cancelled() => {},
            _ = self.snapshot.invalidated_at(self.expires_at_ms) => {},
            _ = tokio::time::sleep_until(self.expires_at.into()) => {},
        }
    }
    pub(crate) fn expires_at(&self) -> Instant {
        self.expires_at
    }
    /// An I/O-local child: cancelling one write must not close its circuit.
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.cancellation.child_token()
    }
}

/// Retains the original execution, circuit and flow deadlines through queued
/// I/O. The carrier is scheduling metadata, never an authorization claim.
pub struct PendingFrame {
    bytes: Vec<u8>,
    next: NodeId,
    carrier: Carrier,
    circuit: [u8; 32],
    guard: Guard,
}
impl fmt::Debug for PendingFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingFrame")
            .field("next", &self.next)
            .field("length", &self.bytes.len())
            .finish_non_exhaustive()
    }
}
impl PendingFrame {
    pub(crate) fn io_guard(&self) -> Guard {
        self.guard.clone()
    }
    pub fn next(&self) -> NodeId {
        self.next
    }
    pub fn carrier(&self) -> &Carrier {
        &self.carrier
    }
    pub fn circuit(&self) -> [u8; 32] {
        self.circuit
    }
    pub fn ensure_current(&self) -> Result<(), Error> {
        self.guard.ensure_current()
    }
    pub async fn invalidated(&self) {
        self.guard.invalidated().await;
    }
    /// Invoke the final synchronous sink, not an asynchronous queue.
    pub fn release<T>(self, sink: impl FnOnce(Vec<u8>) -> T) -> Result<T, Error> {
        self.ensure_current()?;
        Ok(sink(self.bytes))
    }
}

#[derive(Debug)]
pub enum Received {
    Forward(PendingFrame),
    Established {
        circuit: [u8; 32],
        response: Option<PendingFrame>,
    },
    Packet(PendingAction),
}

/// Unauthenticated scheduling hints decoded by the bounded candidate codec.
/// These fields grant no identity, path, carrier or execution authority. The
/// selected owner must still call `receive` with the actual incoming carrier.
#[derive(Clone, Debug)]
pub struct UntrustedIncomingHint {
    pub demand: RequestId,
    pub path: Vec<NodeId>,
    pub last_carrier: Carrier,
}

pub fn incoming_hint(bytes: &[u8]) -> Result<UntrustedIncomingHint, Error> {
    let frame = Frame::decode(bytes)?;
    Ok(UntrustedIncomingHint {
        demand: frame.context.demand,
        path: frame.context.path,
        last_carrier: frame.proofs.last().ok_or(Error::Wire)?.carrier.clone(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CircuitRole {
    Origin,
    Relay,
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CircuitState {
    Handshaking,
    Established,
    Closed,
}

/// Read-only health metadata; no key, cipher state, packet or grant is exposed.
#[derive(Clone, Debug)]
pub struct CircuitInfo {
    pub id: [u8; 32],
    pub demand: RequestId,
    pub path: Vec<NodeId>,
    pub role: CircuitRole,
    pub state: CircuitState,
    pub expires_at: Instant,
    pub expires_at_ms: u64,
}

enum State {
    Initiator(Box<snow::HandshakeState>),
    RelayAwaiting,
    RelayEstablished,
    Endpoint(Box<snow::StatelessTransportState>),
    Closed,
}
impl State {
    fn established(&self) -> bool {
        matches!(self, Self::RelayEstablished | Self::Endpoint(_))
    }
}

struct Circuit {
    context: Context,
    state: State,
    expires_at: Instant,
    expires_at_ms: u64,
    handshake_at: Instant,
    handshake_at_ms: u64,
    permit_expires_at_ms: u64,
    send: [SendNonce; 2],
    receive: [ReplayWindow; 2],
    cancellation: CancellationToken,
}
impl Drop for Circuit {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}
impl Circuit {
    fn new(
        context: Context,
        state: State,
        snapshot: &ExecutionSnapshot,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<Self, Error> {
        let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
        let permit = cfg.permits.get(&context.demand).ok_or(Error::Context)?;
        let now_ms = snapshot.now_ms().map_err(|_| Error::Authority)?;
        let handshake_at = Instant::now().checked_add(timeout).ok_or(Error::Limits)?;
        let handshake_at_ms = now_ms
            .checked_add(u64::try_from(timeout.as_millis()).map_err(|_| Error::Limits)?)
            .ok_or(Error::Limits)?;
        Ok(Self {
            context,
            state,
            expires_at: snapshot.expires_at(),
            expires_at_ms: snapshot.expires_at_ms().min(permit.expires_at_ms),
            handshake_at,
            handshake_at_ms,
            permit_expires_at_ms: permit.expires_at_ms,
            send: Default::default(),
            receive: Default::default(),
            cancellation,
        })
    }
    fn guard(&self, snapshot: &ExecutionSnapshot, handshake: bool) -> Guard {
        Guard {
            snapshot: snapshot.clone(),
            cancellation: self.cancellation.clone(),
            expires_at: if handshake {
                self.expires_at.min(self.handshake_at)
            } else {
                self.expires_at
            },
            expires_at_ms: if handshake {
                self.expires_at_ms.min(self.handshake_at_ms)
            } else {
                self.expires_at_ms
            },
        }
    }
    fn ensure_current(&self, snapshot: &ExecutionSnapshot) -> Result<(), Error> {
        if matches!(self.state, State::Closed) {
            return Err(Error::State);
        }
        self.guard(snapshot, !self.state.established())
            .ensure_current()
    }
    fn retire(&mut self) {
        self.cancellation.cancel();
        self.state = State::Closed;
    }

    // Moving the Noise state out is not a lifecycle transition. Only a failed
    // completion retires the circuit; success keeps every issued guard live.
    fn complete_handshake(
        &mut self,
        finish: impl FnOnce(Box<snow::HandshakeState>) -> Result<snow::StatelessTransportState, Error>,
    ) -> Result<(), Error> {
        if self.cancellation.is_cancelled() {
            self.retire();
            return Err(Error::Expired);
        }
        let State::Initiator(noise) = std::mem::replace(&mut self.state, State::Closed) else {
            self.retire();
            return Err(Error::State);
        };
        match finish(noise) {
            Ok(noise) => {
                if self.cancellation.is_cancelled() {
                    self.retire();
                    return Err(Error::Expired);
                }
                self.state = State::Endpoint(Box::new(noise));
                Ok(())
            }
            Err(error) => {
                self.retire();
                Err(error)
            }
        }
    }
}

/// One execution identity, one shared flow table, and a bounded circuit table.
/// Failed/closed circuits retain bounded tombstones for this owner's lifetime.
/// These count towards capacity; future permit extensions cannot make an old
/// handshake new again. Replacing the owner establishes a new replay domain.
pub struct SecureNode {
    identity: DataPlaneIdentity,
    engine: Engine,
    limits: Limits,
    carriers: BTreeMap<NodeId, Carrier>,
    circuits: BTreeMap<[u8; 32], Circuit>,
    cancellation: CancellationToken,
}
impl Drop for SecureNode {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}
impl fmt::Debug for SecureNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecureNode")
            .field("retained_circuits", &self.circuits.len())
            .finish_non_exhaustive()
    }
}
impl SecureNode {
    /// Replace host-observed authenticated carrier availability without changing
    /// circuit IDs, replay windows, flows or their complete logical paths.
    /// Every later output still checks this carrier against its specific permit.
    pub fn set_carriers(&mut self, carriers: BTreeMap<NodeId, Carrier>) -> Result<(), Error> {
        let snapshot = self.identity.snapshot();
        let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
        let now = snapshot.now_ms().map_err(|_| Error::Authority)?;
        for (peer, carrier) in &carriers {
            if carrier.listener_key.is_empty() || carrier.listener_key.len() > 255 {
                return Err(Error::Unsupported);
            }
            meshway_policy::admission::admit_adjacency(cfg, *peer, carrier, now)
                .map_err(|_| Error::Policy)?;
        }
        snapshot.ensure_current().map_err(|_| Error::Authority)?;
        self.carriers = carriers;
        Ok(())
    }

    pub fn new(
        identity: DataPlaneIdentity,
        limits: Limits,
        carriers: BTreeMap<NodeId, Carrier>,
    ) -> Result<Self, Error> {
        if limits.max_circuits == 0
            || limits.max_circuits > 65_536
            || limits.handshake_timeout < Duration::from_millis(1)
            || limits.handshake_timeout > Duration::from_secs(60)
        {
            return Err(Error::Limits);
        }
        if carriers
            .values()
            .any(|c| c.listener_key.is_empty() || c.listener_key.len() > 255)
        {
            return Err(Error::Unsupported);
        }
        identity
            .snapshot()
            .ensure_current()
            .map_err(|_| Error::Authority)?;
        let engine =
            Engine::new(identity.snapshot().clone(), limits.flows).map_err(
                |error| match error {
                    crate::Error::Limits => Error::Limits,
                    _ => Error::Authority,
                },
            )?;
        Ok(Self {
            identity,
            engine,
            limits,
            carriers,
            circuits: BTreeMap::new(),
            cancellation: CancellationToken::new(),
        })
    }

    pub fn start(&mut self, demand: RequestId, path: Vec<NodeId>) -> Result<PendingFrame, Error> {
        self.maintain()?;
        self.capacity()?;
        let snapshot = self.identity.snapshot();
        let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
        if path.first() != Some(&cfg.node_id) {
            return Err(Error::Context);
        }
        let hop = admit_handshake(
            cfg,
            demand,
            &path,
            cfg.node_id,
            None,
            false,
            snapshot.now_ms().map_err(|_| Error::Authority)?,
        )
        .map_err(|_| Error::Policy)?;
        let mut seed = Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut seed[..]).map_err(|_| Error::Crypto)?;
        let circuit_id = *seed;
        if self.circuits.contains_key(&circuit_id) {
            return Err(Error::State);
        }
        let permit = meshway_policy::permit_authorization_digest(
            cfg.permits.get(&demand).ok_or(Error::Context)?,
        )
        .map_err(|_| Error::Context)?;
        let context = Context {
            network: cfg.network_id,
            demand,
            keys: crypto::key_digest(cfg, &path)?,
            path,
            circuit: circuit_id,
            permit,
        };
        let mut noise = crypto::build(&self.identity, &context, true)?;
        let body = crypto::write_handshake(&mut noise, 96)?;
        snapshot.ensure_current().map_err(|_| Error::Authority)?;
        let circuit = Circuit::new(
            context.clone(),
            State::Initiator(Box::new(noise)),
            snapshot,
            self.limits.handshake_timeout,
            self.cancellation.child_token(),
        )?;
        let frame = Frame {
            context,
            kind: Kind::Init,
            reply: false,
            sequence: 0,
            facts: None,
            body,
            proofs: Vec::new(),
        };
        let pending = self.output(&circuit, frame, hop, None)?;
        pending.ensure_current()?;
        self.circuits.insert(circuit_id, circuit);
        Ok(pending)
    }

    pub fn receive(&mut self, incoming: &Carrier, bytes: Vec<u8>) -> Result<Received, Error> {
        self.maintain()?;
        let frame = Frame::decode(&bytes)?;
        let snapshot = self.identity.snapshot();
        let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
        let previous = crypto::verify(cfg, &frame, incoming)?;
        snapshot.ensure_current().map_err(|_| Error::Authority)?;
        let id = frame.context.circuit;
        if let Some(mut circuit) = self.circuits.remove(&id) {
            // Always restore retained state on rejection. A consumed nonce or
            // handshake must never roll back as a side effect of an output error.
            let result = self.receive_known(&mut circuit, frame, previous);
            if circuit.ensure_current(self.identity.snapshot()).is_err() {
                circuit.retire();
            }
            self.circuits.insert(id, circuit);
            result
        } else {
            if frame.kind != Kind::Init {
                return Err(Error::State);
            }
            self.receive_init(frame, previous)
        }
    }

    fn receive_init(&mut self, frame: Frame, previous: NodeId) -> Result<Received, Error> {
        self.capacity()?;
        let snapshot = self.identity.snapshot();
        let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
        let permit = meshway_policy::permit_authorization_digest(
            cfg.permits
                .get(&frame.context.demand)
                .ok_or(Error::Context)?,
        )
        .map_err(|_| Error::Context)?;
        if permit != frame.context.permit
            || crypto::key_digest(cfg, &frame.context.path)? != frame.context.keys
        {
            return Err(Error::Context);
        }
        let hop = self.handshake_hop(&frame, Some(previous))?;
        let mut circuit = Circuit::new(
            frame.context.clone(),
            State::RelayAwaiting,
            snapshot,
            self.limits.handshake_timeout,
            self.cancellation.child_token(),
        )?;
        let id = circuit.context.circuit;
        let result = match hop {
            Hop::Forward(_) => Received::Forward(self.output(&circuit, frame, hop, None)?),
            Hop::Deliver | Hop::Exit => {
                let mut noise = crypto::build(&self.identity, &frame.context, false)?;
                let expected = crypto::keys(cfg, frame.context.path[0])?.1.public_key.bytes;
                crypto::read_handshake(&mut noise, &frame.body, 96, expected.as_bytes())?;
                let body = crypto::write_handshake(&mut noise, 48)?;
                let response = Frame {
                    context: frame.context,
                    kind: Kind::Response,
                    reply: true,
                    sequence: 0,
                    facts: None,
                    body,
                    proofs: Vec::new(),
                };
                let hop = self.handshake_hop(&response, None)?;
                circuit.state = State::Endpoint(Box::new(
                    noise
                        .into_stateless_transport_mode()
                        .map_err(|_| Error::Crypto)?,
                ));
                let response = self.output(&circuit, response, hop, None)?;
                Received::Established {
                    circuit: id,
                    response: Some(response),
                }
            }
        };
        circuit.guard(snapshot, true).ensure_current()?;
        self.circuits.insert(id, circuit);
        Ok(result)
    }

    fn receive_known(
        &mut self,
        circuit: &mut Circuit,
        frame: Frame,
        previous: NodeId,
    ) -> Result<Received, Error> {
        let snapshot = self.identity.snapshot();
        circuit.ensure_current(snapshot)?;
        if circuit.context != frame.context {
            return Err(Error::Context);
        }
        match frame.kind {
            Kind::Init => Err(Error::Replay),
            Kind::Response => {
                let hop = self.handshake_hop(&frame, Some(previous))?;
                match &circuit.state {
                    State::RelayAwaiting => {
                        let output = self.output(circuit, frame, hop, None)?;
                        circuit.guard(snapshot, true).ensure_current()?;
                        circuit.state = State::RelayEstablished;
                        Ok(Received::Forward(output))
                    }
                    State::Initiator(_) => {
                        if hop != Hop::Deliver {
                            return Err(Error::State);
                        }
                        // Noise handshake reads mutate their state even when
                        // failing. Retire this circuit on any such failure.
                        let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
                        let expected =
                            crypto::keys(cfg, *circuit.context.path.last().ok_or(Error::Context)?)?
                                .1
                                .public_key
                                .bytes;
                        let guard = circuit.guard(snapshot, true);
                        circuit.complete_handshake(|mut noise| {
                            crypto::read_handshake(
                                &mut noise,
                                &frame.body,
                                48,
                                expected.as_bytes(),
                            )?;
                            guard.ensure_current()?;
                            let noise = noise
                                .into_stateless_transport_mode()
                                .map_err(|_| Error::Crypto)?;
                            guard.ensure_current()?;
                            Ok(noise)
                        })?;
                        Ok(Received::Established {
                            circuit: circuit.context.circuit,
                            response: None,
                        })
                    }
                    _ => Err(Error::Replay),
                }
            }
            Kind::Data => self.receive_data(circuit, frame, previous),
        }
    }

    fn receive_data(
        &mut self,
        circuit: &mut Circuit,
        frame: Frame,
        previous: NodeId,
    ) -> Result<Received, Error> {
        if !circuit.state.established() {
            return Err(Error::State);
        }
        let index = usize::from(frame.reply);
        circuit.receive[index].check(frame.sequence)?;
        let facts = frame.facts.ok_or(Error::Wire)?;
        let path = &circuit.context.path;
        let sender = if frame.reply {
            *path.last().ok_or(Error::Context)?
        } else {
            path[0]
        };
        // The verified direction fixes this authenticated producer to one of
        // two distinct endpoints (the codec rejects repeated path nodes).
        // FlowTable::prepare checks that producer against its state-derived
        // direction before commit. A signed wrong-direction packet therefore
        // cannot advance a flow and then fail the defensive direction check.
        let ingress = Ingress::Peer {
            demand: circuit.context.demand,
            path: path.clone(),
            identity: AuthenticatedPeer {
                origin: path[0],
                sender,
                previous,
            },
        };
        self.check_outgoing_carrier(&circuit.context, frame.reply)?;
        let (mut route, packet) = match &circuit.state {
            State::Endpoint(noise) => {
                let packet = crypto::decrypt(noise, frame.sequence, &frame.body, facts)?;
                circuit.ensure_current(self.identity.snapshot())?;
                let route = self
                    .engine
                    .admit_facts(ingress, packet.facts())
                    .map_err(|_| Error::Packet)?;
                (route, Some(packet))
            }
            State::RelayEstablished => (
                self.engine
                    .admit_facts(ingress, facts)
                    .map_err(|_| Error::Packet)?,
                None,
            ),
            _ => return Err(Error::State),
        };
        // Plaintext output must retain the circuit's original caps too, even
        // when its Engine has received a newer same-execution lease snapshot.
        route
            .constrain(
                circuit.expires_at,
                circuit.expires_at_ms,
                circuit.cancellation.clone(),
            )
            .map_err(|_| Error::Authority)?;
        if (route.route().direction == Direction::Reply) != frame.reply {
            return Err(Error::Context);
        }
        let sequence = frame.sequence;
        let result = match (route.hop(), packet) {
            (hop @ Hop::Forward(_), None) => {
                Received::Forward(self.output(circuit, frame, hop, Some(&route))?)
            }
            (Hop::Deliver | Hop::Exit, Some(packet)) => {
                circuit.ensure_current(self.identity.snapshot())?;
                Received::Packet(route.attach(packet).map_err(|_| Error::Packet)?)
            }
            _ => return Err(Error::State),
        };
        circuit.ensure_current(self.identity.snapshot())?;
        circuit.receive[index].commit(sequence);
        Ok(result)
    }

    pub fn send(&mut self, id: [u8; 32], packet: Packet) -> Result<PendingFrame, Error> {
        self.maintain()?;
        let mut circuit = self.circuits.remove(&id).ok_or(Error::State)?;
        let result = self.send_inner(&mut circuit, packet);
        if circuit.ensure_current(self.identity.snapshot()).is_err() {
            circuit.retire();
        }
        self.circuits.insert(id, circuit);
        result
    }

    fn send_inner(&mut self, circuit: &mut Circuit, packet: Packet) -> Result<PendingFrame, Error> {
        circuit.ensure_current(self.identity.snapshot())?;
        let State::Endpoint(noise) = &circuit.state else {
            return Err(Error::State);
        };
        if packet.as_bytes().len() > MAX_PLAINTEXT {
            return Err(Error::Unsupported);
        }
        let facts = packet.facts();
        let local = self
            .identity
            .snapshot()
            .configuration()
            .map_err(|_| Error::Authority)?
            .node_id;
        let reply = local != circuit.context.path[0];
        self.check_outgoing_carrier(&circuit.context, reply)?;
        let ingress = Ingress::Local {
            demand: circuit.context.demand,
            path: circuit.context.path.clone(),
        };
        let route = self
            .engine
            .admit_facts(ingress, facts)
            .map_err(|_| Error::Packet)?;
        if (route.route().direction == Direction::Reply) != reply {
            return Err(Error::Context);
        }
        let sequence = circuit.send[usize::from(reply)].take()?;
        let body = crypto::encrypt(noise, sequence, packet)?;
        circuit.ensure_current(self.identity.snapshot())?;
        let frame = Frame {
            context: circuit.context.clone(),
            kind: Kind::Data,
            reply,
            sequence,
            facts: Some(facts),
            body,
            proofs: Vec::new(),
        };
        self.output(circuit, frame, route.hop(), Some(&route))
    }

    fn handshake_hop(&self, frame: &Frame, previous: Option<NodeId>) -> Result<Hop, Error> {
        let snapshot = self.identity.snapshot();
        admit_handshake(
            snapshot.configuration().map_err(|_| Error::Authority)?,
            frame.context.demand,
            &frame.context.path,
            frame.context.path[0],
            previous,
            frame.reply,
            snapshot.now_ms().map_err(|_| Error::Authority)?,
        )
        .map_err(|_| Error::Policy)
    }

    // Check scheduling failures before stateful flow admission. This derives
    // only adjacency; policy remains the sole carrier authorization owner.
    fn check_outgoing_carrier(&self, context: &Context, reply: bool) -> Result<(), Error> {
        let cfg = self
            .identity
            .snapshot()
            .configuration()
            .map_err(|_| Error::Authority)?;
        let position = context
            .path
            .iter()
            .position(|node| *node == cfg.node_id)
            .ok_or(Error::Context)?;
        let next = if reply {
            position.checked_sub(1).and_then(|i| context.path.get(i))
        } else {
            context.path.get(position + 1)
        };
        if let Some(next) = next {
            let carrier = self.carriers.get(next).ok_or(Error::Unsupported)?;
            admit_carrier(cfg, context.demand, &context.path, *next, carrier)
                .map_err(|_| Error::Policy)?;
        }
        Ok(())
    }

    fn output(
        &self,
        circuit: &Circuit,
        mut frame: Frame,
        hop: Hop,
        route: Option<&PendingRoute>,
    ) -> Result<PendingFrame, Error> {
        let Hop::Forward(next) = hop else {
            return Err(Error::State);
        };
        let snapshot = self.identity.snapshot();
        let carrier = self.carriers.get(&next).ok_or(Error::Unsupported)?.clone();
        admit_carrier(
            snapshot.configuration().map_err(|_| Error::Authority)?,
            frame.context.demand,
            &frame.context.path,
            next,
            &carrier,
        )
        .map_err(|_| Error::Policy)?;
        let mut guard = circuit.guard(snapshot, frame.kind != Kind::Data);
        if let Some(route) = route {
            route.ensure_current().map_err(|_| Error::Authority)?;
            guard.snapshot = route.snapshot().clone();
            guard.expires_at = guard.expires_at.min(route.expires_at());
            guard.expires_at_ms = guard.expires_at_ms.min(route.permit_expires_at_ms());
        }
        guard.ensure_current()?;
        crypto::append(&self.identity, &mut frame, carrier.clone())?;
        let bytes = frame.encode()?;
        guard.ensure_current()?;
        Ok(PendingFrame {
            bytes,
            next,
            carrier,
            circuit: circuit.context.circuit,
            guard,
        })
    }

    fn capacity(&self) -> Result<(), Error> {
        if self.circuits.len() >= self.limits.max_circuits {
            Err(Error::Capacity)
        } else {
            Ok(())
        }
    }

    /// Same-target renewal, using the shared reconciliation implementation.
    pub fn refresh(&mut self, identity: DataPlaneIdentity) -> Result<(), Error> {
        if !self.identity.snapshot().same_execution(identity.snapshot()) {
            return Err(Error::Context);
        }
        self.reconcile(identity)
    }

    /// Carry live compatible circuits through an owner-authenticated direct
    /// publication replacement. Noise state, nonce/replay windows and the one
    /// flow table are retained. Old issued outputs never receive the new guard.
    /// Expired/closed states remain closed, including when a permit is extended.
    pub fn reconcile(&mut self, identity: DataPlaneIdentity) -> Result<(), Error> {
        let snapshot = identity.snapshot();
        let old = snapshot
            .continuation_from(self.identity.snapshot())
            .map_err(|_| Error::Authority)?;
        let new = snapshot.configuration().map_err(|_| Error::Authority)?;
        let now_ms = snapshot.now_ms().map_err(|_| Error::Authority)?;
        let now = Instant::now();
        let flows = self
            .engine
            .reconciled_flows(snapshot)
            .map_err(|_| Error::Authority)?;
        let mut plan = Vec::with_capacity(self.circuits.len());
        for (id, circuit) in &self.circuits {
            let compatible = crate::flow::authorization_unchanged(
                old,
                new,
                circuit.context.demand,
                &circuit.context.path,
                now_ms,
            )
            .map_err(|_| Error::Context)?;
            let live = compatible
                && !matches!(circuit.state, State::Closed)
                && !circuit.cancellation.is_cancelled()
                && now < circuit.expires_at
                && now_ms < circuit.expires_at_ms
                && (circuit.state.established()
                    || (now < circuit.handshake_at && now_ms < circuit.handshake_at_ms));
            let expiry = new
                .permits
                .get(&circuit.context.demand)
                .map(|permit| permit.expires_at_ms);
            plan.push((*id, live, expiry));
        }
        let final_now_ms = snapshot.now_ms().map_err(|_| Error::Authority)?;
        snapshot
            .continuation_from(self.identity.snapshot())
            .map_err(|_| Error::Authority)?;
        for (id, live, expiry) in plan {
            let circuit = self.circuits.get_mut(&id).expect("planned circuit");
            if let Some(expiry) = expiry {
                circuit.permit_expires_at_ms = circuit.permit_expires_at_ms.max(expiry);
            }
            if live && Instant::now() < circuit.expires_at && final_now_ms < circuit.expires_at_ms {
                circuit.expires_at = snapshot.expires_at();
                circuit.expires_at_ms = snapshot.expires_at_ms().min(expiry.expect("live permit"));
            } else {
                circuit.retire();
            }
        }
        self.engine.flows = flows;
        self.engine.snapshot = snapshot.clone();
        self.identity = identity;
        Ok(())
    }

    /// Host-driven cleanup; expired states become bounded replay tombstones.
    pub fn maintain(&mut self) -> Result<(), Error> {
        if self.identity.snapshot().ensure_current().is_err() {
            return Err(Error::Authority);
        }
        self.identity
            .snapshot()
            .now_ms()
            .map_err(|_| Error::Authority)?;
        for circuit in self.circuits.values_mut() {
            if circuit.ensure_current(self.identity.snapshot()).is_err() {
                circuit.retire();
            }
        }
        // Keep bounded replay tombstones for this owner. A later authenticated
        // extension/regrant of the same authorization must not accept an old IK
        // initiation after its original absolute expiry. Capacity fails closed.
        if self.engine.maintain().is_err() {
            return Err(Error::Authority);
        }
        Ok(())
    }

    /// Cancel queued wire/plaintext outputs, retire circuit keys and retain the
    /// ID replay tombstone. Other circuits and Control's grant are unaffected.
    pub fn close(&mut self, id: [u8; 32]) {
        if let Some(circuit) = self.circuits.get_mut(&id) {
            circuit.retire();
        }
    }

    /// Reading metadata never refreshes a deadline or manufactures a sample.
    pub fn circuit_info(&self, id: [u8; 32]) -> Result<Option<CircuitInfo>, Error> {
        let snapshot = self.identity.snapshot();
        let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
        let Some(circuit) = self.circuits.get(&id) else {
            return Ok(None);
        };
        let live = circuit.ensure_current(snapshot).is_ok();
        snapshot.ensure_current().map_err(|_| Error::Authority)?;
        let guard = circuit.guard(snapshot, !circuit.state.established());
        Ok(Some(CircuitInfo {
            id,
            demand: circuit.context.demand,
            path: circuit.context.path.clone(),
            role: if circuit.context.path.first() == Some(&cfg.node_id) {
                CircuitRole::Origin
            } else if circuit.context.path.last() == Some(&cfg.node_id) {
                CircuitRole::Terminal
            } else {
                CircuitRole::Relay
            },
            state: if !live {
                CircuitState::Closed
            } else if circuit.state.established() {
                CircuitState::Established
            } else {
                CircuitState::Handshaking
            },
            expires_at: guard.expires_at,
            expires_at_ms: guard.expires_at_ms,
        }))
    }

    pub fn ready_circuits(&self) -> Result<Vec<CircuitInfo>, Error> {
        self.identity
            .snapshot()
            .ensure_current()
            .map_err(|_| Error::Authority)?;
        let mut ready = Vec::new();
        for id in self.circuits.keys() {
            if let Some(info) = self.circuit_info(*id)?
                && info.state == CircuitState::Established
            {
                ready.push(info);
            }
        }
        Ok(ready)
    }
    pub fn retained_circuits(&self) -> usize {
        self.circuits.len()
    }
    pub fn retained_flows(&self) -> usize {
        self.engine.retained_flows()
    }

    /// Read original demand/path metadata from the one Engine flow table. This
    /// neither selects nor opens a circuit and never grants packet authority.
    /// An existing route remains existing even if its carrier/circuit is down.
    pub fn local_flow(&self, packet: &Packet) -> Result<Option<crate::LocalFlow>, Error> {
        self.identity
            .snapshot()
            .ensure_current()
            .map_err(|_| Error::Authority)?;
        self.engine.local_flow(packet).map_err(|error| match error {
            crate::Error::Control(_) => Error::Authority,
            crate::Error::Flow => Error::Flow,
            crate::Error::Expired => Error::Expired,
            _ => Error::Packet,
        })
    }
    pub async fn invalidated(&mut self) {
        self.identity.snapshot().invalidated().await;
        for circuit in self.circuits.values_mut() {
            circuit.retire();
        }
        self.engine.flows.clear();
    }
}

#[cfg(test)]
mod tests;
