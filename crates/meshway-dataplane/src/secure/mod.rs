//! Versioned candidate Mesh circuits, not a stable wire ABI.
//!
//! Noise IK protects IP payloads end to end between the original source and the
//! terminal node. Relays see authenticated five-tuples, TCP flags/sequence facts,
//! echo identifiers and packet lengths, plus ciphertext. Every forwarding node
//! verifies ordered Ed25519 proofs during ephemeral session establishment, then
//! verifier-specific chained MACs over frames and actual carrier descriptors.
//! Replies use that exact path in
//! reverse; carrier changes never authorize another logical path.
//!
//! This synchronous owner opens no sockets and creates no background tasks.
//! Hosts retain guarded outputs through I/O readiness and release at their final
//! synchronous sink. Closing a circuit cancels its queued wire/plaintext outputs;
//! dropping the node cancels every circuit, independently of Control's lease.
//! Steady-state packets carry a CID and compact carrier path; no per-packet
//! public-key operation or unauthenticated context reconstruction is used.
mod authorization;
mod codec;
mod crypto;
mod diagnostics;
mod probe;
mod recovery;
mod replay;
mod session;

use crate::{
    AuthenticatedPeer, Direction, Engine, Hop, Ingress, Packet, PendingAction, PendingRoute,
};
use codec::{Context, Frame, Kind};
pub use meshway_configuration::execution::CircuitAuthorization;
use meshway_control::{DataPlaneIdentity, ExecutionSnapshot};
use meshway_model::{NodeId, RequestId};
pub use meshway_policy::admission::Carrier;
use meshway_policy::admission::{admit_circuit_carrier, admit_circuit_handshake, circuit_permit};
use replay::{ReplayWindow, SendNonce};
use sha2::{Digest as _, Sha256};
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
    pub authorization: CircuitAuthorization,
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
    pub authorization: CircuitAuthorization,
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
    recovery: recovery::Recovery,
    session: session::Session,
}
impl Drop for Circuit {
    fn drop(&mut self) {
        if !matches!(self.state, State::Closed) && diagnostics::event() {
            tracing::debug!(operation = "drop", cid = ?self.context.circuit,
                authorization = ?self.context.authorization, established = self.state.established(),
                "Dataplane circuit dropped");
        }
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
        let permit = circuit_permit(cfg, context.authorization).map_err(|_| Error::Context)?;
        let now_ms = snapshot.now_ms().map_err(|_| Error::Authority)?;
        let permit_at = Instant::now()
            .checked_add(Duration::from_millis(
                permit.expires_at_ms.saturating_sub(now_ms),
            ))
            .ok_or(Error::Limits)?;
        let handshake_at = Instant::now().checked_add(timeout).ok_or(Error::Limits)?;
        let handshake_at_ms = now_ms
            .checked_add(u64::try_from(timeout.as_millis()).map_err(|_| Error::Limits)?)
            .ok_or(Error::Limits)?;
        Ok(Self {
            context,
            state,
            expires_at: snapshot.expires_at().min(permit_at),
            expires_at_ms: snapshot.expires_at_ms().min(permit.expires_at_ms),
            handshake_at,
            handshake_at_ms,
            permit_expires_at_ms: permit.expires_at_ms,
            send: Default::default(),
            receive: Default::default(),
            cancellation,
            recovery: Default::default(),
            session: session::Session::new()?,
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

    /// Lost snapshot authority stops this operation, but is not a cryptographic
    /// close. Only Control's checked successor can make retained state usable.
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
                authorization = ?self.context.authorization, established = self.state.established(),
                cancelled = self.cancellation.is_cancelled(),
                expires_at_ms = self.expires_at_ms,
                remaining_ms = self.expires_at.saturating_duration_since(Instant::now()).as_millis() as u64,
                handshake_expires_at_ms = self.handshake_at_ms,
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
                cid = ?self.context.circuit, authorization = ?self.context.authorization,
                "Dataplane engine admission rejected");
        }
        Error::Packet
    }

    // Moving the Noise state out is not a lifecycle transition. Only a failed
    // completion retires the circuit; success keeps every issued guard live.
    fn complete_handshake(
        &mut self,
        finish: impl FnOnce(Box<snow::HandshakeState>) -> Result<snow::StatelessTransportState, Error>,
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
                self.state = State::Endpoint(Box::new(noise));
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
/// Failed/closed circuits retain bounded tombstones for this owner's lifetime.
/// These count towards capacity; future permit extensions cannot make an old
/// handshake new again. Replacing the owner establishes a new replay domain.
pub struct SecureNode {
    identity: DataPlaneIdentity,
    engine: Engine,
    limits: Limits,
    carriers: BTreeMap<NodeId, Carrier>,
    // Packet operations temporarily extract entries to borrow the one Engine.
    // Keep keys/Noise/recovery state at a stable address instead of moving a
    // multi-kilobyte Circuit (and reserving eleven such values per tree node).
    circuits: BTreeMap<[u8; 32], Box<Circuit>>,
    probes: probe::Probes,
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
    /// A bounded scheduling lookup. Session metadata comes only from this owner;
    /// packet authentication and replay admission still happen in receive().
    pub fn incoming_hint(&self, bytes: &[u8]) -> Result<UntrustedIncomingHint, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("codec.hint");
        let _profile = PROFILE.enter();
        let context = if let Some(id) = codec::session_id(bytes)? {
            let circuit = self.circuits.get(&id).ok_or(Error::State)?;
            circuit.ensure_current(self.identity.snapshot())?;
            &circuit.context
        } else {
            &Frame::decode(bytes, None)?.context
        };
        Ok(UntrustedIncomingHint {
            authorization: context.authorization,
            path: context.path.clone(),
        })
    }

    /// Replace host-observed authenticated carrier availability without changing
    /// circuit IDs, replay windows, flows or their complete logical paths.
    /// Every later output still checks this carrier against its specific permit.
    pub fn set_carriers(&mut self, carriers: BTreeMap<NodeId, Carrier>) -> Result<(), Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.set_carriers");
        let _profile = PROFILE.enter();
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
            probes: Default::default(),
            cancellation: CancellationToken::new(),
        })
    }

    pub fn start(&mut self, demand: RequestId, path: Vec<NodeId>) -> Result<PendingFrame, Error> {
        let carrier = self
            .carriers
            .get(path.get(1).ok_or(Error::Context)?)
            .ok_or(Error::Unsupported)?
            .clone();
        self.start_authorized(CircuitAuthorization::Business(demand), path, &carrier)
    }

    pub fn start_authorized(
        &mut self,
        authorization: CircuitAuthorization,
        path: Vec<NodeId>,
        carrier: &Carrier,
    ) -> Result<PendingFrame, Error> {
        self.maintain()?;
        self.capacity()?;
        let snapshot = self.identity.snapshot();
        let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
        if path.first() != Some(&cfg.node_id) {
            return Err(Error::Context);
        }
        authorization::source_instance(snapshot, authorization)?;
        if self.carriers.get(path.get(1).ok_or(Error::Context)?) != Some(carrier) {
            return Err(Error::Context);
        }
        let hop = admit_circuit_handshake(
            cfg,
            authorization,
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
        let permit = authorization::digest(cfg, authorization)?;
        let context = Context {
            network: cfg.network_id,
            authorization,
            keys: crypto::key_digest(cfg, &path)?,
            path,
            circuit: circuit_id,
            permit,
        };
        let mut noise = crypto::build(&self.identity, &context, true)?;
        let body = crypto::write_handshake(&mut noise, 96)?;
        snapshot.ensure_current().map_err(|_| Error::Authority)?;
        let mut circuit = Box::new(Circuit::new(
            context.clone(),
            State::Initiator(Box::new(noise)),
            snapshot,
            self.limits.handshake_timeout,
            self.cancellation.child_token(),
        )?);
        let frame = Frame {
            context,
            kind: Kind::Init,
            reply: false,
            sequence: 0,
            facts: None,
            probe: None,
            body,
            proofs: Vec::new(),
            exchange: None,
            auth: Default::default(),
        };
        circuit.session.remember_init(&frame)?;
        let pending = self.output(&circuit, frame, hop, None)?;
        pending.ensure_current()?;
        circuit.recovery.init = Some(recovery::Stage::new(
            Sha256::digest(&pending.bytes).into(),
            pending.guard.clone(),
            Some(&pending),
        )?);
        self.circuits.insert(circuit_id, circuit);
        Ok(pending)
    }

    /// Next paced retry of this origin's unchanged Noise INIT. No new CID,
    /// nonce, proof or authority is created. None means no retry is available;
    /// the host must still respect the circuit's original handshake deadline.
    pub fn handshake_retry_at(&self, id: [u8; 32]) -> Result<Option<Instant>, Error> {
        let circuit = self.circuits.get(&id).ok_or(Error::State)?;
        circuit.ensure_current(self.identity.snapshot())?;
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
        self.maintain_circuits()?;
        let mut circuit = self.circuits.remove(&id).ok_or(Error::State)?;
        let result = (|| {
            circuit.ensure_current(self.identity.snapshot())?;
            if !matches!(circuit.state, State::Initiator(_)) {
                return Ok(None);
            }
            let Some(stage) = circuit.recovery.init.as_mut() else {
                return Ok(None);
            };
            let output = stage.retry()?;
            if let Some(output) = &output {
                self.check_retry_carrier(output, &circuit.context)?;
            }
            Ok(output)
        })();
        let result = circuit.finish_operation(self.identity.snapshot(), "retry_handshake", result);
        self.circuits.insert(id, circuit);
        result
    }

    pub fn receive(&mut self, incoming: &Carrier, bytes: Vec<u8>) -> Result<Received, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.receive");
        let _profile = PROFILE.enter();
        self.maintain_circuits()?;
        let id = codec::session_id(&bytes)?;
        let retained = id
            .as_ref()
            .and_then(|id| self.circuits.get(id))
            .map(|circuit| &circuit.context);
        let frame = Frame::decode(&bytes, retained)?;
        let snapshot = self.identity.snapshot();
        let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
        let previous = if frame.kind.handshake() {
            crypto::verify(cfg, &frame, incoming)
        } else {
            let circuit = self
                .circuits
                .get(&frame.context.circuit)
                .ok_or(Error::State)?;
            circuit.ensure_current(snapshot)?;
            if circuit.context != frame.context {
                return Err(Error::Context);
            }
            circuit.session.verify(cfg, &frame, incoming)
        }
        .inspect_err(|error| {
            if diagnostics::event() {
                tracing::debug!(operation = "verify", ?error,
                    cid = ?frame.context.circuit, authorization = ?frame.context.authorization,
                    metadata_authenticated = false, "Dataplane secure admission rejected");
            }
        })?;
        snapshot.ensure_current().map_err(|_| Error::Authority)?;
        let id = frame.context.circuit;
        if let Some(mut circuit) = self.circuits.remove(&id) {
            // Always restore retained state on rejection. A consumed nonce or
            // handshake must never roll back as a side effect of an output error.
            let result = self.receive_known(&mut circuit, frame, previous);
            let result = circuit.finish_operation(self.identity.snapshot(), "receive", result);
            if let Err(error) = &result
                && diagnostics::event()
            {
                tracing::debug!(operation = "receive", ?error, cid = ?id,
                    authorization = ?circuit.context.authorization,
                    publication = %self.identity.snapshot().reference().execution_publication,
                    "Dataplane secure admission rejected");
            }
            self.circuits.insert(id, circuit);
            result
        } else {
            if frame.kind != Kind::Init {
                if diagnostics::event() {
                    tracing::debug!(operation = "receive", cause = "unknown_circuit", error = ?Error::State,
                        cid = ?id, authorization = ?frame.context.authorization,
                        "Dataplane secure admission rejected");
                }
                return Err(Error::State);
            }
            self.receive_init(frame, previous)
        }
    }

    fn receive_init(&mut self, frame: Frame, previous: NodeId) -> Result<Received, Error> {
        self.capacity()?;
        let snapshot = self.identity.snapshot();
        let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
        let permit = authorization::digest(cfg, frame.context.authorization)?;
        if permit != frame.context.permit
            || crypto::key_digest(cfg, &frame.context.path)? != frame.context.keys
        {
            return Err(Error::Context);
        }
        let hop = self.handshake_hop(&frame, Some(previous))?;
        let mut circuit = Box::new(Circuit::new(
            frame.context.clone(),
            State::RelayAwaiting,
            snapshot,
            self.limits.handshake_timeout,
            self.cancellation.child_token(),
        )?);
        let id = circuit.context.circuit;
        let fingerprint = Sha256::digest(frame.encode()?).into();
        let result = (|| {
            circuit.session.remember_init(&frame)?;
            let result = match hop {
                Hop::Forward(_) => Received::Forward(self.output(&circuit, frame, hop, None)?),
                Hop::Deliver | Hop::Exit => {
                    let expected = crypto::keys(cfg, frame.context.path[0])?.1.public_key.bytes;
                    let completed = (|| {
                        let mut noise = crypto::build(&self.identity, &frame.context, false)?;
                        crypto::read_handshake(&mut noise, &frame.body, 96, expected.as_bytes())?;
                        let body = crypto::write_handshake(&mut noise, 48)?;
                        let noise = noise
                            .into_stateless_transport_mode()
                            .map_err(|_| Error::Crypto)?;
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
                        facts: None,
                        probe: None,
                        body,
                        proofs: Vec::new(),
                        exchange: Some(circuit.session.exchange()?),
                        auth: Default::default(),
                    };
                    let keys = circuit.session.derive(&response, cfg.node_id)?;
                    circuit.session.establish(keys);
                    let hop = self.handshake_hop(&response, None)?;
                    circuit.state = State::Endpoint(Box::new(noise));
                    let response = self.output(&circuit, response, hop, None)?;
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
                circuit.guard(snapshot, true),
                Some(output),
            )?);
            circuit.guard(snapshot, true).ensure_current()?;
            Ok(result)
        })();
        if result.is_err() {
            // Once this admitted INIT has been consumed, even a response output
            // or final authority failure must not allow a second Noise owner for
            // the same CID. Keep the responder failure tombstone.
            circuit.retire_for("handshake", "initial_output_failed");
        }
        self.circuits.insert(id, circuit);
        result
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
            Kind::Init => {
                self.handshake_hop(&frame, Some(previous))?;
                self.repeat_handshake(circuit, &frame)
            }
            Kind::Response => {
                let hop = self.handshake_hop(&frame, Some(previous))?;
                let fingerprint = Sha256::digest(frame.encode()?).into();
                match &circuit.state {
                    State::RelayAwaiting => {
                        let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
                        let keys = circuit.session.derive(&frame, cfg.node_id)?;
                        let output = self.output(circuit, frame, hop, None)?;
                        circuit.guard(snapshot, true).ensure_current()?;
                        circuit.recovery.response = Some(recovery::Stage::new(
                            fingerprint,
                            circuit.guard(snapshot, true),
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
                        let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
                        let keys = circuit.session.derive(&frame, cfg.node_id)?;
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
            Kind::Data => self.receive_data(circuit, frame, previous),
            Kind::ProbeRequest | Kind::ProbeReply => self.receive_probe(circuit, frame, previous),
        }
    }

    fn repeat_handshake(&self, circuit: &mut Circuit, frame: &Frame) -> Result<Received, Error> {
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
            self.check_retry_carrier(output, &circuit.context)?;
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

    fn check_retry_carrier(&self, output: &PendingFrame, context: &Context) -> Result<(), Error> {
        output.ensure_current()?;
        if self.carriers.get(&output.next) != Some(&output.carrier) {
            return Err(Error::Context);
        }
        admit_circuit_carrier(
            self.identity
                .snapshot()
                .configuration()
                .map_err(|_| Error::Authority)?,
            context.authorization,
            &context.path,
            output.next,
            &output.carrier,
        )
        .map_err(|_| Error::Policy)
    }

    fn receive_data(
        &mut self,
        circuit: &mut Circuit,
        frame: Frame,
        previous: NodeId,
    ) -> Result<Received, Error> {
        let CircuitAuthorization::Business(demand) = circuit.context.authorization else {
            return Err(Error::Policy);
        };
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
            demand,
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
                    .admit_circuit_facts(ingress, packet.facts(), circuit.context.circuit)
                    .map_err(|error| circuit.engine_rejected("receive_endpoint", error))?;
                (route, Some(packet))
            }
            State::RelayEstablished => (
                self.engine
                    .admit_circuit_facts(ingress, facts, circuit.context.circuit)
                    .map_err(|error| circuit.engine_rejected("receive_relay", error))?,
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
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.send");
        let _profile = PROFILE.enter();
        self.maintain_circuits()?;
        let mut circuit = self.circuits.remove(&id).ok_or(Error::State)?;
        let result = self.send_inner(&mut circuit, packet);
        let result = circuit.finish_operation(self.identity.snapshot(), "send", result);
        if let Err(error) = &result
            && diagnostics::event()
        {
            tracing::debug!(operation = "send", ?error, cid = ?id,
                authorization = ?circuit.context.authorization,
                publication = %self.identity.snapshot().reference().execution_publication,
                "Dataplane secure admission rejected");
        }
        self.circuits.insert(id, circuit);
        result
    }

    fn send_inner(&mut self, circuit: &mut Circuit, packet: Packet) -> Result<PendingFrame, Error> {
        let CircuitAuthorization::Business(demand) = circuit.context.authorization else {
            return Err(Error::Policy);
        };
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
            demand,
            path: circuit.context.path.clone(),
        };
        let route = self
            .engine
            .admit_circuit_facts(ingress, facts, circuit.context.circuit)
            .map_err(|error| circuit.engine_rejected("send", error))?;
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
            probe: None,
            body,
            proofs: Vec::new(),
            exchange: None,
            auth: Default::default(),
        };
        self.output(circuit, frame, route.hop(), Some(&route))
    }

    fn handshake_hop(&self, frame: &Frame, previous: Option<NodeId>) -> Result<Hop, Error> {
        let snapshot = self.identity.snapshot();
        authorization::source_instance(snapshot, frame.context.authorization)?;
        admit_circuit_handshake(
            snapshot.configuration().map_err(|_| Error::Authority)?,
            frame.context.authorization,
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
            admit_circuit_carrier(cfg, context.authorization, &context.path, *next, carrier)
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
        admit_circuit_carrier(
            snapshot.configuration().map_err(|_| Error::Authority)?,
            frame.context.authorization,
            &frame.context.path,
            next,
            &carrier,
        )
        .map_err(|_| Error::Policy)?;
        let mut guard = circuit.guard(snapshot, matches!(frame.kind, Kind::Init | Kind::Response));
        if let Some(probe) = frame.probe {
            guard.expires_at_ms = guard.expires_at_ms.min(probe.expires_at_ms);
        }
        if let Some(route) = route {
            route.ensure_current().map_err(|_| Error::Authority)?;
            guard.snapshot = route.snapshot().clone();
            guard.expires_at = guard.expires_at.min(route.expires_at());
            guard.expires_at_ms = guard.expires_at_ms.min(route.permit_expires_at_ms());
        }
        guard.ensure_current()?;
        if frame.kind.handshake() {
            crypto::append(
                &self.identity,
                &mut frame,
                carrier.clone(),
                circuit.session.public(),
            )?;
        } else {
            circuit.session.append(&mut frame, carrier.clone())?;
        }
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
            let compatible = authorization::unchanged(
                old,
                new,
                circuit.context.authorization,
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
            let expiry = circuit_permit(new, circuit.context.authorization)
                .ok()
                .map(|permit| permit.expires_at_ms);
            let cause = if !compatible {
                "authorization_changed"
            } else if matches!(circuit.state, State::Closed) {
                "already_closed"
            } else if now >= circuit.expires_at {
                "monotonic_deadline"
            } else if now_ms >= circuit.expires_at_ms {
                "utc_deadline"
            } else if !live && circuit.cancellation.is_cancelled() {
                "cancelled"
            } else {
                "handshake_deadline"
            };
            plan.push((*id, live, expiry, cause));
        }
        let final_now_ms = snapshot.now_ms().map_err(|_| Error::Authority)?;
        snapshot
            .continuation_from(self.identity.snapshot())
            .map_err(|_| Error::Authority)?;
        for (id, live, expiry, cause) in plan {
            let circuit = self.circuits.get_mut(&id).expect("planned circuit");
            circuit.recovery.maintain();
            if let Some(expiry) = expiry {
                circuit.permit_expires_at_ms = circuit.permit_expires_at_ms.max(expiry);
            }
            if live && Instant::now() < circuit.expires_at && final_now_ms < circuit.expires_at_ms {
                let permit_at = Instant::now()
                    .checked_add(Duration::from_millis(
                        expiry.expect("live permit").saturating_sub(final_now_ms),
                    ))
                    .unwrap_or(snapshot.expires_at());
                circuit.expires_at = snapshot.expires_at().min(permit_at);
                circuit.expires_at_ms = snapshot.expires_at_ms().min(expiry.expect("live permit"));
            } else {
                let cause = if !live {
                    cause
                } else {
                    "deadline_during_reconcile"
                };
                circuit.retire_for("reconcile", cause);
            }
        }
        self.engine.flows = flows;
        self.engine.snapshot = snapshot.clone();
        self.identity = identity;
        Ok(())
    }

    /// Host-driven cleanup; expired states become bounded replay tombstones.
    pub fn maintain(&mut self) -> Result<(), Error> {
        self.maintain_circuits()?;
        self.engine.maintain().map_err(|_| Error::Authority)
    }

    // Packet admission collects expired flows with its final FlowTime. Avoid
    // sweeping that same table again here, while retaining circuit fencing.
    fn maintain_circuits(&mut self) -> Result<(), Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.maintain");
        let _profile = PROFILE.enter();
        self.probes.maintain();
        if self.identity.snapshot().ensure_current().is_err() {
            return Err(Error::Authority);
        }
        self.identity
            .snapshot()
            .now_ms()
            .map_err(|_| Error::Authority)?;
        for circuit in self.circuits.values_mut() {
            circuit.recovery.maintain();
            if circuit.validate_operation(self.identity.snapshot(), "maintain")
                == Err(Error::Authority)
            {
                return Err(Error::Authority);
            }
        }
        // Keep bounded replay tombstones for this owner. A later authenticated
        // extension/regrant of the same authorization must not accept an old IK
        // initiation after its original absolute expiry. Capacity fails closed.
        // Retain the shared Engine's authority/clock boundary without its
        // redundant flow collection; prepare will collect with final FlowTime.
        self.engine
            .snapshot
            .now_ms()
            .map_err(|_| Error::Authority)?;
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
            authorization: circuit.context.authorization,
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

    /// Select a local output from the unique flow owner's authenticated cipher
    /// binding. A terminal must never guess among locally completed handshakes:
    /// the origin may not have received one of their responses. Only an origin
    /// can acquire another cipher for its unchanged route after the old one ends.
    /// Reading this does not advance any flow, nonce or deadline; send rechecks.
    pub fn local_circuit(
        &self,
        demand: RequestId,
        path: &[NodeId],
        packet: &Packet,
    ) -> Result<Option<CircuitInfo>, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.local_circuit");
        let _profile = PROFILE.enter();
        let cfg = self
            .identity
            .snapshot()
            .configuration()
            .map_err(|_| Error::Authority)?;
        let origin = path.first() == Some(&cfg.node_id);
        if let Some(flow) = self.local_flow(packet)? {
            if flow.demand != demand || flow.path != path {
                return Err(Error::Flow);
            }
            if let Some(id) = flow.circuit
                && let Some(info) = self.circuit_info(id)?
                && info.state != CircuitState::Closed
            {
                if info.authorization != CircuitAuthorization::Business(demand) || info.path != path
                {
                    return Err(Error::Context);
                }
                return Ok((info.state == CircuitState::Established).then_some(info));
            }
        }
        if !origin {
            return Ok(None);
        }
        // Filter immutable bindings before checking liveness or allocating
        // metadata. Unrelated circuits cannot supply this origin's output.
        for (id, circuit) in &self.circuits {
            if circuit.context.authorization == CircuitAuthorization::Business(demand)
                && circuit.context.path == path
                && circuit.state.established()
                && let Some(info) = self.circuit_info(*id)?
                && info.state == CircuitState::Established
            {
                return Ok(Some(info));
            }
        }
        Ok(None)
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
            circuit.retire_for("invalidated_wait", "guard_woke");
        }
        self.engine.flows.clear();
    }
}

#[cfg(test)]
mod tests;
