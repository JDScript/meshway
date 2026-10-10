//! Shared L3 admission, bounded flow ownership and protected packet transport.
//!
//! `meshway-policy` is the only grant/path/address admission implementation;
//! Engine's desired netmap is the only content it admits on. The core parses
//! endpoint packets and associates replies with previously
//! admitted forward facts. The secure adapter authenticates and decrypts before
//! using that same core; relays authenticate only their circuit and neighbour.
//!
//! Peer ingress is a trusted-adapter contract for the cryptographic layer,
//! **not** a decoder for peer-supplied identity claims. In particular plaintext
//! transport frames must not be passed off as authenticated end-to-end traffic.
#![forbid(unsafe_code)]

mod admission;
mod authority;
mod buffer;
mod diagnostics;
mod engine;
pub mod exit;
#[cfg(test)]
mod fixtures;
mod flow;
mod netmap;
mod packet;

use authority::{Lifetime, View};
pub use buffer::{BufferError, DEFAULT_HEADROOM, DEFAULT_TAILROOM, PacketBuffer, PacketPool};
use flow::{CommittedRoute, FlowTable};
use meshway_engine::Desired;
use meshway_model::NodeId;
use meshway_policy::admission::AdmissionError;
pub use meshway_policy::admission::Hop;
pub use meshway_policy::compile::netmap::Target;
pub use netmap::Purpose;
pub use packet::{Packet, PacketError, PacketFacts, PacketInput, TcpSegment, Transport};
use std::{fmt, time::Duration};

/// Hard memory bounds. The idle duration is a state retention limit, not an
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

/// Both directions name the (origin = `path[0]`, target) pair and the
/// original full forward path. There is intentionally no caller-controlled
/// reply flag or caller-supplied forward traffic tuple.
#[derive(Debug)]
pub enum Ingress {
    Local {
        target: Target,
        path: Vec<NodeId>,
    },
    Peer {
        target: Target,
        path: Vec<NodeId>,
        identity: AuthenticatedPeer,
    },
}

impl Ingress {
    fn entry(&self, local: NodeId) -> Result<flow::Entry<'_>, Error> {
        Ok(match self {
            Self::Local { target, path } => flow::Entry {
                grant: target.clone(),
                path,
                origin: *path.first().ok_or(Error::Ingress)?,
                previous: None,
                sender: local,
            },
            Self::Peer {
                target,
                path,
                identity,
            } => flow::Entry {
                grant: target.clone(),
                path,
                origin: identity.origin,
                previous: Some(identity.previous),
                sender: identity.sender,
            },
        })
    }
}

impl LocalFlow {
    pub(crate) fn new(grant: Target, path: Vec<NodeId>, circuit: Option<[u8; 32]>) -> Self {
        Self {
            grant,
            path,
            circuit,
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
    /// Target of the flow's (origin, target) pair.
    pub grant: Target,
    pub origin: NodeId,
    pub original_path: Vec<NodeId>,
    pub direction: Direction,
}

/// Original local-flow routing metadata, not a grant or a circuit selection.
/// A sender must still perform normal current admission on the selected circuit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalFlow {
    /// Target of the flow's (origin, target) pair.
    pub grant: Target,
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
    #[error("execution authority is no longer current")]
    Authority,
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
    #[error("route was already bound to its circuit")]
    ExecutionScope,
    #[error("exit projection has not been acknowledged")]
    ProjectionPending,
}

/// Owns the action until its packet lifetime is checked at delivery. A delayed
/// queue must retain this wrapper, not a released Action.
pub struct PendingAction {
    pending: PendingRoute,
    packet: Packet,
}

/// The same checked route and immutable lifetime for plaintext and encrypted
/// admission. It contains no plaintext and cannot release an unchecked packet.
pub(crate) struct PendingRoute {
    /// Flow cap and cancellation; the circuit's is added by `constrain`.
    life: Lifetime,
    constrained: bool,
    checked: CommittedRoute,
    facts: PacketFacts,
    projection: Option<std::sync::Arc<exit::Gate>>,
    exit_incarnation: Option<exit::FlowIncarnation>,
}

impl fmt::Debug for PendingRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingRoute")
            .field("cap", &self.life.cap)
            .field("checked", &self.checked)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for PendingAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingAction")
            .field("cap", &self.pending.life.cap)
            .field("route", &self.pending.checked)
            .field("packet", &self.packet)
            .finish_non_exhaustive()
    }
}

impl PendingAction {
    /// Scheduling metadata only. The packet keeps its lifetime while the host
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

    /// Original per-packet lifetime for the host's guarded exit-TUN I/O adapter.
    /// Obtain only after projection ACK. The adapter must retain this scope and
    /// recheck it at every write poll; a later flow projection cannot replace it.
    pub fn exit_scope(&self) -> Result<exit::ProjectionScope, Error> {
        if self.hop() != Hop::Exit {
            return Err(Error::Ingress);
        }
        self.ensure_current()?;
        Ok(exit::ProjectionScope::new(secure::OutputBinding::from_route(
            &self.pending,
        )))
    }

    /// Opaque kernel projection identity for this exact admitted exit packet.
    /// Scheduling metadata only; `exit_scope`/`release` remain the output fence.
    pub fn exit_flow(&self) -> Option<exit::FlowIncarnation> {
        (self.hop() == Hop::Exit)
            .then_some(self.pending.exit_incarnation)
            .flatten()
    }

    /// Await the Install/Renew ACK while retaining the original packet lifetime.
    /// The host drives `SecureNode::next_exit_projection` independently.
    pub async fn projection_ready(&self) -> Result<(), Error> {
        self.pending.ensure_current()?;
        if let Some(gate) = &self.pending.projection {
            tokio::select! { biased;
                _ = self.ended() => return Err(Error::Expired),
                result = gate.wait() => result?,
            }
        }
        self.ensure_current()
    }

    /// Invoke the final synchronous sink immediately after validation. The sink
    /// must not queue the released action for a later write. Async I/O adapters
    /// must retain the wrapper while waiting for readiness, then release at the
    /// write boundary; its end also needs to cancel their own pending I/O.
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

    /// Its flow/circuit was retired, or this packet's retention deadline passed.
    pub async fn ended(&self) {
        self.pending.ended().await;
    }
}

impl PendingRoute {
    pub(crate) fn hop(&self) -> Hop {
        self.checked.hop
    }

    pub(crate) fn route(&self) -> &Route {
        &self.checked.route
    }

    pub(crate) fn ensure_current(&self) -> Result<(), Error> {
        let (instant, now_ms) = (std::time::Instant::now(), authority::now_ms());
        if !self.life.live_at(instant, now_ms) {
            return Err(Error::Expired);
        }
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

    /// Bind the receiving circuit's own bound and cancellation, once.
    pub(crate) fn constrain(
        &mut self,
        reading: &View<'_>,
        circuit: &Lifetime,
    ) -> Result<(), Error> {
        if self.constrained {
            return Err(Error::ExecutionScope);
        }
        self.constrained = true;
        self.life.restrict(circuit.cap);
        self.life.cancellation = circuit.cancellation.clone();
        self.check_at(reading)
    }

    pub(crate) fn check_at(&self, reading: &View<'_>) -> Result<(), Error> {
        if !self.life.live_at(reading.instant, reading.now_ms) {
            return Err(Error::Expired);
        }
        Ok(())
    }

    pub(crate) async fn ended(&self) {
        if self.ensure_current().is_err() {
            return;
        }
        self.life.ended().await;
    }
}

/// Unique flow state owner on Engine's desired content. New content keeps
/// exactly the flows it still decides; see [`secure::SecureNode::apply`].
pub struct Engine {
    desired: Desired,
    flows: FlowTable,
    exit_counters: exit::Counters,
    decisions: netmap::Decisions,
}

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Engine")
            .field("flow_count", &self.flows.len())
            .finish_non_exhaustive()
    }
}

pub mod carrier;
pub mod secure;
