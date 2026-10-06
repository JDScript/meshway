//! Shared L3 admission, bounded flow ownership and protected packet transport.
//!
//! `meshway-policy` is the only grant/path/address admission implementation;
//! `meshway-control::ExecutionSnapshot` is the only execution authority. This
//! The core parses packets and associates replies with previously admitted
//! forward facts. The secure adapter authenticates metadata before using that
//! same core; relays need no plaintext. Neither core admission nor authenticated
//! metadata substitutes for checking decrypted packet facts at an endpoint.
//!
//! Peer ingress is a trusted-adapter contract for the cryptographic layer,
//! **not** a decoder for peer-supplied identity claims. In particular plaintext
//! transport frames must not be passed off as authenticated end-to-end traffic.
#![forbid(unsafe_code)]

mod diagnostics;
#[cfg(test)]
mod fixtures;
mod flow;
mod packet;

use flow::{CommittedRoute, FlowTable, FlowTime};
use meshway_control::ExecutionSnapshot;
use meshway_model::{NodeId, RequestId};
use meshway_policy::admission::AdmissionError;
pub use meshway_policy::admission::Hop;
pub use packet::{Packet, PacketError, PacketFacts, TcpSegment, Transport};
use std::{
    fmt,
    time::{Duration, Instant},
};

/// Hard memory bounds; flow expiry never exceeds the immutable permit or the
/// current control guard. The idle duration is a state retention limit, not an
/// independent source of execution authority.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_flows: usize,
    pub idle_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_flows: 4096,
            idle_timeout: Duration::from_secs(60),
        }
    }
}

impl Limits {
    fn validate(self) -> Result<Self, Error> {
        if self.max_flows == 0
            || self.max_flows > 65_536
            || self.idle_timeout.is_zero()
            || self.idle_timeout > Duration::from_secs(86_400)
        {
            return Err(Error::Limits);
        }
        Ok(self)
    }
}

/// Identity facts already verified by the trusted cryptographic adapter.
///
/// `origin` is the original business source bound into the protected full-path
/// context, including on replies. `sender` is the authenticated end-to-end
/// producer of this packet (origin for forwards, terminal node for replies).
/// `previous` is the authenticated immediately adjacent transport peer.
/// None of these fields is a bearer credential or a claim to read from plaintext.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthenticatedPeer {
    pub origin: NodeId,
    pub sender: NodeId,
    pub previous: NodeId,
}

/// Both directions name the original full forward path. There is intentionally
/// no caller-controlled reply flag or caller-supplied forward traffic tuple.
#[derive(Debug)]
pub enum Ingress {
    Local {
        demand: RequestId,
        path: Vec<NodeId>,
    },
    Peer {
        demand: RequestId,
        path: Vec<NodeId>,
        identity: AuthenticatedPeer,
    },
}

impl Ingress {
    fn demand(&self) -> RequestId {
        match self {
            Self::Local { demand, .. } | Self::Peer { demand, .. } => *demand,
        }
    }
    fn path(&self) -> &[NodeId] {
        match self {
            Self::Local { path, .. } | Self::Peer { path, .. } => path,
        }
    }
    fn origin(&self) -> Option<NodeId> {
        match self {
            Self::Local { path, .. } => path.first().copied(),
            Self::Peer { identity, .. } => Some(identity.origin),
        }
    }
    fn previous(&self) -> Option<NodeId> {
        match self {
            Self::Local { .. } => None,
            Self::Peer { identity, .. } => Some(identity.previous),
        }
    }
    fn sender(&self, local: NodeId) -> NodeId {
        match self {
            Self::Local { .. } => local,
            Self::Peer { identity, .. } => identity.sender,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Reply,
}

/// Output context derived from the admitted flow, never reverse-recompiled.
#[derive(Debug)]
pub struct Route {
    pub demand: RequestId,
    pub origin: NodeId,
    pub original_path: Vec<NodeId>,
    pub direction: Direction,
}

/// Original local-flow routing metadata, not a grant or a circuit selection.
/// A sender must still perform normal current admission on the selected circuit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalFlow {
    pub demand: RequestId,
    pub path: Vec<NodeId>,
    /// Cipher context of the last admitted forward packet, when this flow is
    /// owned by SecureNode. Plaintext Engine consumers have no cipher context.
    pub circuit: Option<[u8; 32]>,
}

#[derive(Debug)]
pub enum Action {
    Forward {
        next: NodeId,
        packet: Packet,
        route: Route,
    },
    Deliver {
        packet: Packet,
        route: Route,
    },
    Exit {
        packet: Packet,
        route: Route,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid packet facts: {0}")]
    Packet(#[from] PacketError),
    #[error("packet does not match the admitted facts")]
    FactsMismatch,
    #[error("execution authority is no longer current: {0}")]
    Control(#[from] meshway_control::Error),
    #[error("packet admission rejected: {0}")]
    Admission(#[from] AdmissionError),
    #[error("invalid flow limits")]
    Limits,
    #[error("invalid or unsupported ingress context")]
    Ingress,
    #[error("authenticated packet producer does not match the flow endpoint")]
    Sender,
    #[error("packet does not qualify for the current flow/protocol state")]
    Flow,
    #[error("flow table capacity reached")]
    Capacity,
    #[error("pending packet exceeded its flow/permit lifetime")]
    Expired,
    #[error("execution refresh does not match the current control scope")]
    ExecutionScope,
}

/// Owns the action until the same control guard and packet lifetime are checked
/// at delivery. A delayed queue must retain this wrapper, not a released Action.
pub struct PendingAction {
    pending: PendingRoute,
    packet: Packet,
}

/// The same checked route and immutable lifetime for plaintext and encrypted
/// admission. It contains no plaintext and cannot release an unchecked packet.
pub(crate) struct PendingRoute {
    snapshot: ExecutionSnapshot,
    expires_at: Instant,
    permit_expires_at_ms: u64,
    checked: CommittedRoute,
    facts: PacketFacts,
    cancellation: Option<tokio_util::sync::CancellationToken>,
}

impl fmt::Debug for PendingRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingRoute")
            .field("expires_at", &self.expires_at)
            .field("checked", &self.checked)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for PendingAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingAction")
            .field("expires_at", &self.pending.expires_at)
            .field("route", &self.pending.checked)
            .field("packet", &self.packet)
            .finish_non_exhaustive()
    }
}

impl PendingAction {
    /// Scheduling metadata only. The packet remains guarded while the host
    /// waits for the selected sink to become writable.
    pub fn hop(&self) -> Hop {
        self.pending.hop()
    }

    pub fn route(&self) -> &Route {
        self.pending.route()
    }

    pub fn ensure_current(&self) -> Result<(), Error> {
        self.pending.ensure_current()
    }

    /// Invoke the final synchronous sink immediately after validation. The sink
    /// must not queue the released action for a later write. Async I/O adapters
    /// must retain the wrapper while waiting for readiness, then release at the
    /// write boundary; invalidation also needs to cancel their own pending I/O.
    pub fn release<T>(self, sink: impl FnOnce(Action) -> T) -> Result<T, Error> {
        self.ensure_current()?;
        let route = self.pending.checked.route;
        let packet = self.packet;
        let action = match self.pending.checked.hop {
            Hop::Forward(next) => Action::Forward {
                next,
                packet,
                route,
            },
            Hop::Deliver => Action::Deliver { packet, route },
            Hop::Exit => Action::Exit { packet, route },
        };
        Ok(sink(action))
    }

    /// Wait for the original guard or this packet's shorter retention deadline.
    /// A refresh of the Engine cannot extend either deadline on this action.
    /// Permit-clock changes use the control owner's same notification source.
    pub async fn invalidated(&self) {
        self.pending.invalidated().await;
    }
}

impl PendingRoute {
    pub(crate) fn hop(&self) -> Hop {
        self.checked.hop
    }

    pub(crate) fn route(&self) -> &Route {
        &self.checked.route
    }

    pub(crate) fn snapshot(&self) -> &ExecutionSnapshot {
        &self.snapshot
    }

    pub(crate) fn expires_at(&self) -> Instant {
        self.expires_at
    }

    pub(crate) fn permit_expires_at_ms(&self) -> u64 {
        self.permit_expires_at_ms
    }

    pub(crate) fn ensure_current(&self) -> Result<(), Error> {
        if self
            .cancellation
            .as_ref()
            .is_some_and(|token| token.is_cancelled())
        {
            return Err(Error::Expired);
        }
        self.snapshot.ensure_current()?;
        if Instant::now() >= self.expires_at || self.snapshot.now_ms()? >= self.permit_expires_at_ms
        {
            return Err(Error::Expired);
        }
        self.snapshot.ensure_current()?;
        Ok(())
    }

    pub(crate) fn attach(self, packet: Packet) -> Result<PendingAction, Error> {
        self.facts.check_packet(&packet)?;
        self.ensure_current()?;
        Ok(PendingAction {
            pending: self,
            packet,
        })
    }

    pub(crate) fn constrain(
        &mut self,
        expires_at: Instant,
        expires_at_ms: u64,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<(), Error> {
        if self.cancellation.is_some() {
            return Err(Error::ExecutionScope);
        }
        self.expires_at = self.expires_at.min(expires_at);
        self.permit_expires_at_ms = self.permit_expires_at_ms.min(expires_at_ms);
        self.cancellation = Some(cancellation);
        self.ensure_current()
    }

    pub(crate) async fn invalidated(&self) {
        if self.ensure_current().is_err() {
            return;
        }
        tokio::select! { biased;
            _ = self.snapshot.invalidated_at(self.permit_expires_at_ms) => {},
            _ = tokio::time::sleep_until(self.expires_at.into()) => {},
            _ = async {
                match &self.cancellation {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending().await,
                }
            } => {},
        }
    }
}

/// Unique flow state owner. Only Control-authenticated direct successors may
/// carry compatible live state into a new immutable target.
pub struct Engine {
    snapshot: ExecutionSnapshot,
    flows: FlowTable,
}

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Engine")
            .field("retained_flows", &self.flows.len())
            .finish_non_exhaustive()
    }
}

impl Engine {
    pub fn new(snapshot: ExecutionSnapshot, limits: Limits) -> Result<Self, Error> {
        snapshot.configuration()?;
        Ok(Self {
            snapshot,
            flows: FlowTable::new(limits.validate()?),
        })
    }

    /// Same-target renewal, using the shared reconciliation implementation.
    pub fn refresh(&mut self, snapshot: ExecutionSnapshot) -> Result<(), Error> {
        if !self.snapshot.same_execution(&snapshot) {
            return Err(Error::ExecutionScope);
        }
        self.reconcile(snapshot)
    }

    /// Reconcile a Control-issued successor transactionally. Old queued actions
    /// keep their original cancelled guard. Retained flow deadlines/idle time
    /// are unchanged; only a subsequently admitted packet can refresh them.
    pub fn reconcile(&mut self, snapshot: ExecutionSnapshot) -> Result<(), Error> {
        let flows = self.reconciled_flows(&snapshot)?;
        snapshot.continuation_from(&self.snapshot)?;
        self.flows = flows;
        self.snapshot = snapshot;
        Ok(())
    }

    fn reconciled_flows(&self, snapshot: &ExecutionSnapshot) -> Result<FlowTable, Error> {
        let old = snapshot.continuation_from(&self.snapshot)?;
        let now_ms = snapshot.now_ms()?;
        self.flows.reconciled(
            old,
            snapshot.configuration()?,
            FlowTime {
                instant: Instant::now(),
                now_ms,
                guard_expires_at: snapshot.expires_at(),
                guard_expires_at_ms: snapshot.expires_at_ms(),
            },
        )
    }

    pub fn process(&mut self, ingress: Ingress, packet: Packet) -> Result<PendingAction, Error> {
        self.admit_facts(ingress, packet.facts())?.attach(packet)
    }

    /// Read the unique flow owner without advancing protocol state or deadlines.
    /// `None` means only that the packet could initiate a new protocol flow;
    /// it does not authorize its source, destination or a new demand.
    pub fn local_flow(&self, packet: &Packet) -> Result<Option<LocalFlow>, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("flow.lookup");
        let _profile = PROFILE.enter();
        let config = self.snapshot.configuration()?;
        let found = self.flows.local_flow(
            config,
            &packet.facts(),
            FlowTime {
                instant: Instant::now(),
                now_ms: self.snapshot.now_ms()?,
                guard_expires_at: self.snapshot.expires_at(),
                guard_expires_at_ms: self.snapshot.expires_at_ms(),
            },
        )?;
        let now_ms = self.snapshot.now_ms()?;
        self.snapshot.ensure_current()?;
        match found {
            Some(found) if Instant::now() >= found.expires_at || now_ms >= found.expires_at_ms => {
                Err(Error::Expired)
            }
            Some(found) => Ok(Some(found.route)),
            None => Ok(None),
        }
    }

    /// The secure adapter must authenticate the complete facts/path/producer
    /// before calling this shared state-changing admission boundary.
    pub(crate) fn admit_facts(
        &mut self,
        ingress: Ingress,
        facts: PacketFacts,
    ) -> Result<PendingRoute, Error> {
        self.admit_inner(ingress, facts, None)
    }

    pub(crate) fn admit_circuit_facts(
        &mut self,
        ingress: Ingress,
        facts: PacketFacts,
        circuit: [u8; 32],
    ) -> Result<PendingRoute, Error> {
        self.admit_inner(ingress, facts, Some(circuit))
    }

    fn admit_inner(
        &mut self,
        ingress: Ingress,
        facts: PacketFacts,
        circuit: Option<[u8; 32]>,
    ) -> Result<PendingRoute, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("flow.admit");
        let _profile = PROFILE.enter();
        // Prepare collects expired entries once, with this admission's final
        // FlowTime. Authority checks remain independent of that collection.
        self.snapshot.ensure_current()?;
        let time = FlowTime {
            instant: Instant::now(),
            now_ms: self.snapshot.now_ms()?,
            guard_expires_at: self.snapshot.expires_at(),
            guard_expires_at_ms: self.snapshot.expires_at_ms(),
        };
        let mut prepared =
            self.flows
                .prepare(self.snapshot.configuration()?, &ingress, &facts, time)?;
        if let Some(circuit) = circuit {
            prepared.bind_circuit(circuit)?;
        }
        // A rejected/fenced transaction cannot establish or advance flow state.
        tracing::trace!(target: "meshway_dataplane::flow::authority_boundary", operation = "commit",
            "Validating flow admission authority");
        self.snapshot.ensure_current()?;
        if self.snapshot.now_ms()? >= prepared.permit_expires_at_ms
            || Instant::now() >= prepared.expires_at
        {
            return Err(Error::Expired);
        }
        let expires_at = prepared.expires_at;
        let permit_expires_at_ms = prepared.permit_expires_at_ms;
        // Clock reads can fence a guard; the final guard check precedes commit.
        self.snapshot.ensure_current()?;
        let checked = self.flows.commit(prepared);
        Ok(PendingRoute {
            snapshot: self.snapshot.clone(),
            expires_at,
            permit_expires_at_ms,
            checked,
            facts,
            cancellation: None,
        })
    }

    /// Host-driven idle cleanup. Reading counters never changes permits/state.
    /// Invalidated entries are unusable immediately. Authority loss retains inert
    /// state for a possible Control-approved successor; reconciliation checks
    /// every original expiry. Explicit invalidated().await/drop still tears down.
    pub fn maintain(&mut self) -> Result<(), Error> {
        self.snapshot.ensure_current()?;
        self.flows.expire(Instant::now(), self.snapshot.now_ms()?);
        Ok(())
    }

    /// Number of retained entries; deliberately does not refresh or expire them.
    pub fn retained_flows(&self) -> usize {
        self.flows.len()
    }

    /// Caller-driven cancellation hook; no spawned task or independent lease.
    pub async fn invalidated(&mut self) {
        self.snapshot.invalidated().await;
        self.flows.clear();
    }
}

pub mod carrier;
pub mod secure;
