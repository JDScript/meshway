//! One bounded owner of forward facts. Preparation is read-only apart from
//! expired-entry collection; commit happens only after final guard validation.
mod exit;
mod index;
mod tcp;

use crate::{Direction, Error, Ingress, Limits, LocalFlow, PacketFacts, Route, Transport};
use meshway_configuration::execution::{BusinessProtocol, NodeConfiguration};
use meshway_model::{NetworkId, NodeId, PublicationId, RequestId};
use meshway_policy::admission::{Hop, Traffic, admit_forward, admit_reply};
use meshway_transport_spec::Port;
use std::{
    collections::BTreeMap,
    net::IpAddr,
    time::{Duration, Instant},
};
use tcp::TcpState;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Tuple {
    source: IpAddr,
    destination: IpAddr,
    protocol: u8,
    source_port: Option<Port>,
    destination_port: Option<Port>,
    echo: Option<(u16, u16)>,
}
impl Tuple {
    fn facts(facts: &PacketFacts) -> Self {
        let traffic = facts.traffic;
        Self {
            source: traffic.source,
            destination: traffic.destination,
            protocol: match traffic.protocol {
                BusinessProtocol::Tcp => 6,
                BusinessProtocol::Udp => 17,
                BusinessProtocol::IcmpEcho => 1,
            },
            source_port: traffic.source_port,
            destination_port: traffic.destination_port,
            echo: match facts.transport {
                Transport::Echo {
                    identifier,
                    sequence,
                    ..
                } => Some((identifier, sequence)),
                _ => None,
            },
        }
    }
    fn reversed(&self) -> Self {
        Self {
            source: self.destination,
            destination: self.source,
            source_port: self.destination_port,
            destination_port: self.source_port,
            ..self.clone()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Role {
    Origin,
    Relay,
    Terminal,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    network: NetworkId,
    publication: PublicationId,
    demand: RequestId,
    node: NodeId,
    role: Role,
    path: Vec<NodeId>,
    tuple: Tuple,
}

#[derive(Clone)]
enum ProtocolState {
    Udp,
    Echo,
    Tcp(TcpState),
}

impl ProtocolState {
    /// The single initial-flow validator, shared by routing lookup and admission.
    fn new(transport: Transport) -> Result<Self, Error> {
        match transport {
            Transport::Udp => Ok(Self::Udp),
            Transport::Echo { reply: false, .. } => Ok(Self::Echo),
            Transport::Tcp(segment) => Ok(Self::Tcp(TcpState::new(segment)?)),
            _ => Err(Error::Flow),
        }
    }
}

#[derive(Clone)]
struct Flow {
    forward: Traffic,
    state: ProtocolState,
    expires_at: Instant,
    permit_expires_at_ms: u64,
    guard_expires_at_ms: u64,
    circuit: Option<[u8; 32]>,
    exit: Option<exit::ExitState>,
    /// Exit TCP completion drains its final guarded output within the already
    /// installed deadline. It accepts no further packets and never renews.
    closing: bool,
}

impl Flow {
    fn live_at(&self, now: Instant, now_ms: u64) -> bool {
        now < self.expires_at
            && self.exit.as_ref().is_none_or(|exit| exit.live(now, now_ms))
            && now_ms < self.permit_expires_at_ms
            && now_ms < self.guard_expires_at_ms
    }
}

pub(super) struct LocalMatch {
    pub route: LocalFlow,
    pub expires_at: Instant,
    pub expires_at_ms: u64,
}

pub(super) struct FlowTime {
    pub instant: Instant,
    pub now_ms: u64,
    pub guard_expires_at: Instant,
    pub guard_expires_at_ms: u64,
}

pub(super) struct Prepared {
    key: Key,
    flow: Flow,
    remove: bool,
    hop: Hop,
    direction: Direction,
    pub expires_at: Instant,
    pub permit_expires_at_ms: u64,
}

impl Prepared {
    /// Called only after SecureNode authenticated the complete packet. Binding
    /// commits with the existing protocol/route transaction, never from a hint,
    /// a handshake response, a metadata read, or a failed admission.
    pub fn bind_circuit(&mut self, circuit: [u8; 32]) -> Result<(), Error> {
        match self.direction {
            Direction::Forward => {
                if self.flow.exit.is_some() && self.flow.circuit.is_some_and(|old| old != circuit) {
                    return Err(Error::Flow);
                }
                self.flow.circuit = Some(circuit);
            }
            Direction::Reply if self.flow.circuit != Some(circuit) => return Err(Error::Flow),
            Direction::Reply => {}
        }
        Ok(())
    }
}

pub(super) struct FlowTable {
    limits: Limits,
    entries: BTreeMap<Key, Flow>,
    index: index::Index,
    exit_limits: crate::exit::Limits,
    tombstones: BTreeMap<Tuple, exit::Tombstone>,
    pending_projections: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    exit_count: usize,
    retired_circuits_pending: bool,
}

#[derive(Debug)]
pub(super) struct CommittedRoute {
    pub hop: Hop,
    pub route: Route,
}

impl FlowTable {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            entries: BTreeMap::new(),
            index: Default::default(),
            exit_limits: Default::default(),
            tombstones: Default::default(),
            pending_projections: Default::default(),
            exit_count: 0,
            retired_circuits_pending: false,
        }
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn clear(&mut self) {
        if !self.entries.is_empty() && crate::diagnostics::event() {
            tracing::debug!(operation = "clear", count = self.entries.len(),
                publication = ?self.entries.keys().next().map(|key| key.publication),
                "Dataplane flow table cleared");
        }
        for flow in self.entries.values() {
            if let Some(exit) = &flow.exit {
                exit.cancel();
            }
        }
        let entries = std::mem::take(&mut self.entries);
        for (key, flow) in entries {
            self.retire_exit(&key, &flow);
        }
        self.exit_count = 0;
        self.index = Default::default();
    }
    pub fn expire(&mut self, now: Instant, now_ms: u64) {
        self.expire_exits(now, now_ms);
        while let Some(key) = self.index.expired(now, now_ms).cloned() {
            let flow = self.remove(&key).expect("indexed flow");
            if crate::diagnostics::event() {
                let cause = if now >= flow.expires_at {
                    "monotonic_or_idle"
                } else if now_ms >= flow.permit_expires_at_ms {
                    "permit_deadline"
                } else {
                    "guard_deadline"
                };
                tracing::debug!(operation = "expire", cause, demand = %key.demand,
                    publication = %key.publication, now_ms,
                    permit_expires_at_ms = flow.permit_expires_at_ms,
                    guard_expires_at_ms = flow.guard_expires_at_ms,
                    "Dataplane flow removed");
            }
        }
    }

    fn insert(&mut self, key: Key, flow: Flow) -> Option<Flow> {
        self.exit_count += usize::from(flow.exit.is_some());
        self.exit_count -=
            usize::from(self.entries.get(&key).is_some_and(|old| old.exit.is_some()));
        self.index.insert(&key, &flow, self.entries.get(&key));
        self.entries.insert(key, flow)
    }

    fn remove(&mut self, key: &Key) -> Option<Flow> {
        if let Some(exit) = &self.entries.get(key)?.exit {
            exit.cancel();
        }
        let flow = self.entries.remove(key)?;
        self.exit_count -= usize::from(flow.exit.is_some());
        self.retire_exit(key, &flow);
        self.index.remove(key, &flow);
        Some(flow)
    }

    /// Exact tuple lookup in the established table; never refresh or advance it.
    /// Relay traffic and the remote direction of endpoint traffic cannot be
    /// borrowed to originate a local packet. Ambiguous metadata fails closed.
    pub fn local_flow(
        &self,
        config: &NodeConfiguration,
        facts: &PacketFacts,
        time: FlowTime,
    ) -> Result<Option<LocalMatch>, Error> {
        facts.validate()?;
        let tuple = Tuple::facts(facts);
        let reverse = tuple.reversed();
        if tuple == reverse {
            return Err(Error::Flow);
        }
        let mut found = None;
        for key in self.index.matching(&tuple) {
            let flow = &self.entries[key];
            if key.network != config.network_id
                || key.node != config.node_id
                || key.publication != config.publication
                || (key.tuple != tuple && key.tuple != reverse)
                || !flow.live_at(time.instant, time.now_ms)
            {
                continue;
            }
            let Some(permit) = config.permits.get(&key.demand) else {
                continue;
            };
            if time.now_ms >= permit.expires_at_ms {
                continue;
            }
            let local = match key.role {
                Role::Origin => key.tuple == tuple && key.path.first() == Some(&config.node_id),
                Role::Terminal => key.tuple == reverse && key.path.last() == Some(&config.node_id),
                Role::Relay => false,
            };
            if !local || found.is_some() {
                return Err(Error::Flow);
            }
            found = Some(LocalMatch {
                route: LocalFlow {
                    demand: key.demand,
                    path: key.path.clone(),
                    circuit: flow.circuit,
                },
                expires_at: flow.expires_at.min(time.guard_expires_at),
                expires_at_ms: flow
                    .permit_expires_at_ms
                    .min(flow.guard_expires_at_ms)
                    .min(permit.expires_at_ms)
                    .min(time.guard_expires_at_ms),
            });
        }
        if found.is_none() {
            ProtocolState::new(facts.transport)?;
        }
        Ok(found)
    }

    /// Prepare a bounded replacement index without mutating protocol/idle state.
    /// All old keys belong to one publication, so relabeling is injective; reject
    /// defensively if that invariant is ever broken.
    pub fn reconciled(
        &self,
        old: &NodeConfiguration,
        new: &NodeConfiguration,
        time: FlowTime,
    ) -> Result<Self, Error> {
        let mut next = Self::new(self.limits);
        next.exit_limits = self.exit_limits;
        next.pending_projections = self.pending_projections.clone();
        for (key, flow) in &self.entries {
            if !flow.live_at(time.instant, time.now_ms)
                || flow.closing
                || flow
                    .exit
                    .as_ref()
                    .and_then(|exit| exit.gate.as_ref())
                    .is_some_and(|gate| !gate.acknowledged())
                || time.now_ms >= flow.permit_expires_at_ms
                || time.now_ms >= flow.guard_expires_at_ms
                || key.publication != old.publication
                || !authorization_unchanged(old, new, key.demand, &key.path, time.now_ms)?
            {
                if crate::diagnostics::event() {
                    let cause = if time.instant >= flow.expires_at {
                        "monotonic_or_idle"
                    } else if time.now_ms >= flow.permit_expires_at_ms {
                        "permit_deadline"
                    } else if time.now_ms >= flow.guard_expires_at_ms {
                        "guard_deadline"
                    } else if key.publication != old.publication {
                        "publication"
                    } else {
                        "authorization_changed"
                    };
                    tracing::debug!(operation = "reconcile", cause, demand = %key.demand,
                        old_publication = %old.publication, new_publication = %new.publication,
                        now_ms = time.now_ms, permit_expires_at_ms = flow.permit_expires_at_ms,
                        guard_expires_at_ms = flow.guard_expires_at_ms,
                        "Dataplane flow excluded from reconciliation");
                }
                continue;
            }
            let mut key = key.clone();
            key.publication = new.publication;
            let mut flow = flow.clone();
            flow.permit_expires_at_ms = flow
                .permit_expires_at_ms
                .min(new.permits[&key.demand].expires_at_ms);
            flow.guard_expires_at_ms = flow.guard_expires_at_ms.min(time.guard_expires_at_ms);
            flow.expires_at = flow.expires_at.min(time.guard_expires_at).min(
                time.instant
                    .checked_add(Duration::from_millis(
                        flow.permit_expires_at_ms.saturating_sub(time.now_ms),
                    ))
                    .ok_or(Error::Limits)?,
            );
            if next.insert(key, flow).is_some() {
                return Err(Error::ExecutionScope);
            }
        }
        Ok(next)
    }

    pub fn prepare(
        &mut self,
        config: &NodeConfiguration,
        ingress: &Ingress,
        facts: &PacketFacts,
        time: FlowTime,
    ) -> Result<Prepared, Error> {
        facts.validate()?;
        self.expire(time.instant, time.now_ms);
        let path = ingress.path();
        // Match the compiler's bound before retaining any caller-owned path.
        if path.is_empty() || path.len() > meshway_policy::compile::MAX_HOPS {
            return Err(Error::Ingress);
        }
        let position = path
            .iter()
            .position(|node| *node == config.node_id)
            .ok_or(Error::Ingress)?;
        let origin = ingress.origin().ok_or(Error::Ingress)?;
        let tuple = Tuple::facts(facts);
        if tuple == tuple.reversed() {
            return Err(Error::Flow);
        }
        let mut key = Key {
            network: config.network_id,
            publication: config.publication,
            demand: ingress.demand(),
            node: config.node_id,
            role: if position == 0 {
                Role::Origin
            } else if position + 1 == path.len() {
                Role::Terminal
            } else {
                Role::Relay
            },
            path: path.to_vec(),
            tuple,
        };
        let reverse_key = Key {
            tuple: key.tuple.reversed(),
            ..key.clone()
        };
        // A second permit or candidate is not permission to move a live session
        // or duplicate its protocol state. Bind both demand and original path
        // across the node's entire live tuple/role scope.
        if self.index.matching(&key.tuple).any(|other| {
            other.network == key.network
                && other.publication == key.publication
                && other.node == key.node
                && other.role == key.role
                && (other.demand != key.demand || other.path != key.path)
                && (other.tuple == key.tuple || other.tuple == reverse_key.tuple)
        }) {
            return Err(Error::Flow);
        }
        let (existing, direction) = if let Some(flow) = self.entries.get(&reverse_key) {
            key = reverse_key;
            (Some(flow), Direction::Reply)
        } else {
            (self.entries.get(&key), Direction::Forward)
        };
        if existing.is_some_and(|flow| flow.closing) {
            return Err(Error::Flow);
        }
        if let Some(exit) = existing.and_then(|flow| flow.exit.as_ref()) {
            if !exit.live(time.instant, time.now_ms) {
                return Err(Error::Expired);
            }
            if direction == Direction::Reply
                && let Some(gate) = &exit.gate
            {
                // A pre-ACK kernel packet cannot create or advance reply state.
                // An invalidated snapshot may be legally handed off; in that
                // case a fresh projection is required before output instead.
                if gate.scope.ensure_current().is_ok() {
                    gate.ready()?;
                }
            }
        }
        let expected_sender = match direction {
            Direction::Forward => path.first(),
            Direction::Reply => path.last(),
        }
        .copied()
        .ok_or(Error::Ingress)?;
        if ingress.sender(config.node_id) != expected_sender {
            return Err(Error::Sender);
        }
        let hop = {
            static PROFILE: meshway_logging::profile::Metric =
                meshway_logging::profile::Metric::new("policy.packet");
            let _profile = PROFILE.enter();
            match direction {
                Direction::Forward => admit_forward(
                    config,
                    ingress.demand(),
                    path,
                    origin,
                    ingress.previous(),
                    &facts.traffic,
                    time.now_ms,
                )?,
                Direction::Reply => admit_reply(
                    config,
                    ingress.demand(),
                    path,
                    origin,
                    ingress.previous(),
                    &existing.ok_or(Error::Flow)?.forward,
                    &facts.traffic,
                    time.now_ms,
                )?,
            }
        };
        // Policy owns grant validity. Looking up its expiry here only bounds
        // retention and delayed output; it is not an alternate admission path.
        let permit_expires_at_ms = config
            .permits
            .get(&ingress.demand())
            .ok_or(Error::Ingress)?
            .expires_at_ms;
        let mut state = match existing {
            Some(flow) => flow.state.clone(),
            None => {
                if self.entries.len() == self.limits.max_flows {
                    return Err(Error::Capacity);
                }
                ProtocolState::new(facts.transport)?
            }
        };
        let remove = if existing.is_some() {
            match (&mut state, facts.transport) {
                (ProtocolState::Udp, Transport::Udp) => false,
                (ProtocolState::Echo, Transport::Echo { reply, .. })
                    if reply == (direction == Direction::Reply) =>
                {
                    false
                }
                (ProtocolState::Tcp(tcp), Transport::Tcp(segment)) => {
                    tcp.accept(direction, segment)?
                }
                _ => return Err(Error::Flow),
            }
        } else {
            false
        };
        let is_exit = config
            .permits
            .get(&ingress.demand())
            .is_some_and(|permit| permit.exit.is_some());
        let closing = remove && is_exit;
        let mut expires_at = time
            .instant
            .checked_add(self.limits.idle_timeout.min(Duration::from_millis(
                permit_expires_at_ms.saturating_sub(time.now_ms),
            )))
            .ok_or(Error::Limits)?
            .min(time.guard_expires_at);
        let permit_expires_at_ms = if closing {
            let previous = existing.ok_or(Error::Flow)?;
            expires_at = expires_at.min(previous.expires_at);
            permit_expires_at_ms.min(previous.permit_expires_at_ms)
        } else {
            permit_expires_at_ms
        };
        let flow = Flow {
            forward: existing.map_or(facts.traffic, |flow| flow.forward),
            state,
            expires_at,
            permit_expires_at_ms,
            guard_expires_at_ms: time.guard_expires_at_ms,
            circuit: existing.and_then(|flow| flow.circuit),
            exit: self.exit_state(&key, existing, is_exit)?,
            closing,
        };
        Ok(Prepared {
            key,
            flow,
            remove: remove && !closing,
            hop,
            direction,
            expires_at,
            permit_expires_at_ms,
        })
    }

    pub fn commit(&mut self, prepared: Prepared) -> CommittedRoute {
        let route = Route {
            demand: prepared.key.demand,
            origin: prepared.key.path[0],
            original_path: prepared.key.path.clone(),
            direction: prepared.direction,
        };
        if prepared.remove {
            self.remove(&prepared.key);
        } else {
            self.insert(prepared.key, prepared.flow);
        }
        CommittedRoute {
            hop: prepared.hop,
            route,
        }
    }
}

/// Shared retention predicate, not packet admission. Policy still checks each
/// current packet/handshake. Compare complete permit authorization and every
/// selected-path identity, address, key and locally published carrier binding.
pub(super) fn authorization_unchanged(
    old: &NodeConfiguration,
    new: &NodeConfiguration,
    demand: RequestId,
    path: &[NodeId],
    now_ms: u64,
) -> Result<bool, Error> {
    let (Some(before), Some(after)) = (old.permits.get(&demand), new.permits.get(&demand)) else {
        return Ok(false);
    };
    if now_ms >= before.expires_at_ms
        || now_ms >= after.expires_at_ms
        || meshway_policy::permit_transition(before, after, now_ms)
            .map_err(|_| Error::ExecutionScope)?
            == meshway_policy::PermitTransition::Replacement
        || path.iter().any(|node| {
            *node != old.node_id
                && (!old.peers.contains_key(node) || old.peers.get(node) != new.peers.get(node))
        })
        || (path.last() == Some(&old.node_id) && old.exit_enabled != new.exit_enabled)
    {
        return Ok(false);
    }
    let Some(selected) = before
        .paths
        .iter()
        .find(|candidate| candidate.nodes == path)
    else {
        return Ok(false);
    };
    let prefix = format!("{}/", old.node_id);
    for (dialer, keys) in &selected.dial_candidates {
        for key in keys {
            if *dialer == old.node_id && old.dialers.get(key) != new.dialers.get(key) {
                return Ok(false);
            }
            if let Some(listener) = key.strip_prefix(&prefix)
                && old.listeners.get(listener) != new.listeners.get(listener)
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests;
