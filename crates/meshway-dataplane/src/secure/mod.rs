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
//! Hosts retain guarded outputs through I/O readiness and release at their final
//! synchronous sink. Closing a circuit cancels its queued wire/plaintext outputs;
//! dropping the node cancels every circuit, independently of Control's lease.
//! Steady-state packets carry a CID and a fixed-size hop MAC; no per-packet
//! public-key operation or unauthenticated context reconstruction is used.
mod authorization;
mod codec;
mod crypto;
mod diagnostics;
mod exit;
mod maintenance;
mod netmap;
pub mod pipeline;
mod probe;
mod recovery;
mod replay;
mod session;

use crate::{
    Direction, Engine, Hop, Packet, PacketBuffer, PendingAction, PendingRoute,
    authority::{Cap, Lifetime, View},
    flow::{Entry, FlowGrant},
};
pub use codec::is_control_frame;
use codec::{Context, Frame, Kind};
use meshway_authority::{DataPlaneIdentity, ExecutionSnapshot};
pub use meshway_configuration::execution::CircuitAuthorization;
use meshway_configuration::execution::LogicalPath;
use meshway_model::{NodeId, ObservationId, RequestId};
pub use meshway_policy::admission::Carrier;
use meshway_policy::admission::circuit_permit;
pub use meshway_policy::compile::netmap::Target;
pub use netmap::{AdjacencyInterest, Flushed, Outbound, Update, diagnostic_paths};
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

/// The stable grant a circuit is admitted under; never installed content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CircuitGrant {
    /// (origin = `path[0]`, target, purpose) on the installed netmap.
    Netmap {
        purpose: crate::Purpose,
        target: Target,
    },
    /// Legacy per-demand or per-observation grant; removed with the daemon switch.
    Legacy(CircuitAuthorization),
}

impl CircuitGrant {
    fn diagnostic(&self) -> bool {
        matches!(
            self,
            Self::Legacy(CircuitAuthorization::Diagnostic(_))
                | Self::Netmap {
                    purpose: crate::Purpose::Diagnostic,
                    ..
                }
        )
    }

    /// The legacy field value: the grant itself, or a fixed placeholder for
    /// netmap circuits (meaningless there; read `grant`).
    pub fn legacy(&self) -> CircuitAuthorization {
        match self {
            Self::Legacy(authorization) => *authorization,
            Self::Netmap {
                purpose: crate::Purpose::Business,
                ..
            } => CircuitAuthorization::Business(crate::netmap_demand()),
            Self::Netmap { .. } => CircuitAuthorization::Diagnostic(netmap_observation()),
        }
    }

    fn flow(&self) -> Option<FlowGrant> {
        match self {
            Self::Legacy(CircuitAuthorization::Business(demand)) => {
                Some(FlowGrant::Legacy(*demand))
            }
            Self::Netmap {
                purpose: crate::Purpose::Business,
                target,
            } => Some(FlowGrant::Netmap(target.clone())),
            _ => None,
        }
    }
}

fn netmap_observation() -> ObservationId {
    static ID: std::sync::OnceLock<ObservationId> = std::sync::OnceLock::new();
    *ID.get_or_init(|| {
        "00000000-0000-4000-8000-00000000ffed"
            .parse()
            .expect("valid placeholder")
    })
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
    pub(crate) life: Lifetime,
    projection: Option<Arc<crate::exit::Gate>>,
    /// Minted only by this SecureNode for outputs sharing an exact snapshot.
    /// It permits one fresh reading per batch, never across an await or syscall.
    pub(crate) section: Option<Arc<()>>,
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
            snapshot: route.snapshot.clone(),
            life: route.life.clone(),
            section: route.section.clone(),
            projection: route.projection.clone(),
        }
    }
    pub(crate) fn expiry(&self) -> Cap {
        self.life.expiry(&self.snapshot)
    }

    pub(crate) fn check_batch(guards: &[Self]) -> Result<(), Error> {
        if let Some(first) = guards.first()
            && let Some(section) = &first.section
            && guards.iter().all(|guard| {
                guard
                    .section
                    .as_ref()
                    .is_some_and(|other| Arc::ptr_eq(section, other))
            })
        {
            let reading = View::of(&first.snapshot).map_err(|_| Error::Authority)?;
            for guard in guards {
                guard.check_at(&reading)?;
                if let Some(gate) = &guard.projection {
                    gate.ready().map_err(|_| Error::Expired)?;
                }
            }
            return Ok(());
        }
        let mut readings: Vec<(usize, View<'_>)> = Vec::new();
        for (index, guard) in guards.iter().enumerate() {
            if let Some(gate) = &guard.projection {
                gate.ready().map_err(|_| Error::Expired)?;
            }
            let shared = readings.iter().find(|(first, _)| {
                guards[*first]
                    .section
                    .as_ref()
                    .zip(guard.section.as_ref())
                    .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
            });
            if let Some((_, reading)) = shared {
                guard.check_at(reading)?;
            } else {
                let reading = View::of(&guard.snapshot).map_err(|_| Error::Authority)?;
                guard.check_at(&reading)?;
                readings.push((index, reading));
            }
        }
        Ok(())
    }
    pub(crate) fn snapshot(&self) -> &ExecutionSnapshot {
        &self.snapshot
    }
    // Only for a synchronous section using this owner's entry/final reading.
    // Original queued/recovery scopes must use their own dynamic guard instead.
    pub(crate) fn check_at(&self, reading: &View<'_>) -> Result<(), Error> {
        if !self.life.live_at(reading.instant, reading.now_ms) {
            return Err(Error::Expired);
        }
        Ok(())
    }

    // A complete fresh snapshot check immediately before a synchronous commit
    // or output. This never replaces the original release/poll guard below.
    fn finish(&self) -> Result<(), Error> {
        let current = View::of(&self.snapshot).map_err(|_| Error::Authority)?;
        self.check_at(&current)
    }

    pub(crate) fn ensure_current(&self) -> Result<(), Error> {
        self.ensure_before(u64::MAX)
    }

    pub(crate) fn ensure_before(&self, deadline_ms: u64) -> Result<(), Error> {
        if self.life.cancellation.is_cancelled() {
            return Err(Error::Expired);
        }
        let reading = View::of(&self.snapshot).map_err(|_| Error::Authority)?;
        self.check_at(&reading)?;
        if reading.now_ms >= deadline_ms {
            return Err(Error::Expired);
        }
        if let Some(gate) = &self.projection {
            gate.ready().map_err(|_| Error::Expired)?;
        }
        Ok(())
    }
    pub(crate) async fn invalidated(&self) {
        if self.ensure_current().is_err() {
            return;
        }
        self.life.ended(&self.snapshot).await;
    }
    pub(crate) fn expires_at(&self) -> Instant {
        self.expiry().at
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
    pub async fn invalidated(&self) {
        self.guard.invalidated().await;
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
    /// Legacy view of `grant`; see [`CircuitGrant::legacy`].
    pub authorization: CircuitAuthorization,
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
    /// Legacy view of `grant`; see [`CircuitGrant::legacy`].
    pub authorization: CircuitAuthorization,
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
    /// Own bound: the legacy permit and execution expiry. Netmap circuits
    /// have none; their lease is checked at every use and renews in place.
    cap: Option<Cap>,
    handshake: Cap,
    permit_expires_at_ms: u64,
    /// Netmap: the exact path this owner admitted, with its dial candidates.
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
        let (cap, permit_expires_at_ms) = match reading.legacy() {
            Some(cfg) => {
                let CircuitGrant::Legacy(authorization) = context.grant else {
                    return Err(Error::Context);
                };
                let permit = circuit_permit(cfg, authorization).map_err(|_| Error::Context)?;
                let permit = Cap::until_ms(reading.instant, reading.now_ms, permit.expires_at_ms)
                    .ok_or(Error::Limits)?;
                let execution = Cap {
                    at: reading.expires_at,
                    ms: reading.expires_at_ms,
                };
                (Some(permit.min(execution)), permit.ms)
            }
            None => (None, u64::MAX),
        };
        let mut handshake =
            Cap::after(reading.instant, reading.now_ms, timeout).ok_or(Error::Limits)?;
        // The INIT window also bounds a responder's handshake state.
        let window = context.initiated_at_ms.saturating_add(
            u64::try_from((timeout + INIT_SKEW).as_millis()).map_err(|_| Error::Limits)?,
        );
        handshake = handshake
            .min(Cap::until_ms(reading.instant, reading.now_ms, window).ok_or(Error::Limits)?);
        Ok(Self {
            context,
            state,
            cap,
            handshake,
            permit_expires_at_ms,
            selected,
            send: Default::default(),
            receive: Default::default(),
            cancellation,
            recovery: Default::default(),
            session: session::Session::new()?,
        })
    }
    fn guard(&self, snapshot: &ExecutionSnapshot, handshake: bool) -> Guard {
        let mut life = Lifetime::new(self.cap, self.cancellation.clone());
        if handshake {
            life.restrict(Some(self.handshake));
        }
        Guard {
            snapshot: snapshot.clone(),
            section: None,
            life,
            projection: None,
        }
    }
    fn ensure_current(&self, snapshot: &ExecutionSnapshot) -> Result<(), Error> {
        let current = View::of(snapshot).map_err(|_| Error::Authority)?;
        self.check_at(&current)
    }

    fn check_at(&self, reading: &View<'_>) -> Result<(), Error> {
        if matches!(self.state, State::Closed) {
            return Err(Error::State);
        }
        let now = (reading.instant, reading.now_ms);
        if self.cancellation.is_cancelled()
            || self.cap.is_some_and(|cap| !cap.live_at(now.0, now.1))
            || (!self.state.established() && !self.handshake.live_at(now.0, now.1))
        {
            return Err(Error::Expired);
        }
        Ok(())
    }

    /// Bound of this circuit for scheduling, before the snapshot's own end.
    fn next_deadline(&self) -> Option<Cap> {
        let handshake = (!self.state.established()).then_some(self.handshake);
        crate::authority::restrict(self.cap, handshake)
    }

    /// Lost snapshot authority stops this operation, but is not a cryptographic
    /// close. Only a Control-checked update can make retained state usable.
    /// Terminal state, cancellation and observed circuit expiry remain terminal.
    fn validate_operation(
        &mut self,
        snapshot: &ExecutionSnapshot,
        operation: &'static str,
    ) -> Result<(), Error> {
        tracing::trace!(target: "meshway_dataplane::secure::authority_boundary", operation,
            "Validating circuit authority");
        let result = self.ensure_current(snapshot);
        if let Err(error) = result
            && error != Error::Authority
        {
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
        snapshot: &ExecutionSnapshot,
        operation: &'static str,
        result: Result<T, Error>,
    ) -> Result<T, Error> {
        let current = self.validate_operation(snapshot, operation);
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
                permit_expires_at_ms = self.permit_expires_at_ms,
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

/// One execution identity, one shared flow table, and a bounded circuit table.
/// Closed circuits leave only a compact dead-CID record until their INIT
/// window has passed; any later replay of that INIT is rejected as stale.
/// Dead records do not count towards `max_circuits`.
pub struct SecureNode {
    identity: DataPlaneIdentity,
    engine: Engine,
    limits: Limits,
    carriers: BTreeMap<NodeId, Carrier>,
    // Stable allocation for the owner state: remove/restore coordinates borrows
    // without moving replay windows, recovery state and session metadata per packet.
    circuits: BTreeMap<[u8; 32], Box<Circuit>>,
    /// Closed CID → UTC milliseconds until which its INIT is still acceptable.
    dead: BTreeMap<[u8; 32], u64>,
    probes: probe::Probes,
    maintenance: maintenance::Maintenance,
    cancellation: CancellationToken,
    output_epoch: Arc<()>,
    first: netmap::FirstPackets,
}

#[derive(Clone, Copy)]
struct OutputOwner<'a> {
    identity: &'a DataPlaneIdentity,
    carriers: &'a BTreeMap<NodeId, Carrier>,
    epoch: &'a Arc<()>,
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
        let snapshot = self.identity.snapshot();
        let session = codec::session_id(bytes)?;
        let context = if let Some(id) = session {
            let reading = View::of(snapshot).map_err(|_| Error::Authority)?;
            let circuit = self.circuits.get(&id).ok_or(Error::State)?;
            circuit.check_at(&reading)?;
            circuit.context.clone()
        } else {
            // A full handshake hint is only an untrusted bounded decode, even
            // when this owner's grant has ended. receive still requires entry
            // authority before authenticating or accepting anything from it.
            Frame::decode(bytes, None)?.context
        };
        if session.is_some() {
            snapshot.ensure_current().map_err(|_| Error::Authority)?;
        }
        Ok(UntrustedIncomingHint {
            authorization: context.grant.legacy(),
            grant: context.grant.clone(),
            path: context.path.clone(),
        })
    }

    /// Replace host-observed authenticated carrier availability without changing
    /// circuit IDs, replay windows, flows or their complete logical paths.
    /// Every carrier must be an admitted adjacency of the current target; every
    /// later output still checks it against its circuit's exact path.
    pub fn set_carriers(&mut self, carriers: BTreeMap<NodeId, Carrier>) -> Result<(), Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.set_carriers");
        let _profile = PROFILE.enter();
        let snapshot = self.identity.snapshot().clone();
        let reading = View::of(&snapshot).map_err(|_| Error::Authority)?;
        for (peer, carrier) in &carriers {
            if carrier.listener_key.is_empty() || carrier.listener_key.len() > 255 {
                return Err(Error::Unsupported);
            }
            if !authorization::adjacency(&mut self.engine.decisions, &reading, *peer, carrier) {
                return Err(Error::Policy);
            }
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
        View::of(identity.snapshot()).map_err(|_| Error::Authority)?;
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
        let engine =
            Engine::new_current(identity.snapshot().clone(), limits.flows).map_err(|error| {
                match error {
                    crate::Error::Limits => Error::Limits,
                    _ => Error::Authority,
                }
            })?;
        let mut node = Self {
            identity,
            engine,
            limits,
            carriers: BTreeMap::new(),
            circuits: BTreeMap::new(),
            dead: BTreeMap::new(),
            probes: Default::default(),
            maintenance: Default::default(),
            cancellation: CancellationToken::new(),
            output_epoch: Arc::new(()),
            first: Default::default(),
        };
        if node.identity.snapshot().is_netmap() {
            node.set_carriers(carriers)?;
        } else {
            // Legacy construction accepted carriers before any permit used them.
            node.carriers = carriers;
        }
        Ok(node)
    }

    /// Legacy: open a business circuit of a demand on its next-hop carrier.
    pub fn start(&mut self, demand: RequestId, path: Vec<NodeId>) -> Result<PendingFrame, Error> {
        let carrier = self
            .carriers
            .get(path.get(1).ok_or(Error::Context)?)
            .ok_or(Error::Unsupported)?
            .clone();
        self.start_grant(
            CircuitGrant::Legacy(CircuitAuthorization::Business(demand)),
            path,
            &carrier,
        )
    }

    /// Legacy: open a circuit of an exact business or diagnostic grant.
    pub fn start_authorized(
        &mut self,
        authorization: CircuitAuthorization,
        path: Vec<NodeId>,
        carrier: &Carrier,
    ) -> Result<PendingFrame, Error> {
        self.start_grant(CircuitGrant::Legacy(authorization), path, carrier)
    }

    /// Open a circuit of a grant on the installed target. A netmap grant
    /// names (purpose, target); `path` must be one of the pair's paths.
    pub fn start_grant(
        &mut self,
        grant: CircuitGrant,
        path: Vec<NodeId>,
        carrier: &Carrier,
    ) -> Result<PendingFrame, Error> {
        let snapshot = self.identity.snapshot().clone();
        let reading = View::of(&snapshot).map_err(|_| Error::Authority)?;
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
        self.engine
            .maintain_at(reading)
            .map_err(|_| Error::Authority)?;
        self.capacity()?;
        let snapshot = self.identity.snapshot().clone();
        if path.first() != Some(&reading.node()) {
            return Err(Error::Context);
        }
        if self.carriers.get(path.get(1).ok_or(Error::Context)?) != Some(carrier) {
            return Err(Error::Context);
        }
        let admitted = authorization::admit(
            &mut self.engine.decisions,
            &snapshot,
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
        let mut noise = crypto::build(&self.identity, reading, &context, true)?;
        let body = crypto::write_handshake(&mut noise, 96)?;
        snapshot.ensure_current().map_err(|_| Error::Authority)?;
        let mut circuit = Circuit::new(
            context.clone(),
            State::Initiator(Box::new(noise)),
            reading,
            self.limits.handshake_timeout,
            self.cancellation.child_token(),
            admitted.selected,
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
        let snapshot = self.identity.snapshot();
        let reading = View::of(snapshot).map_err(|_| Error::Authority)?;
        let circuit = self.circuits.get(&id).ok_or(Error::State)?;
        circuit.check_at(&reading)?;
        snapshot.ensure_current().map_err(|_| Error::Authority)?;
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
        let snapshot = self.identity.snapshot().clone();
        let reading = View::of(&snapshot).map_err(|_| Error::Authority)?;
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
                    &reading,
                )?;
            }
            Ok(output)
        })();
        let result = circuit.finish_operation(self.identity.snapshot(), "retry_handshake", result);
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
        let snapshot = self.identity.snapshot().clone();
        let reading = View::of(&snapshot).map_err(|_| Error::Authority)?;
        self.maintain_circuits(&reading, false)?;
        let id = codec::session_id(bytes.as_ref())?;
        let retained = id
            .as_ref()
            .and_then(|id| self.circuits.get(id))
            .map(|circuit| &circuit.context);
        let frame = Frame::decode_owned(bytes.into(), retained)?;
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
            let selected =
                authorization::selected(&reading, &circuit.context, circuit.selected.as_deref())?;
            let previous = circuit.session.verify(selected, &frame, incoming)?;
            authorization::incoming(
                &mut self.engine.decisions,
                &reading,
                &circuit.context,
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
            // Always restore retained state on rejection. A consumed nonce or
            // handshake must never roll back as a side effect of an output error.
            let result = self.receive_known(&mut circuit, frame, previous, &reading);
            let result = circuit.finish_operation(self.identity.snapshot(), "receive", result);
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
        let snapshot = self.identity.snapshot().clone();
        // Path admission precedes signature work; the arrival direction is
        // checked after the chain names the authenticated previous hop.
        let selected = match &frame.context.grant {
            CircuitGrant::Legacy(_) => None,
            CircuitGrant::Netmap { target, .. } => {
                let netmap = reading.netmap().ok_or(Error::Context)?;
                let origin = frame.context.path[0];
                let paths = self
                    .engine
                    .decisions
                    .transit(netmap, origin, target)
                    .map_err(|_| Error::Policy)?;
                Some(Arc::new(
                    paths
                        .iter()
                        .find(|path| path.nodes == frame.context.path)
                        .ok_or(Error::Policy)?
                        .clone(),
                ))
            }
        };
        let path = authorization::selected(reading, &frame.context, selected.as_deref())?;
        let previous = crypto::verify(reading, frame, incoming, path)?;
        authorization::incoming(
            &mut self.engine.decisions,
            reading,
            &frame.context,
            path,
            previous,
            incoming,
        )?;
        let admitted = authorization::admit(
            &mut self.engine.decisions,
            &snapshot,
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
        let snapshot = self.identity.snapshot().clone();
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
            admitted.selected,
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
                        let mut noise =
                            crypto::build(&self.identity, reading, &frame.context, false)?;
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
                    circuit.ensure_current(&snapshot)?;
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
                circuit.guard(&snapshot, true),
                Some(output),
            )?);
            circuit.guard(&snapshot, true).finish()?;
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
        let snapshot = self.identity.snapshot().clone();
        circuit.check_at(reading)?;
        if circuit.context != frame.context {
            return Err(Error::Context);
        }
        match frame.kind {
            Kind::Init => self.repeat_handshake(circuit, &frame, reading),
            Kind::Response => {
                let hop = self.handshake_hop(&frame, Some(previous), reading)?;
                let fingerprint = Sha256::digest(frame.encode()?).into();
                match &circuit.state {
                    State::RelayAwaiting => {
                        let keys = circuit.session.derive(&frame, reading.node())?;
                        let output = self.output(circuit, frame, hop, None, reading)?;
                        circuit.guard(&snapshot, true).finish()?;
                        circuit.recovery.response = Some(recovery::Stage::new(
                            fingerprint,
                            circuit.guard(&snapshot, true),
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
                        let guard = circuit.guard(&snapshot, true);
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
                        self.repeat_handshake(circuit, &frame, reading)
                    }
                    _ => Err(Error::Replay),
                }
            }
            Kind::Data => Err(Error::State),
            Kind::ProbeRequest | Kind::ProbeReply => {
                self.receive_probe(circuit, frame, previous, reading)
            }
        }
    }

    fn repeat_handshake(
        &self,
        circuit: &mut Circuit,
        frame: &Frame,
        reading: &View<'_>,
    ) -> Result<Received, Error> {
        let stage = match frame.kind {
            Kind::Init => &mut circuit.recovery.init,
            Kind::Response => &mut circuit.recovery.response,
            Kind::Data | Kind::ProbeRequest | Kind::ProbeReply => return Err(Error::State),
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
                reading,
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
        reading: &View<'_>,
    ) -> Result<(), Error> {
        output.ensure_current()?;
        if carriers.get(&output.next) != Some(&output.carrier) {
            return Err(Error::Context);
        }
        let selected = authorization::selected(reading, context, selected).ok();
        authorization::outgoing(reading, context, selected, output.next, &output.carrier)
    }

    pub fn send(&mut self, id: [u8; 32], packet: Packet) -> Result<PendingFrame, Error> {
        let job = self.prepare_send_packet(id, packet)?;
        let result = pipeline::seal(job);
        let circuit = self.circuits.get_mut(&id).ok_or(Error::State)?;
        circuit.finish_operation(self.identity.snapshot(), "send", result)
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
        let snapshot = self.identity.snapshot().clone();
        let reading = View::of(&snapshot).map_err(|_| Error::Authority)?;
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
                identity: &self.identity,
                carriers: &self.carriers,
                epoch: &self.output_epoch,
            },
            &mut self.engine,
            circuit,
            packet,
            reading,
        );
        let result = circuit.finish_operation(self.identity.snapshot(), "send", result);
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
                &circuit.guard(owner.identity.snapshot(), false),
            )
            .map_err(|error| circuit.engine_rejected("send", error))?;
        if (route.route().direction == Direction::Reply) != reply {
            return Err(Error::Context);
        }
        // Flow and cryptographic state have distinct irreversible commit points.
        // Neither a later output failure nor a scope handoff rolls either back.
        circuit.ensure_current(owner.identity.snapshot())?;
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
        Ok(pipeline::SendJob {
            frame,
            noise: noise.clone(),
            mac,
            next,
            carrier,
            guard,
        })
    }

    fn handshake_hop(
        &mut self,
        frame: &Frame,
        previous: Option<NodeId>,
        reading: &View<'_>,
    ) -> Result<Hop, Error> {
        let snapshot = self.identity.snapshot().clone();
        authorization::admit(
            &mut self.engine.decisions,
            &snapshot,
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
            let selected =
                authorization::selected(reading, context, circuit.selected.as_deref()).ok();
            authorization::outgoing(reading, context, selected, *next, carrier)?;
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
                identity: &self.identity,
                carriers: &self.carriers,
                epoch: &self.output_epoch,
            },
            circuit,
            &frame,
            hop,
            route,
            reading,
        )?;
        if frame.kind.handshake() {
            crypto::append(
                &self.identity,
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
        let snapshot = owner.identity.snapshot();
        let carrier = owner.carriers.get(&next).ok_or(Error::Unsupported)?.clone();
        let selected =
            authorization::selected(reading, &frame.context, circuit.selected.as_deref()).ok();
        authorization::outgoing(reading, &frame.context, selected, next, &carrier)?;
        let mut guard = circuit.guard(snapshot, matches!(frame.kind, Kind::Init | Kind::Response));
        if let Some(probe) = frame.probe {
            guard.life.restrict(Some(Cap {
                at: guard.life.cap.map_or(reading.expires_at, |cap| cap.at),
                ms: probe.expires_at_ms,
            }));
        }
        if let Some(route) = route {
            route.check_at(reading).map_err(|_| Error::Authority)?;
            guard.snapshot = route.snapshot().clone();
            guard.life.restrict(route.life.cap);
            guard.life.flow = route.life.flow.clone();
            guard.projection = route.projection.clone();
        }
        guard.check_at(reading)?;
        guard.section = Some(owner.epoch.clone());
        Ok((next, carrier, guard))
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

    /// Legacy name of [`Self::update`].
    pub fn reconcile(&mut self, identity: DataPlaneIdentity) -> Result<(), Error> {
        self.update(identity).map(|_| ())
    }

    /// Host-driven cleanup; closed states leave compact dead-CID records.
    pub fn maintain(&mut self) -> Result<(), Error> {
        let snapshot = self.identity.snapshot().clone();
        let reading = match View::of(&snapshot) {
            Ok(reading) => reading,
            Err(_) => {
                self.terminate_if_expired();
                return Err(Error::Authority);
            }
        };
        self.maintain_circuits(&reading, true)?;
        self.engine
            .maintain_at(&reading)
            .map_err(|_| Error::Authority)
    }

    /// Cancel queued wire/plaintext outputs, retire circuit keys and retain the
    /// ID replay record. Other circuits and Control's grant are unaffected.
    pub fn close(&mut self, id: [u8; 32]) {
        self.engine.flows.retire_exit_circuit(id);
        if let Some(mut circuit) = self.circuits.remove(&id) {
            circuit.retire();
            let now_ms = meshway_authority::now().unwrap_or(u64::MAX);
            self.bury(id, &circuit, now_ms);
        }
    }

    /// Reading metadata never refreshes a deadline or manufactures a sample.
    pub fn circuit_info(&self, id: [u8; 32]) -> Result<Option<CircuitInfo>, Error> {
        let snapshot = self.identity.snapshot();
        let reading = View::of(snapshot).map_err(|_| Error::Authority)?;
        let info = self.circuit_info_at(id, &reading);
        snapshot.ensure_current().map_err(|_| Error::Authority)?;
        Ok(info)
    }

    fn circuit_info_at(&self, id: [u8; 32], reading: &View<'_>) -> Option<CircuitInfo> {
        let snapshot = self.identity.snapshot();
        let circuit = self.circuits.get(&id)?;
        let live = circuit.check_at(reading).is_ok();
        let expiry = circuit
            .guard(snapshot, !circuit.state.established())
            .expiry();
        let local = reading.node();
        Some(CircuitInfo {
            id,
            authorization: circuit.context.grant.legacy(),
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
        let snapshot = self.identity.snapshot();
        let reading = View::of(snapshot).map_err(|_| Error::Authority)?;
        let mut ready = Vec::new();
        for id in self.circuits.keys() {
            if let Some(info) = self.circuit_info_at(*id, &reading)
                && info.state == CircuitState::Established
            {
                ready.push(info);
            }
        }
        snapshot.ensure_current().map_err(|_| Error::Authority)?;
        Ok(ready)
    }

    /// Legacy: select a local output of a demand; see [`Self::local_grant_circuit`].
    pub fn local_circuit(
        &self,
        demand: RequestId,
        path: &[NodeId],
        packet: &Packet,
    ) -> Result<Option<CircuitInfo>, Error> {
        self.local_grant_circuit(
            &CircuitGrant::Legacy(CircuitAuthorization::Business(demand)),
            path,
            packet,
        )
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
        let snapshot = self.identity.snapshot();
        let reading = View::of(snapshot).map_err(|_| Error::Authority)?;
        let result = self.local_circuit_at(grant, path, packet, &reading);
        snapshot.ensure_current().map_err(|_| Error::Authority)?;
        result
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
    pub fn retained_circuits(&self) -> usize {
        self.circuits.len()
    }
    /// Compact closed-CID records still inside their INIT window.
    pub fn dead_circuits(&self) -> usize {
        self.dead.len()
    }
    pub fn retained_flows(&self) -> usize {
        self.engine.retained_flows()
    }

    /// Read original grant/path metadata from the one Engine flow table. This
    /// neither selects nor opens a circuit and never grants packet authority.
    /// An existing route remains existing even if its carrier/circuit is down.
    pub fn local_flow(&self, packet: &Packet) -> Result<Option<crate::LocalFlow>, Error> {
        let reading = View::of(self.identity.snapshot()).map_err(|_| Error::Authority)?;
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
                crate::Error::Authority(_) => Error::Authority,
                crate::Error::Flow => Error::Flow,
                crate::Error::Expired => Error::Expired,
                _ => Error::Packet,
            })
    }

    /// Wait for the snapshot to end, then terminate every circuit and flow.
    pub async fn invalidated(&mut self) {
        self.identity.snapshot().invalidated().await;
        self.terminate("invalidated_wait");
    }

    /// Retire every circuit (cancelling all their queued outputs), clear every
    /// flow and publish Retire for every exit projection.
    fn terminate(&mut self, cause: &'static str) {
        let now_ms = meshway_authority::now().unwrap_or(u64::MAX);
        for (id, mut circuit) in std::mem::take(&mut self.circuits) {
            circuit.retire_for(cause, "terminated");
            self.bury(id, &circuit, now_ms);
        }
        self.first.clear();
        self.engine.flows.clear();
    }

    /// An ended netmap lease terminates everything; an invalidated snapshot
    /// may still be followed by a Control-checked update.
    fn terminate_if_expired(&mut self) {
        let snapshot = self.identity.snapshot();
        if snapshot.is_netmap()
            && meshway_authority::now().is_ok_and(|now| now >= snapshot.expires_at_ms())
        {
            self.terminate("lease_expired");
        }
    }
}

#[cfg(test)]
mod tests;
