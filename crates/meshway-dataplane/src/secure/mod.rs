//! Versioned candidate Mesh circuits, not a stable wire ABI.
//!
//! Noise IK protects IP payloads end to end between the original source and the
//! terminal node. Relays see only circuit routing fields and ciphertext. Every forwarding node
//! verifies ordered Ed25519 proofs during ephemeral session establishment, then
//! directional adjacent MACs over frames on authenticated carriers.
//! Replies use that exact path in
//! reverse; carrier changes never authorize another logical path.
//!
//! This synchronous owner opens no sockets and creates no background tasks.
//! Hosts retain outputs with their lifetimes through I/O readiness and release
//! at their final synchronous sink. Closing a circuit cancels its queued
//! wire/plaintext outputs; applying new content retires exactly the circuits
//! and flows it no longer admits, and dropping the node cancels every circuit.
//! Steady-state packets carry a CID and a fixed-size hop MAC; no per-packet
//! public-key operation or unauthenticated context reconstruction is used.
mod authorization;
mod codec;
mod crypto;
mod diagnostics;
mod dns;
mod exit;
mod maintenance;
mod netmap;
pub mod pipeline;
mod probe;
pub use dns::{DnsReply, DnsRequest, MAX_DNS_BYTES};
mod recovery;
mod replay;
mod session;

use crate::{
    Direction, Engine, Hop, Packet, PacketBuffer, PendingAction, PendingRoute,
    authority::{Cap, Lifetime, View},
    flow::Entry,
};
pub use codec::is_control_frame;
use codec::{Context, Frame, Kind};
use meshway_authority::DataPlaneKeys;
use meshway_configuration::execution::LogicalPath;
use meshway_engine::Desired;
use meshway_model::NodeId;
pub use meshway_policy::admission::Carrier;
pub use meshway_policy::compile::netmap::Target;
pub use netmap::{AdjacencyInterest, Applied, Flushed, Outbound, diagnostic_paths};
use replay::{ReplayWindow, SendNonce};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub const MAX_FRAME: usize = 128 * 1024;
/// Noise's 65535-byte message limit minus its 16-byte authentication tag.
pub const MAX_PLAINTEXT: usize = 65_519;
/// Fixed steady-state Data frame overhead per IP packet: 54-byte header, 16-byte
/// AEAD tag and 16-byte adjacent MAC. Independent of path length and relays.
pub const DATA_FRAME_OVERHEAD: usize = codec::DATA_HEADER + 16 + 16;
/// Largest inner MTU that keeps one Data frame inside a 1500-byte underlay
/// link over IPv6/UDP (40 + 8 bytes); IPv4 underlays leave 20 more bytes.
pub const DEFAULT_INNER_MTU: usize = 1500 - 40 - 8 - DATA_FRAME_OVERHEAD;

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

/// Allowed difference between an origin's INIT time and a receiver's clock.
const INIT_SKEW: Duration = Duration::from_secs(2);
/// An established circuit without any frame for this long ends; below the
/// 120 s adjacency idle bound, so it never outlives its next hop silently.
const CIRCUIT_IDLE: Duration = Duration::from_secs(90);
/// An origin that sent for this long without one authenticated receive ends
/// the circuit and re-handshakes on its next packet (a restarted peer).
const NO_REPLY: Duration = Duration::from_secs(30);

/// The stable grant a circuit is admitted under: (origin = `path[0]`,
/// target, purpose) on the installed netmap; never installed content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CircuitGrant {
    pub purpose: crate::Purpose,
    pub target: Target,
}

impl CircuitGrant {
    fn diagnostic(&self) -> bool {
        self.purpose == crate::Purpose::Diagnostic
    }

    fn flow(&self) -> Option<Target> {
        (self.purpose == crate::Purpose::Business).then(|| self.target.clone())
    }
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
    pub(crate) life: Lifetime,
    projection: Option<Arc<crate::exit::Gate>>,
}
impl Guard {
    pub(crate) fn for_projection(&self, cap: Cap, cancellation: CancellationToken) -> Self {
        let mut guard = self.clone();
        guard.life.restrict(Some(cap));
        guard.life.flow = Some(cancellation);
        guard.projection = None;
        guard
    }
    pub(crate) fn from_route(route: &PendingRoute) -> Self {
        Self {
            life: route.life.clone(),
            projection: route.projection.clone(),
        }
    }
    /// The own bound of this output, if any.
    pub(crate) fn expiry(&self) -> Option<Cap> {
        self.life.cap
    }

    /// One clock reading for a whole batch of outputs.
    pub(crate) fn check_batch(guards: &[Self]) -> Result<(), Error> {
        let now = (Instant::now(), crate::authority::now_ms());
        for guard in guards {
            guard.check_now(now)?;
            if let Some(gate) = &guard.projection {
                gate.ready().map_err(|_| Error::Expired)?;
            }
        }
        Ok(())
    }
    fn check_now(&self, (instant, now_ms): (Instant, u64)) -> Result<(), Error> {
        if !self.life.live_at(instant, now_ms) {
            return Err(Error::Expired);
        }
        Ok(())
    }
    pub(crate) fn check_at(&self, reading: &View<'_>) -> Result<(), Error> {
        self.check_now(reading.now())
    }

    // A fresh check immediately before a synchronous commit or output.
    fn finish(&self) -> Result<(), Error> {
        self.check_now((Instant::now(), crate::authority::now_ms()))
    }

    pub(crate) fn ensure_current(&self) -> Result<(), Error> {
        self.ensure_before(u64::MAX)
    }

    pub(crate) fn ensure_before(&self, deadline_ms: u64) -> Result<(), Error> {
        if self.life.cancellation.is_cancelled() {
            return Err(Error::Expired);
        }
        let now = (Instant::now(), crate::authority::now_ms());
        self.check_now(now)?;
        if now.1 >= deadline_ms {
            return Err(Error::Expired);
        }
        if let Some(gate) = &self.projection {
            gate.ready().map_err(|_| Error::Expired)?;
        }
        Ok(())
    }
    pub(crate) async fn ended(&self) {
        if self.ensure_current().is_err() {
            return;
        }
        self.life.ended().await;
    }
    pub(crate) fn expires_at(&self) -> Option<Instant> {
        self.expiry().map(|cap| cap.at)
    }
    /// An I/O-local child: cancelling one write must not close its circuit.
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.life.cancellation.child_token()
    }
}

/// Retains the original execution, circuit and flow deadlines through queued
/// I/O. The carrier is scheduling metadata, never an authorization claim.
pub struct PendingFrame {
    bytes: PacketBuffer,
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
    pub(crate) fn capacity(&self) -> usize {
        self.bytes.capacity()
    }
    pub(crate) fn release_batch(frames: Vec<Self>) -> Result<Vec<PacketBuffer>, Error> {
        let guards: Vec<_> = frames.iter().map(Self::io_guard).collect();
        Guard::check_batch(&guards)?;
        Ok(frames.into_iter().map(|frame| frame.bytes).collect())
    }
    pub fn is_data(&self) -> bool {
        self.bytes.get(8) == Some(&3)
    }
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
    pub async fn ended(&self) {
        self.guard.ended().await;
    }
    /// A reply which refreshes an exit projection retains its cipher/flow guard
    /// while the host completes the corresponding Renew operation.
    pub async fn projection_ready(&self) -> Result<(), Error> {
        self.guard.finish()?;
        if let Some(gate) = &self.guard.projection {
            gate.wait().await.map_err(|_| Error::Expired)?;
        }
        self.ensure_current()
    }
    /// Invoke the final synchronous sink, not an asynchronous queue.
    pub fn release<T>(self, sink: impl FnOnce(PacketBuffer) -> T) -> Result<T, Error> {
        self.ensure_current()?;
        Ok(sink(self.bytes))
    }
}

#[derive(Debug)]
pub enum Received {
    DnsRequest(DnsRequest),
    DnsReply(DnsReply),
    Forward(PendingFrame),
    /// Also returned for an exact, still-authorized duplicate handshake receipt.
    /// That idempotent result does not imply a new Noise session or RTT sample.
    Established {
        circuit: [u8; 32],
        response: Option<PendingFrame>,
    },
    Packet(PendingAction),
    ProbeRequest {
        circuit: [u8; 32],
        measurement_id: meshway_model::EventId,
        response: PendingFrame,
    },
    ProbeReply {
        circuit: [u8; 32],
        measurement_id: meshway_model::EventId,
        challenge: meshway_model::Digest,
    },
}

/// Unauthenticated scheduling hints decoded by the bounded candidate codec.
/// These fields grant no identity, path, carrier or execution authority. The
/// selected owner must still call `receive` with the actual incoming carrier.
#[derive(Clone, Debug)]
pub struct UntrustedIncomingHint {
    pub grant: CircuitGrant,
    pub path: Vec<NodeId>,
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
    pub grant: CircuitGrant,
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
    Endpoint(Arc<crypto::Transport>),
    Closed,
}
impl State {
    fn established(&self) -> bool {
        matches!(self, Self::RelayEstablished | Self::Endpoint(_))
    }
}

struct Circuit {
    context: Arc<Context>,
    state: State,
    /// Own bound beyond activity, if any.
    cap: Option<Cap>,
    handshake: Cap,
    /// Last authenticated frame received or sent on this circuit.
    active: Instant,
    /// The oldest send not followed by any authenticated receive.
    unanswered: Option<Instant>,
    /// This Node is the circuit's origin (`path[0]`).
    origin: bool,
    /// The exact path this owner admitted, with its dial candidates.
    selected: Option<Arc<LogicalPath>>,
    send: [SendNonce; 2],
    receive: [ReplayWindow; 2],
    cancellation: CancellationToken,
    recovery: recovery::Recovery,
    session: session::Session,
}
impl Drop for Circuit {
    fn drop(&mut self) {
        if !matches!(self.state, State::Closed) && diagnostics::event() {
            tracing::debug!(operation = "drop", cid = ?self.context.circuit,
                grant = ?self.context.grant, established = self.state.established(),
                "Dataplane circuit dropped");
        }
        self.cancellation.cancel();
    }
}
impl Circuit {
    fn new(
        context: Arc<Context>,
        state: State,
        reading: &View<'_>,
        timeout: Duration,
        cancellation: CancellationToken,
        selected: Option<Arc<LogicalPath>>,
    ) -> Result<Self, Error> {
        let cap = None;
        let mut handshake =
            Cap::after(reading.instant, reading.now_ms, timeout).ok_or(Error::Limits)?;
        // The INIT window also bounds a responder's handshake state.
        let window = context.initiated_at_ms.saturating_add(
            u64::try_from((timeout + INIT_SKEW).as_millis()).map_err(|_| Error::Limits)?,
        );
        handshake = handshake
            .min(Cap::until_ms(reading.instant, reading.now_ms, window).ok_or(Error::Limits)?);
        let origin = context.path.first() == Some(&reading.node());
        Ok(Self {
            context,
            state,
            cap,
            handshake,
            active: reading.instant,
            unanswered: None,
            origin,
            selected,
            send: Default::default(),
            receive: Default::default(),
            cancellation,
            recovery: Default::default(),
            session: session::Session::new()?,
        })
    }
    fn guard(&self, handshake: bool) -> Guard {
        let mut life = Lifetime::new(self.cap, self.cancellation.clone());
        if handshake {
            life.restrict(Some(self.handshake));
        }
        Guard {
            life,
            projection: None,
        }
    }
    fn ensure_current(&self) -> Result<(), Error> {
        self.check_now(Instant::now(), crate::authority::now_ms())
    }

    fn check_at(&self, reading: &View<'_>) -> Result<(), Error> {
        self.check_now(reading.instant, reading.now_ms)
    }

    fn check_now(&self, instant: Instant, now_ms: u64) -> Result<(), Error> {
        if matches!(self.state, State::Closed) {
            return Err(Error::State);
        }
        if self.cancellation.is_cancelled()
            || self.cap.is_some_and(|cap| !cap.live_at(instant, now_ms))
            || (!self.state.established() && !self.handshake.live_at(instant, now_ms))
            || self
                .activity_deadline()
                .is_some_and(|deadline| instant >= deadline)
        {
            return Err(Error::Expired);
        }
        Ok(())
    }

    /// An established circuit ends after [`CIRCUIT_IDLE`] without any frame
    /// in either direction (its adjacency idles out too), and its origin
    /// ends it after [`NO_REPLY`] of sending without a single authenticated
    /// receive (a restarted peer drops its frames). The origin's next packet
    /// then acquires a fresh adjacency and handshake.
    fn activity_deadline(&self) -> Option<Instant> {
        if !self.state.established() {
            return None;
        }
        let idle = self.active + CIRCUIT_IDLE;
        Some(match self.unanswered {
            Some(sent) if matches!(self.state, State::Endpoint(_)) && self.origin() => {
                idle.min(sent + NO_REPLY)
            }
            _ => idle,
        })
    }

    fn origin(&self) -> bool {
        self.origin
    }

    fn received(&mut self, at: Instant) {
        self.active = self.active.max(at);
        self.unanswered = None;
    }

    fn sent(&mut self, at: Instant) {
        self.active = self.active.max(at);
        self.unanswered.get_or_insert(at);
    }

    /// Bound of this circuit for scheduling.
    fn next_deadline(&self) -> Option<Cap> {
        let handshake = (!self.state.established()).then_some(self.handshake);
        let activity = self.activity_deadline().and_then(|at| {
            let now = Instant::now();
            Cap::after(
                now,
                crate::authority::now_ms(),
                at.saturating_duration_since(now),
            )
        });
        crate::authority::restrict(crate::authority::restrict(self.cap, handshake), activity)
    }

    /// Terminal state, cancellation and observed circuit expiry are terminal.
    fn validate_operation(&mut self, operation: &'static str) -> Result<(), Error> {
        let result = self.ensure_current();
        if let Err(error) = result {
            self.retire_for(
                operation,
                match error {
                    Error::Expired => "deadline_or_cancel",
                    _ => "state",
                },
            );
        }
        result
    }

    fn finish_operation<T>(
        &mut self,
        operation: &'static str,
        result: Result<T, Error>,
    ) -> Result<T, Error> {
        let current = self.validate_operation(operation);
        // Preserve the original rejection. A produced output must also pass the
        // final check; dropping it never rolls back consumed nonces/flow state.
        result.and_then(|output| current.map(|()| output))
    }
    fn retire(&mut self) {
        self.retire_for("local", "explicit_close");
    }

    fn retire_for(&mut self, operation: &'static str, cause: &'static str) {
        if !matches!(self.state, State::Closed) && diagnostics::event() {
            tracing::debug!(operation, cause, cid = ?self.context.circuit,
                grant = ?self.context.grant, established = self.state.established(),
                cancelled = self.cancellation.is_cancelled(),
                expires_at_ms = self.cap.map(|cap| cap.ms),
                handshake_expires_at_ms = self.handshake.ms,
                "Dataplane circuit retired");
        }
        self.cancellation.cancel();
        self.state = State::Closed;
        self.recovery = Default::default();
        self.session = Default::default();
    }

    fn engine_rejected(&self, operation: &'static str, error: crate::Error) -> Error {
        if diagnostics::event() {
            let admission = match &error {
                crate::Error::Admission(value) => Some(*value),
                _ => None,
            };
            tracing::debug!(operation, cause = diagnostics::engine_error(&error), ?admission,
                cid = ?self.context.circuit, grant = ?self.context.grant,
                "Dataplane engine admission rejected");
        }
        Error::Packet
    }

    // Moving the Noise state out is not a lifecycle transition. Only a failed
    // completion retires the circuit; success keeps every issued guard live.
    fn complete_handshake(
        &mut self,
        finish: impl FnOnce(Box<snow::HandshakeState>) -> Result<crypto::Transport, Error>,
    ) -> Result<(), Error> {
        if self.cancellation.is_cancelled() {
            self.retire_for("handshake", "cancelled");
            return Err(Error::Expired);
        }
        let State::Initiator(noise) = std::mem::replace(&mut self.state, State::Closed) else {
            self.retire_for("handshake", "invalid_state");
            return Err(Error::State);
        };
        match finish(noise) {
            Ok(noise) => {
                if self.cancellation.is_cancelled() {
                    self.retire_for("handshake", "cancelled");
                    return Err(Error::Expired);
                }
                self.state = State::Endpoint(Arc::new(noise));
                Ok(())
            }
            Err(error) => {
                self.retire_for("handshake", "completion_failed");
                Err(error)
            }
        }
    }
}

/// One desired content and its packet keys, one shared flow table, and a
/// bounded circuit table. Closed circuits leave only a compact dead-CID record
/// until their INIT window has passed; any later replay of that INIT is
/// rejected as stale. Dead records do not count towards `max_circuits`.
pub struct SecureNode {
    desired: Desired,
    keys: DataPlaneKeys,
    engine: Engine,
    limits: Limits,
    carriers: BTreeMap<NodeId, Carrier>,
    // Stable allocation for the owner state: remove/restore coordinates borrows
    // without moving replay windows, recovery state and session metadata per packet.
    circuits: BTreeMap<[u8; 32], Box<Circuit>>,
    /// Closed CID → UTC milliseconds until which its INIT is still acceptable.
    dead: BTreeMap<[u8; 32], u64>,
    probes: probe::Probes,
    dns: dns::Exchanges,
    maintenance: maintenance::Maintenance,
    cancellation: CancellationToken,
    first: netmap::FirstPackets,
}

#[derive(Clone, Copy)]
struct OutputOwner<'a> {
    carriers: &'a BTreeMap<NodeId, Carrier>,
}
impl Drop for SecureNode {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}
impl fmt::Debug for SecureNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecureNode")
            .field("circuits", &self.circuits.len())
            .field("dead_circuits", &self.dead.len())
            .finish_non_exhaustive()
    }
}
impl SecureNode {
    /// A bounded scheduling lookup. Session metadata comes only from this owner;
    /// packet authentication and replay admission still happen in receive().
    pub fn incoming_hint(&self, bytes: &[u8]) -> Result<UntrustedIncomingHint, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("codec.hint");
        let _profile = PROFILE.enter();
        let session = codec::session_id(bytes)?;
        let context = if let Some(id) = session {
            let reading = View::of(&self.desired);
            let circuit = self.circuits.get(&id).ok_or(Error::State)?;
            circuit.check_at(&reading)?;
            circuit.context.clone()
        } else {
            // A full handshake hint is only an untrusted bounded decode;
            // receive admits it on the desired content before using anything.
            Frame::decode(bytes, None)?.context
        };
        Ok(UntrustedIncomingHint {
            grant: context.grant.clone(),
            path: context.path.clone(),
        })
    }

    /// Replace host-observed authenticated carrier availability without changing
    /// circuit IDs, replay windows, flows or their complete logical paths.
    /// Every carrier must be an admitted adjacency of the desired content;
    /// every later output still checks it against its circuit's exact path.
    pub fn set_carriers(&mut self, carriers: BTreeMap<NodeId, Carrier>) -> Result<(), Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.set_carriers");
        let _profile = PROFILE.enter();
        let desired = self.desired.clone();
        let reading = View::of(&desired);
        for (peer, carrier) in &carriers {
            if carrier.listener_key.is_empty() || carrier.listener_key.len() > 255 {
                return Err(Error::Unsupported);
            }
            if !authorization::adjacency(&mut self.engine.decisions, &reading, *peer, carrier) {
                return Err(Error::Policy);
            }
        }
        self.carriers = carriers;
        Ok(())
    }

    /// The packet owner of `desired`, with this Node's registered keys.
    pub fn new(
        desired: &Desired,
        keys: DataPlaneKeys,
        limits: Limits,
        carriers: BTreeMap<NodeId, Carrier>,
    ) -> Result<Self, Error> {
        if !keys.matches(&desired.local().self_keys) {
            return Err(Error::Authority);
        }
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
        let engine = Engine::new(desired, limits.flows).map_err(|_| Error::Limits)?;
        let mut node = Self {
            desired: desired.clone(),
            keys,
            engine,
            limits,
            carriers: BTreeMap::new(),
            circuits: BTreeMap::new(),
            dead: BTreeMap::new(),
            probes: Default::default(),
            dns: Default::default(),
            maintenance: Default::default(),
            cancellation: CancellationToken::new(),
            first: Default::default(),
        };
        node.set_carriers(carriers)?;
        Ok(node)
    }

    /// The desired content this owner admits on.
    pub fn desired(&self) -> &Desired {
        &self.desired
    }

    /// Open a circuit of a grant on the installed target. A netmap grant
    /// names (purpose, target); `path` must be one of the pair's paths.
    pub fn start_grant(
        &mut self,
        grant: CircuitGrant,
        path: Vec<NodeId>,
        carrier: &Carrier,
    ) -> Result<PendingFrame, Error> {
        let desired = self.desired.clone();
        let reading = View::of(&desired);
        self.start_at(grant, path, carrier, &reading)
    }

    fn start_at(
        &mut self,
        grant: CircuitGrant,
        path: Vec<NodeId>,
        carrier: &Carrier,
        reading: &View<'_>,
    ) -> Result<PendingFrame, Error> {
        self.maintain_circuits(reading, false)?;
        self.engine.maintain_at(reading);
        self.capacity()?;
        if path.first() != Some(&reading.node()) {
            return Err(Error::Context);
        }
        if self.carriers.get(path.get(1).ok_or(Error::Context)?) != Some(carrier) {
            return Err(Error::Context);
        }
        let admitted = authorization::admit(
            &mut self.engine.decisions,
            reading,
            &grant,
            &path,
            None,
            false,
        )?;
        let mut seed = Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut seed[..]).map_err(|_| Error::Crypto)?;
        let circuit_id = *seed;
        if self.circuits.contains_key(&circuit_id) || self.dead.contains_key(&circuit_id) {
            return Err(Error::State);
        }
        let context = Arc::new(Context {
            network: reading.network(),
            grant,
            keys: crypto::key_digest(reading, &path)?,
            path,
            circuit: circuit_id,
            authorization: admitted.authorization,
            initiated_at_ms: reading.now_ms,
        });
        let mut noise = crypto::build(&self.keys, reading, &context, true)?;
        let body = crypto::write_handshake(&mut noise, 96)?;
        let mut circuit = Circuit::new(
            context.clone(),
            State::Initiator(Box::new(noise)),
            reading,
            self.limits.handshake_timeout,
            self.cancellation.child_token(),
            Some(admitted.selected),
        )?;
        let frame = Frame {
            context,
            kind: Kind::Init,
            reply: false,
            sequence: 0,
            probe: None,
            body: body.into(),
            proofs: Vec::new(),
            exchange: None,
            auth: Default::default(),
        };
        circuit.session.remember_init(&frame)?;
        let pending = self.output(&circuit, frame, admitted.hop, None, reading)?;
        circuit.recovery.init = Some(recovery::Stage::new(
            Sha256::digest(&pending.bytes).into(),
            pending.guard.clone(),
            Some(&pending),
        )?);
        pending.guard.finish()?;
        self.circuits.insert(circuit_id, Box::new(circuit));
        self.maintenance.changed();
        Ok(pending)
    }

    /// Next paced retry of this origin's unchanged Noise INIT. No new CID,
    /// nonce, proof or authority is created. None means no retry is available;
    /// the host must still respect the circuit's original handshake deadline.
    pub fn handshake_retry_at(&self, id: [u8; 32]) -> Result<Option<Instant>, Error> {
        let desired = &self.desired;
        let reading = View::of(desired);
        let circuit = self.circuits.get(&id).ok_or(Error::State)?;
        circuit.check_at(&reading)?;
        if !matches!(circuit.state, State::Initiator(_)) {
            return Ok(None);
        }
        circuit
            .recovery
            .init
            .as_ref()
            .map_or(Ok(None), |stage| stage.retry_at())
    }

    /// Issue at most one original guarded INIT when due. Repeated polling never
    /// bypasses pacing/budget; old outputs remain invalid after scope handoff.
    pub fn retry_handshake(&mut self, id: [u8; 32]) -> Result<Option<PendingFrame>, Error> {
        let desired = self.desired.clone();
        let reading = View::of(&desired);
        self.maintain_circuits(&reading, false)?;
        let mut circuit = self.circuits.remove(&id).ok_or(Error::State)?;
        let result = (|| {
            circuit.check_at(&reading)?;
            if !matches!(circuit.state, State::Initiator(_)) {
                return Ok(None);
            }
            let Some(stage) = circuit.recovery.init.as_mut() else {
                return Ok(None);
            };
            let output = stage.retry()?;
            if let Some(output) = &output {
                Self::check_retry_carrier(
                    &self.carriers,
                    output,
                    &circuit.context,
                    circuit.selected.as_deref(),
                )?;
            }
            Ok(output)
        })();
        let result = circuit.finish_operation("retry_handshake", result);
        self.restore(id, circuit, &reading);
        result
    }

    /// Return an operated circuit to the live table, or keep only its compact
    /// dead-CID record once closed.
    fn restore(&mut self, id: [u8; 32], circuit: Box<Circuit>, reading: &View<'_>) {
        if matches!(circuit.state, State::Closed) {
            self.bury(id, &circuit, reading.now_ms);
        } else {
            self.circuits.insert(id, circuit);
        }
    }

    fn bury(&mut self, id: [u8; 32], circuit: &Circuit, now_ms: u64) {
        let until = circuit.context.initiated_at_ms.saturating_add(
            u64::try_from((self.limits.handshake_timeout + 2 * INIT_SKEW).as_millis())
                .unwrap_or(u64::MAX),
        );
        if until > now_ms {
            self.dead.insert(id, until);
        }
        self.maintenance.changed();
    }

    pub fn receive(
        &mut self,
        incoming: &Carrier,
        bytes: impl AsRef<[u8]> + Into<PacketBuffer>,
    ) -> Result<Received, Error> {
        if bytes.as_ref().get(8) == Some(&3) {
            let job = self.prepare_receive(incoming, bytes.into())?;
            return self.commit_receive(pipeline::open(job)?);
        }
        self.receive_control(incoming, bytes)
    }

    fn receive_control(
        &mut self,
        incoming: &Carrier,
        bytes: impl AsRef<[u8]> + Into<PacketBuffer>,
    ) -> Result<Received, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.receive");
        let _profile = PROFILE.enter();
        let desired = self.desired.clone();
        let reading = View::of(&desired);
        self.maintain_circuits(&reading, false)?;
        let id = codec::session_id(bytes.as_ref())?;
        let kept = id
            .as_ref()
            .and_then(|id| self.circuits.get(id))
            .map(|circuit| &circuit.context);
        let frame = Frame::decode_owned(bytes.into(), kept)?;
        let id = frame.context.circuit;
        if self.dead.contains_key(&id) {
            return Err(Error::Replay);
        }
        let verified = if frame.kind.handshake() {
            self.verify_handshake(&frame, incoming, &reading)
                .map(|(previous, admitted)| (previous, Some(admitted)))
        } else {
            let circuit = self.circuits.get_mut(&id).ok_or(Error::State)?;
            if let Err(error) = circuit.check_at(&reading) {
                circuit.retire_for("receive", "deadline_or_cancel");
                return Err(error);
            }
            if circuit.context != frame.context {
                return Err(Error::Context);
            }
            circuit.receive[usize::from(frame.reply)].check(frame.sequence)?;
            let selected = authorization::selected(&circuit.context, circuit.selected.as_deref())?;
            let previous = circuit.session.verify(selected, &frame, incoming)?;
            authorization::incoming(
                &mut self.engine.decisions,
                &reading,
                selected,
                previous,
                incoming,
            )
            .map(|()| (previous, None))
        }
        .inspect_err(|error| {
            if diagnostics::event() {
                tracing::debug!(operation = "verify", ?error,
                    cid = ?frame.context.circuit, grant = ?frame.context.grant,
                    metadata_authenticated = false, "Dataplane secure admission rejected");
            }
        })?;
        let (previous, admitted) = verified;
        if let Some(mut circuit) = self.circuits.remove(&id) {
            // Always restore kept state on rejection. A consumed nonce or
            // handshake must never roll back as a side effect of an output error.
            let result = self.receive_known(&mut circuit, frame, previous, &reading);
            if result.is_ok() {
                circuit.received(reading.instant);
            }
            let result = circuit.finish_operation("receive", result);
            if let Err(error) = &result
                && diagnostics::event()
            {
                tracing::debug!(operation = "receive", ?error, cid = ?id,
                    grant = ?circuit.context.grant, "Dataplane secure admission rejected");
            }
            self.restore(id, circuit, &reading);
            result
        } else {
            if frame.kind != Kind::Init {
                if diagnostics::event() {
                    tracing::debug!(operation = "receive", cause = "unknown_circuit", error = ?Error::State,
                        cid = ?id, grant = ?frame.context.grant,
                        "Dataplane secure admission rejected");
                }
                return Err(Error::State);
            }
            self.receive_init(frame, previous, admitted.ok_or(Error::State)?, &reading)
        }
    }

    /// Authenticate a handshake frame's proof chain on the path this owner
    /// admits for its grant, and the incoming carrier.
    fn verify_handshake(
        &mut self,
        frame: &Frame,
        incoming: &Carrier,
        reading: &View<'_>,
    ) -> Result<(NodeId, authorization::Admitted), Error> {
        // Path admission precedes signature work; the arrival direction is
        // checked after the chain names the authenticated previous hop.
        let origin = frame.context.path[0];
        let paths = authorization::paths(
            &mut self.engine.decisions,
            reading.netmap(),
            origin,
            &frame.context.grant,
        )?;
        let path = paths
            .iter()
            .find(|path| path.nodes == frame.context.path)
            .ok_or(Error::Policy)?
            .clone();
        let previous = crypto::verify(reading, frame, incoming, &path)?;
        authorization::incoming(
            &mut self.engine.decisions,
            reading,
            &path,
            previous,
            incoming,
        )?;
        let admitted = authorization::admit(
            &mut self.engine.decisions,
            reading,
            &frame.context.grant,
            &frame.context.path,
            Some(previous),
            frame.reply,
        )?;
        Ok((previous, admitted))
    }

    fn receive_init(
        &mut self,
        frame: Frame,
        _previous: NodeId,
        admitted: authorization::Admitted,
        reading: &View<'_>,
    ) -> Result<Received, Error> {
        self.capacity()?;
        let window = u64::try_from((self.limits.handshake_timeout + INIT_SKEW).as_millis())
            .map_err(|_| Error::Limits)?;
        let skew = u64::try_from(INIT_SKEW.as_millis()).map_err(|_| Error::Limits)?;
        if frame.context.initiated_at_ms.saturating_add(window) <= reading.now_ms
            || frame.context.initiated_at_ms > reading.now_ms.saturating_add(skew)
        {
            return Err(Error::Expired);
        }
        if admitted.authorization != frame.context.authorization
            || crypto::key_digest(reading, &frame.context.path)? != frame.context.keys
        {
            return Err(Error::Context);
        }
        let hop = admitted.hop;
        if let Hop::Forward(next) = hop
            && !self.carriers.contains_key(&next)
        {
            // A relay without a carrier to the next hop consumes nothing: it
            // asks for that adjacency, and the origin's paced retry of the
            // same INIT is admitted afresh once it exists.
            self.first.interest(next);
            return Err(Error::Unsupported);
        }
        let mut circuit = Circuit::new(
            frame.context.clone(),
            State::RelayAwaiting,
            reading,
            self.limits.handshake_timeout,
            self.cancellation.child_token(),
            Some(admitted.selected),
        )?;
        let id = circuit.context.circuit;
        let fingerprint = Sha256::digest(frame.encode()?).into();
        let result = (|| {
            circuit.session.remember_init(&frame)?;
            let result = match hop {
                Hop::Forward(_) => {
                    Received::Forward(self.output(&circuit, frame, hop, None, reading)?)
                }
                Hop::Deliver | Hop::Exit => {
                    let expected = crypto::keys(reading, frame.context.path[0])?
                        .1
                        .public_key
                        .bytes;
                    let completed = (|| {
                        let mut noise = crypto::build(&self.keys, reading, &frame.context, false)?;
                        crypto::read_handshake(&mut noise, &frame.body, 96, expected.as_bytes())?;
                        let body = crypto::write_handshake(&mut noise, 48)?;
                        let noise = crypto::Transport::finish(noise)?;
                        Ok((noise, body))
                    })();
                    let (noise, body) = match completed {
                        Ok(completed) => completed,
                        Err(error) => {
                            // Proof/path admission has succeeded. A cryptographic
                            // failure consumes this CID just as an initiator failure
                            // does; replay or renewal cannot start it again.
                            if error == Error::Crypto {
                                circuit.retire_for("handshake", "completion_failed");
                            }
                            return Err(error);
                        }
                    };
                    let response = Frame {
                        context: frame.context,
                        kind: Kind::Response,
                        reply: true,
                        sequence: 0,
                        probe: None,
                        body: body.into(),
                        proofs: Vec::new(),
                        exchange: Some(circuit.session.exchange()?),
                        auth: Default::default(),
                    };
                    let keys = circuit.session.derive(&response, reading.node())?;
                    circuit.ensure_current()?;
                    circuit.session.establish(keys);
                    let hop = self.handshake_hop(&response, None, reading)?;
                    circuit.state = State::Endpoint(Arc::new(noise));
                    let response = self.output(&circuit, response, hop, None, reading)?;
                    Received::Established {
                        circuit: id,
                        response: Some(response),
                    }
                }
            };
            let output = match &result {
                Received::Forward(output)
                | Received::Established {
                    response: Some(output),
                    ..
                } => output,
                _ => return Err(Error::State),
            };
            circuit.recovery.init = Some(recovery::Stage::new(
                fingerprint,
                circuit.guard(true),
                Some(output),
            )?);
            circuit.guard(true).finish()?;
            Ok(result)
        })();
        if result.is_err() {
            // Once this admitted INIT has been consumed, even a response output
            // or final authority failure must not allow a second Noise owner for
            // the same CID. Keep the responder failure tombstone.
            circuit.retire_for("handshake", "initial_output_failed");
        }
        self.restore(id, Box::new(circuit), reading);
        self.maintenance.changed();
        result
    }

    fn receive_known(
        &mut self,
        circuit: &mut Circuit,
        frame: Frame,
        previous: NodeId,
        reading: &View<'_>,
    ) -> Result<Received, Error> {
        circuit.check_at(reading)?;
        if circuit.context != frame.context {
            return Err(Error::Context);
        }
        match frame.kind {
            Kind::Init => self.repeat_handshake(circuit, &frame),
            Kind::Response => {
                let hop = self.handshake_hop(&frame, Some(previous), reading)?;
                let fingerprint = Sha256::digest(frame.encode()?).into();
                match &circuit.state {
                    State::RelayAwaiting => {
                        let keys = circuit.session.derive(&frame, reading.node())?;
                        let output = self.output(circuit, frame, hop, None, reading)?;
                        circuit.guard(true).finish()?;
                        circuit.recovery.response = Some(recovery::Stage::new(
                            fingerprint,
                            circuit.guard(true),
                            Some(&output),
                        )?);
                        circuit.state = State::RelayEstablished;
                        circuit.session.establish(keys);
                        Ok(Received::Forward(output))
                    }
                    State::Initiator(_) => {
                        if hop != Hop::Deliver {
                            return Err(Error::State);
                        }
                        // Noise handshake reads mutate their state even when
                        // failing. Retire this circuit on any such failure.
                        let keys = circuit.session.derive(&frame, reading.node())?;
                        let expected = crypto::keys(
                            reading,
                            *circuit.context.path.last().ok_or(Error::Context)?,
                        )?
                        .1
                        .public_key
                        .bytes;
                        let guard = circuit.guard(true);
                        guard.finish()?;
                        circuit.complete_handshake(|mut noise| {
                            crypto::read_handshake(
                                &mut noise,
                                &frame.body,
                                48,
                                expected.as_bytes(),
                            )?;
                            guard.finish()?;
                            let noise = crypto::Transport::finish(*noise)?;
                            guard.finish()?;
                            Ok(noise)
                        })?;
                        // Keep the completed cipher and its MAC keys together,
                        // even if the later receipt/currentness check loses the
                        // snapshot to a legal Control successor.
                        circuit.session.establish(keys);
                        circuit.recovery.response =
                            Some(recovery::Stage::new(fingerprint, guard, None)?);
                        circuit.recovery.init = None;
                        Ok(Received::Established {
                            circuit: circuit.context.circuit,
                            response: None,
                        })
                    }
                    State::RelayEstablished | State::Endpoint(_) => {
                        self.repeat_handshake(circuit, &frame)
                    }
                    _ => Err(Error::Replay),
                }
            }
            Kind::Data => Err(Error::State),
            Kind::ProbeRequest | Kind::ProbeReply => {
                self.receive_probe(circuit, frame, previous, reading)
            }
            Kind::DnsRequest | Kind::DnsReply => {
                self.receive_dns(circuit, frame, previous, reading)
            }
        }
    }

    fn repeat_handshake(&self, circuit: &mut Circuit, frame: &Frame) -> Result<Received, Error> {
        let stage = match frame.kind {
            Kind::Init => &mut circuit.recovery.init,
            Kind::Response => &mut circuit.recovery.response,
            _ => return Err(Error::State),
        };
        let stage = stage.as_mut().ok_or(Error::Replay)?;
        let fingerprint: [u8; 32] = Sha256::digest(frame.encode()?).into();
        if stage.fingerprint != fingerprint {
            return Err(Error::Replay);
        }
        stage.ensure_current()?;
        if let Some(output) = stage.output() {
            Self::check_retry_carrier(
                &self.carriers,
                output,
                &circuit.context,
                circuit.selected.as_deref(),
            )?;
            let output = stage.retry()?.ok_or(Error::Replay)?;
            if matches!(circuit.state, State::Endpoint(_)) {
                Ok(Received::Established {
                    circuit: circuit.context.circuit,
                    response: Some(output),
                })
            } else {
                Ok(Received::Forward(output))
            }
        } else {
            Ok(Received::Established {
                circuit: circuit.context.circuit,
                response: None,
            })
        }
    }

    fn check_retry_carrier(
        carriers: &BTreeMap<NodeId, Carrier>,
        output: &PendingFrame,
        context: &Context,
        selected: Option<&LogicalPath>,
    ) -> Result<(), Error> {
        output.ensure_current()?;
        if carriers.get(&output.next) != Some(&output.carrier) {
            return Err(Error::Context);
        }
        let selected = authorization::selected(context, selected).ok();
        authorization::outgoing(selected, output.next, &output.carrier)
    }

    pub fn send(&mut self, id: [u8; 32], packet: Packet) -> Result<PendingFrame, Error> {
        let job = self.prepare_send_packet(id, packet)?;
        let result = pipeline::seal(job);
        let circuit = self.circuits.get_mut(&id).ok_or(Error::State)?;
        circuit.finish_operation("send", result)
    }

    /// Owner admission for already parsed local input; retains its allocation.
    pub fn prepare_send_packet(
        &mut self,
        id: [u8; 32],
        packet: Packet,
    ) -> Result<pipeline::SendJob, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.send");
        let _profile = PROFILE.enter();
        let desired = self.desired.clone();
        let reading = View::of(&desired);
        self.prepare_send_at(id, packet, &reading)
    }

    fn prepare_send_at(
        &mut self,
        id: [u8; 32],
        packet: Packet,
        reading: &View<'_>,
    ) -> Result<pipeline::SendJob, Error> {
        self.maintain_circuits(reading, false)?;
        let circuit = self.circuits.get_mut(&id).ok_or(Error::State)?;
        let result = Self::send_inner(
            OutputOwner {
                carriers: &self.carriers,
            },
            &mut self.engine,
            circuit,
            packet,
            reading,
        );
        let result = circuit.finish_operation("send", result);
        if let Err(error) = &result
            && diagnostics::event()
        {
            tracing::debug!(operation = "send", ?error, cid = ?id,
                grant = ?circuit.context.grant, "Dataplane secure admission rejected");
        }
        result
    }

    fn send_inner(
        owner: OutputOwner<'_>,
        engine: &mut Engine,
        circuit: &mut Circuit,
        packet: Packet,
        reading: &View<'_>,
    ) -> Result<pipeline::SendJob, Error> {
        let grant = circuit.context.grant.flow().ok_or(Error::Policy)?;
        circuit.check_at(reading)?;
        let State::Endpoint(noise) = &circuit.state else {
            return Err(Error::State);
        };
        if packet.as_bytes().len() > MAX_PLAINTEXT {
            return Err(Error::Unsupported);
        }
        let facts = packet.facts();
        let local = reading.node();
        let reply = local != circuit.context.path[0];
        Self::check_outgoing_carrier(owner.carriers, circuit, reply, reading)?;
        let path = &circuit.context.path;
        let entry = Entry {
            grant,
            path,
            origin: path[0],
            previous: None,
            sender: local,
        };
        let route = engine
            .admit_circuit_facts(
                &entry,
                facts,
                reading,
                circuit.context.circuit,
                &circuit.guard(false),
            )
            .map_err(|error| circuit.engine_rejected("send", error))?;
        if (route.route().direction == Direction::Reply) != reply {
            return Err(Error::Context);
        }
        // Flow and cryptographic state have distinct irreversible commit points.
        // Neither a later output failure nor a scope handoff rolls either back.
        circuit.ensure_current()?;
        let sequence = circuit.send[usize::from(reply)].take()?;
        let frame = Frame {
            context: circuit.context.clone(),
            kind: Kind::Data,
            reply,
            sequence,
            probe: None,
            body: packet.into_buffer(),
            proofs: Vec::new(),
            exchange: None,
            auth: Default::default(),
        };
        let (next, carrier, guard) =
            Self::output_scope(owner, circuit, &frame, route.hop(), Some(&route), reading)?;
        let mac = circuit.session.prepare_append(&frame, &carrier)?;
        guard.finish()?;
        let job = pipeline::SendJob {
            frame,
            noise: noise.clone(),
            mac,
            next,
            carrier,
            guard,
        };
        circuit.sent(reading.instant);
        Ok(job)
    }

    fn handshake_hop(
        &mut self,
        frame: &Frame,
        previous: Option<NodeId>,
        reading: &View<'_>,
    ) -> Result<Hop, Error> {
        authorization::admit(
            &mut self.engine.decisions,
            reading,
            &frame.context.grant,
            &frame.context.path,
            previous,
            frame.reply,
        )
        .map(|admitted| admitted.hop)
    }

    // Check scheduling failures before stateful flow admission. This derives
    // only adjacency; policy remains the sole carrier authorization owner.
    fn check_outgoing_carrier(
        carriers: &BTreeMap<NodeId, Carrier>,
        circuit: &Circuit,
        reply: bool,
        reading: &View<'_>,
    ) -> Result<(), Error> {
        let context = &circuit.context;
        let position = context
            .path
            .iter()
            .position(|node| *node == reading.node())
            .ok_or(Error::Context)?;
        let next = if reply {
            position.checked_sub(1).and_then(|i| context.path.get(i))
        } else {
            context.path.get(position + 1)
        };
        if let Some(next) = next {
            let carrier = carriers.get(next).ok_or(Error::Unsupported)?;
            let selected = authorization::selected(context, circuit.selected.as_deref()).ok();
            authorization::outgoing(selected, *next, carrier)?;
        }
        Ok(())
    }

    fn output(
        &self,
        circuit: &Circuit,
        mut frame: Frame,
        hop: Hop,
        route: Option<&PendingRoute>,
        reading: &View<'_>,
    ) -> Result<PendingFrame, Error> {
        let (next, carrier, guard) = Self::output_scope(
            OutputOwner {
                carriers: &self.carriers,
            },
            circuit,
            &frame,
            hop,
            route,
            reading,
        )?;
        if frame.kind.handshake() {
            crypto::append(
                &self.keys,
                reading,
                &mut frame,
                carrier.clone(),
                circuit.session.public(),
            )?;
        } else {
            circuit.session.append(&mut frame, carrier.clone())?;
        }
        let bytes = frame.encode_owned()?;
        guard.finish()?;
        Ok(PendingFrame {
            bytes,
            next,
            carrier,
            circuit: circuit.context.circuit,
            guard,
        })
    }

    fn output_scope(
        owner: OutputOwner<'_>,
        circuit: &Circuit,
        frame: &Frame,
        hop: Hop,
        route: Option<&PendingRoute>,
        reading: &View<'_>,
    ) -> Result<(NodeId, Carrier, Guard), Error> {
        let Hop::Forward(next) = hop else {
            return Err(Error::State);
        };
        let carrier = owner.carriers.get(&next).ok_or(Error::Unsupported)?.clone();
        let selected = authorization::selected(&frame.context, circuit.selected.as_deref()).ok();
        authorization::outgoing(selected, next, &carrier)?;
        let mut guard = circuit.guard(matches!(frame.kind, Kind::Init | Kind::Response));
        if let Some(probe) = frame.probe {
            guard.life.restrict(Some(
                Cap::until_ms(reading.instant, reading.now_ms, probe.expires_at_ms)
                    .ok_or(Error::Expired)?,
            ));
        }
        if let Some(route) = route {
            route.check_at(reading).map_err(|_| Error::Authority)?;
            guard.life.restrict(route.life.cap);
            guard.life.flow = route.life.flow.clone();
            guard.projection = route.projection.clone();
        }
        guard.check_at(reading)?;
        Ok((next, carrier, guard))
    }

    /// The next hop of an origin's established circuit when it has no
    /// carrier now (the adjacency idled out or its channel failed).
    fn missing_next_hop(&self, id: [u8; 32], local: NodeId) -> Option<NodeId> {
        let circuit = self.circuits.get(&id)?;
        let path = &circuit.context.path;
        let next = (path.first() == Some(&local))
            .then(|| path.get(1))
            .flatten()?;
        (!self.carriers.contains_key(next)).then_some(*next)
    }

    fn capacity(&self) -> Result<(), Error> {
        if self.circuits.len() >= self.limits.max_circuits
            || self.dead.len() >= self.limits.max_circuits.saturating_mul(4)
        {
            Err(Error::Capacity)
        } else {
            Ok(())
        }
    }

    /// Host-driven cleanup; closed states leave compact dead-CID records.
    pub fn maintain(&mut self) -> Result<(), Error> {
        let desired = self.desired.clone();
        let reading = View::of(&desired);
        self.maintain_circuits(&reading, true)?;
        self.engine.maintain_at(&reading);
        Ok(())
    }

    /// Cancel queued wire/plaintext outputs, retire circuit keys and retain the
    /// ID replay record. Other circuits and Control's grant are unaffected.
    pub fn close(&mut self, id: [u8; 32]) {
        self.engine.flows.retire_exit_circuit(id);
        if let Some(mut circuit) = self.circuits.remove(&id) {
            circuit.retire();
            self.bury(id, &circuit, crate::authority::now_ms());
        }
    }

    /// Reading metadata never refreshes a deadline or manufactures a sample.
    pub fn circuit_info(&self, id: [u8; 32]) -> Result<Option<CircuitInfo>, Error> {
        Ok(self.circuit_info_at(id, &View::of(&self.desired)))
    }

    fn circuit_info_at(&self, id: [u8; 32], reading: &View<'_>) -> Option<CircuitInfo> {
        let circuit = self.circuits.get(&id)?;
        let live = circuit.check_at(reading).is_ok();
        let expiry = circuit.next_deadline()?;
        let local = reading.node();
        Some(CircuitInfo {
            id,
            grant: circuit.context.grant.clone(),
            path: circuit.context.path.clone(),
            role: if circuit.context.path.first() == Some(&local) {
                CircuitRole::Origin
            } else if circuit.context.path.last() == Some(&local) {
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
            expires_at: expiry.at,
            expires_at_ms: expiry.ms,
        })
    }

    pub fn ready_circuits(&self) -> Result<Vec<CircuitInfo>, Error> {
        let desired = &self.desired;
        let reading = View::of(desired);
        let mut ready = Vec::new();
        for id in self.circuits.keys() {
            if let Some(info) = self.circuit_info_at(*id, &reading)
                && info.state == CircuitState::Established
            {
                ready.push(info);
            }
        }
        Ok(ready)
    }

    /// Select a local output from the unique flow owner's authenticated cipher
    /// binding. A terminal must never guess among locally completed handshakes:
    /// the origin may not have received one of their responses. Only an origin
    /// can acquire another cipher for its unchanged route after the old one ends.
    /// Reading this does not advance any flow, nonce or deadline; send rechecks.
    pub fn local_grant_circuit(
        &self,
        grant: &CircuitGrant,
        path: &[NodeId],
        packet: &Packet,
    ) -> Result<Option<CircuitInfo>, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.local_circuit");
        let _profile = PROFILE.enter();
        self.local_circuit_at(grant, path, packet, &View::of(&self.desired))
    }

    fn local_circuit_at(
        &self,
        grant: &CircuitGrant,
        path: &[NodeId],
        packet: &Packet,
        reading: &View<'_>,
    ) -> Result<Option<CircuitInfo>, Error> {
        let origin = path.first() == Some(&reading.node());
        let flow_grant = grant.flow().ok_or(Error::Policy)?;
        if let Some(flow) = self.local_flow_at(packet, reading)? {
            if flow.grant != flow_grant || flow.path != path {
                return Err(Error::Flow);
            }
            if let Some(id) = flow.circuit
                && let Some(info) = self.circuit_info_at(id, reading)
                && info.state != CircuitState::Closed
            {
                if info.grant != *grant || info.path != path {
                    return Err(Error::Context);
                }
                return Ok((info.state == CircuitState::Established).then_some(info));
            }
        }
        if !origin {
            return Ok(None);
        }
        Ok(self.established(grant, path, reading))
    }

    /// An established origin circuit of exactly this grant and path.
    fn established(
        &self,
        grant: &CircuitGrant,
        path: &[NodeId],
        reading: &View<'_>,
    ) -> Option<CircuitInfo> {
        // Filter immutable bindings before checking liveness or allocating
        // metadata. Unrelated circuits cannot supply this origin's output.
        for (id, circuit) in &self.circuits {
            if circuit.context.grant == *grant
                && circuit.context.path == path
                && circuit.state.established()
                && let Some(info) = self.circuit_info_at(*id, reading)
                && info.state == CircuitState::Established
            {
                return Some(info);
            }
        }
        None
    }
    pub fn circuit_count(&self) -> usize {
        self.circuits.len()
    }
    /// Compact closed-CID records still inside their INIT window.
    pub fn dead_circuits(&self) -> usize {
        self.dead.len()
    }
    pub fn flow_count(&self) -> usize {
        self.engine.flow_count()
    }

    /// Read original grant/path metadata from the one Engine flow table. This
    /// neither selects nor opens a circuit and never grants packet authority.
    /// An existing route remains existing even if its carrier/circuit is down.
    pub fn local_flow(&self, packet: &Packet) -> Result<Option<crate::LocalFlow>, Error> {
        let reading = View::of(&self.desired);
        self.local_flow_at(packet, &reading)
    }

    fn local_flow_at(
        &self,
        packet: &Packet,
        reading: &View<'_>,
    ) -> Result<Option<crate::LocalFlow>, Error> {
        self.engine
            .local_flow_at(packet, reading)
            .map_err(|error| match error {
                crate::Error::Authority => Error::Authority,
                crate::Error::Flow => Error::Flow,
                crate::Error::Expired => Error::Expired,
                _ => Error::Packet,
            })
    }

    /// Retire every circuit (cancelling all their queued outputs), clear every
    /// flow and publish Retire for every exit projection.
    fn terminate(&mut self, cause: &'static str) {
        let now_ms = crate::authority::now_ms();
        for (id, mut circuit) in std::mem::take(&mut self.circuits) {
            circuit.retire_for(cause, "terminated");
            self.bury(id, &circuit, now_ms);
        }
        self.first.clear();
        self.engine.flows.clear();
    }
}

#[cfg(test)]
mod tests;
