//! Shared plaintext L3 admission and bounded flow ownership.
//!
//! `meshway-policy` is the only grant/path/address admission implementation;
//! `meshway-control::ExecutionSnapshot` is the only execution authority. This
//! crate parses packets and associates replies with previously admitted forward
//! facts. It implements no encryption, authentication handshake, wire envelope,
//! I/O, NAT, TCP stack, lease renewal or background task.
//!
//! Peer ingress is a trusted-adapter contract for a future cryptographic layer,
//! **not** a decoder for peer-supplied identity claims. In particular plaintext
//! transport frames must not be passed off as authenticated end-to-end traffic.
#![forbid(unsafe_code)]

#[cfg(test)]
mod fixtures;
mod flow;
mod packet;

use flow::{FlowTable, FlowTime};
use meshway_control::ExecutionSnapshot;
use meshway_model::{NodeId, RequestId};
use meshway_policy::admission::AdmissionError;
pub use meshway_policy::admission::Hop;
pub use packet::{Packet, PacketError, TcpSegment, Transport};
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
    snapshot: ExecutionSnapshot,
    expires_at: Instant,
    permit_expires_at_ms: u64,
    action: Action,
}

impl fmt::Debug for PendingAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingAction")
            .field("expires_at", &self.expires_at)
            .field("action", &self.action)
            .finish_non_exhaustive()
    }
}

impl PendingAction {
    /// Scheduling metadata only. The packet remains guarded while the host
    /// waits for the selected sink to become writable.
    pub fn hop(&self) -> Hop {
        match &self.action {
            Action::Forward { next, .. } => Hop::Forward(*next),
            Action::Deliver { .. } => Hop::Deliver,
            Action::Exit { .. } => Hop::Exit,
        }
    }

    pub fn route(&self) -> &Route {
        match &self.action {
            Action::Forward { route, .. }
            | Action::Deliver { route, .. }
            | Action::Exit { route, .. } => route,
        }
    }

    pub fn ensure_current(&self) -> Result<(), Error> {
        self.snapshot.ensure_current()?;
        if Instant::now() >= self.expires_at || self.snapshot.now_ms()? >= self.permit_expires_at_ms
        {
            return Err(Error::Expired);
        }
        Ok(())
    }

    /// Invoke the final synchronous sink immediately after validation. The sink
    /// must not queue the released action for a later write. Async I/O adapters
    /// must retain the wrapper while waiting for readiness, then release at the
    /// write boundary; invalidation also needs to cancel their own pending I/O.
    pub fn release<T>(self, sink: impl FnOnce(Action) -> T) -> Result<T, Error> {
        self.ensure_current()?;
        Ok(sink(self.action))
    }

    /// Wait for the original guard or this packet's shorter retention deadline.
    /// A refresh of the Engine cannot extend either deadline on this action.
    /// Permit-clock changes use the control owner's same notification source.
    pub async fn invalidated(&self) {
        if self.ensure_current().is_err() {
            return;
        }
        tokio::select! { biased;
            _ = self.snapshot.invalidated_at(self.permit_expires_at_ms) => {},
            _ = tokio::time::sleep_until(self.expires_at.into()) => {},
        }
    }
}

/// Unique state owner for one immutable control target. Construct a new Engine
/// for a replacement snapshot; no flows migrate across publications or targets.
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

    /// Explicit same-target authentication/lease renewal handoff. Both guards
    /// must still be current and share Control's exact execution scope. A new
    /// target, instance, generation or invalidation epoch requires a new Engine.
    /// Rejection leaves this Engine's guard and retained state unchanged.
    ///
    /// Existing flow deadlines are retained conservatively; only a subsequently
    /// admitted packet can refresh a live flow against the new guard. Expired
    /// flows are removed, never revived. Already-issued PendingActions retain
    /// their original snapshots and deadlines.
    pub fn refresh(&mut self, snapshot: ExecutionSnapshot) -> Result<(), Error> {
        snapshot.ensure_current()?;
        self.snapshot.ensure_current()?;
        if !self.snapshot.same_execution(&snapshot) {
            return Err(Error::ExecutionScope);
        }
        let now_ms = snapshot.now_ms()?;
        // Reading the shared clock may itself fence an older deadline.
        self.snapshot.ensure_current()?;
        snapshot.ensure_current()?;
        self.flows.expire(Instant::now(), now_ms);
        self.snapshot = snapshot;
        Ok(())
    }

    pub fn process(&mut self, ingress: Ingress, packet: Packet) -> Result<PendingAction, Error> {
        self.maintain()?;
        let time = FlowTime {
            instant: Instant::now(),
            now_ms: self.snapshot.now_ms()?,
            guard_expires_at: self.snapshot.expires_at(),
            guard_expires_at_ms: self.snapshot.expires_at_ms(),
        };
        let prepared =
            self.flows
                .prepare(self.snapshot.configuration()?, &ingress, &packet, time)?;
        // A rejected/fenced transaction cannot establish or advance flow state.
        if let Err(error) = self.snapshot.ensure_current() {
            self.flows.clear();
            return Err(error.into());
        }
        if self.snapshot.now_ms()? >= prepared.permit_expires_at_ms
            || Instant::now() >= prepared.expires_at
        {
            return Err(Error::Expired);
        }
        let expires_at = prepared.expires_at;
        let permit_expires_at_ms = prepared.permit_expires_at_ms;
        let action = self.flows.commit(prepared, packet);
        Ok(PendingAction {
            snapshot: self.snapshot.clone(),
            expires_at,
            permit_expires_at_ms,
            action,
        })
    }

    /// Host-driven idle cleanup. Reading counters never changes permits/state.
    /// Invalidated entries are logically unusable immediately; physical removal
    /// occurs here, on process, on invalidated().await, or when Engine is dropped.
    pub fn maintain(&mut self) -> Result<(), Error> {
        if let Err(error) = self.snapshot.ensure_current() {
            self.flows.clear();
            return Err(error.into());
        }
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
