//! Shared L3 admission, bounded flow ownership and protected packet transport.
//!
//! `meshway-policy` is the only grant/path/address admission implementation;
//! `meshway-control::ExecutionSnapshot` is the only execution authority. This
//! The core parses endpoint packets and associates replies with previously
//! admitted forward facts. The secure adapter authenticates and decrypts before
//! using that same core; relays authenticate only their circuit and neighbour.
//!
//! Peer ingress is a trusted-adapter contract for the cryptographic layer,
//! **not** a decoder for peer-supplied identity claims. In particular plaintext
//! transport frames must not be passed off as authenticated end-to-end traffic.
#![forbid(unsafe_code)]

mod admission;
mod buffer;
mod diagnostics;
mod engine;
pub mod exit;
#[cfg(test)]
mod fixtures;
mod flow;
mod packet;

pub use buffer::{BufferError, DEFAULT_HEADROOM, DEFAULT_TAILROOM, PacketBuffer, PacketPool};
use flow::{CommittedRoute, FlowTable};
use meshway_authority::{ExecutionSnapshot, Reading};
use meshway_model::{NodeId, RequestId};
use meshway_policy::admission::AdmissionError;
pub use meshway_policy::admission::Hop;
pub use packet::{Packet, PacketError, PacketFacts, PacketInput, TcpSegment, Transport};
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
    Authority(#[from] meshway_authority::Error),
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
    #[error("exit projection has not been acknowledged")]
    ProjectionPending,
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
    section: Option<std::sync::Arc<()>>,
    flow_cancellation: Option<tokio_util::sync::CancellationToken>,
    projection: Option<std::sync::Arc<exit::Gate>>,
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
        self.pending.ensure_current()?;
        if let Some(gate) = &self.pending.projection {
            gate.ready()?;
        }
        Ok(())
    }

    /// Await the Install/Renew ACK while retaining the original packet guard.
    /// The host drives `SecureNode::next_exit_projection` independently.
    pub async fn projection_ready(&self) -> Result<(), Error> {
        self.pending.ensure_current()?;
        if let Some(gate) = &self.pending.projection {
            tokio::select! { biased;
                _ = self.invalidated() => return Err(Error::Expired),
                result = gate.wait() => result?,
            }
        }
        self.ensure_current()
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
        self.check_at(&self.snapshot.reading()?)
    }

    pub(crate) fn attach(self, packet: Packet) -> Result<PendingAction, Error> {
        self.facts.check_packet(&packet)?;
        self.check_at(&self.snapshot.reading()?)?;
        Ok(PendingAction {
            pending: self,
            packet,
        })
    }

    pub(crate) fn constrain(
        &mut self,
        reading: &Reading<'_>,
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
        self.check_at(reading)
    }

    pub(crate) fn check_at(&self, reading: &Reading<'_>) -> Result<(), Error> {
        if self
            .flow_cancellation
            .as_ref()
            .is_some_and(|token| token.is_cancelled())
            || self
                .cancellation
                .as_ref()
                .is_some_and(|token| token.is_cancelled())
            || reading.instant >= self.expires_at
            || reading.now_ms >= self.permit_expires_at_ms
        {
            return Err(Error::Expired);
        }
        Ok(())
    }

    pub(crate) async fn invalidated(&self) {
        if self.ensure_current().is_err() {
            return;
        }
        tokio::select! { biased;
            _ = self.snapshot.invalidated_at(self.permit_expires_at_ms) => {},
            _ = tokio::time::sleep_until(self.expires_at.into()) => {},
            _ = async {
                match &self.flow_cancellation {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending().await,
                }
            } => {},
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
    exit_counters: exit::Counters,
}

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Engine")
            .field("retained_flows", &self.flows.len())
            .finish_non_exhaustive()
    }
}

pub mod carrier;
pub mod secure;
