//! One bounded owner of forward facts. Preparation is read-only apart from
//! expired-entry collection; commit happens only after final guard validation.
//!
//! A flow is keyed by its stable grant, local role, complete original path and
//! tuple. The installed content is not part of the key: a content update
//! checks each flow in place against the new content and either keeps it with
//! all protocol state, or retires it alone. No table is cloned or relabeled.
mod exit;
mod index;
mod tcp;

use crate::{Direction, Error, LocalFlow, PacketFacts, Route, Transport, authority::Cap};
use meshway_configuration::execution::{BusinessProtocol, NodeConfiguration};
use meshway_model::{Digest, NodeId, RequestId};
use meshway_policy::{
    admission::{Hop, Traffic, admit_forward, admit_reply, netmap::Leased},
    compile::netmap::{Decision, Target},
};
use meshway_transport_spec::Port;
use std::{
    collections::BTreeMap,
    net::IpAddr,
    sync::Arc,
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

/// The stable grant a flow was admitted under. It never names installed content.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FlowGrant {
    /// The flow's (origin, target) pair on the installed netmap; origin is `path[0]`.
    Netmap(Target),
    /// Legacy per-demand grant; removed with the daemon switch.
    Legacy(RequestId),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Role {
    Origin,
    Relay,
    Terminal,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    grant: FlowGrant,
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
    /// Idle bound and, for legacy targets, the permit and guard expiry at the
    /// last admission. Netmap flows are additionally bounded by the live lease.
    cap: Cap,
    circuit: Option<[u8; 32]>,
    exit: Option<exit::ExitState>,
    /// Source egress selection of an exit flow; a change ends the flow.
    selection: Option<Digest>,
    /// Exit TCP completion drains its final guarded output within the already
    /// installed deadline. It accepts no further packets and never renews.
    closing: bool,
}

impl Flow {
    fn live_at(&self, now: Instant, now_ms: u64) -> bool {
        self.cap.live_at(now, now_ms)
            && self.exit.as_ref().is_none_or(|exit| exit.live(now, now_ms))
    }
}

pub(super) struct LocalMatch {
    pub route: LocalFlow,
    pub cap: Cap,
}

/// One entry reading. `guard` is the legacy snapshot expiry; netmap readings
/// carry none since the lease is checked at every use and renews in place.
pub(super) struct FlowTime {
    pub instant: Instant,
    pub now_ms: u64,
    pub guard: Option<Cap>,
}

impl FlowTime {
    fn idle(&self, idle: Duration) -> Result<Cap, Error> {
        let at = self.instant.checked_add(idle).ok_or(Error::Limits)?;
        let cap = Cap { at, ms: u64::MAX };
        Ok(self.guard.map_or(cap, |guard| cap.min(guard)))
    }
}

/// Authenticated ingress facts for one packet; never decoded from plaintext.
pub(crate) struct Entry<'a> {
    pub grant: FlowGrant,
    pub path: &'a [NodeId],
    pub origin: NodeId,
    pub previous: Option<NodeId>,
    pub sender: NodeId,
}

/// The policy view a packet is admitted under.
pub(crate) enum Policy<'a, 'd> {
    Legacy(&'a NodeConfiguration),
    /// `decide` returns the Permitted decision of the forward traffic's
    /// (origin, destination, service) on the same content, or an error.
    Netmap {
        leased: Leased<'a>,
        decide: &'d mut dyn FnMut(&Traffic) -> Result<Arc<Decision>, Error>,
    },
}

pub(super) struct Prepared {
    key: Key,
    flow: Flow,
    remove: bool,
    hop: Hop,
    direction: Direction,
    pub cap: Cap,
    previous_deadline: Option<Cap>,
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

/// Read-only facts of one retained flow for an in-place content update.
pub(crate) struct Retained<'a> {
    pub grant: &'a FlowGrant,
    pub path: &'a [NodeId],
    pub forward: &'a Traffic,
    pub selection: Option<Digest>,
}

pub(crate) enum Keep {
    Retire,
    /// Keep all protocol state; optionally shorten (never extend) its cap.
    Retain(Option<Cap>),
}

pub(super) struct FlowTable {
    limits: crate::Limits,
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
    pub fn new(limits: crate::Limits) -> Self {
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
            tracing::debug!(
                operation = "clear",
                count = self.entries.len(),
                "Dataplane flow table cleared"
            );
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
                let cause = if now >= flow.cap.at {
                    "monotonic_or_idle"
                } else {
                    "deadline"
                };
                tracing::debug!(operation = "expire", cause, grant = ?key.grant, now_ms,
                    expires_at_ms = flow.cap.ms, "Dataplane flow removed");
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
    /// `legacy` adds the per-demand permit check of a legacy target.
    pub fn local_flow(
        &self,
        node: NodeId,
        legacy: Option<&NodeConfiguration>,
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
            if key.node != node
                || (key.tuple != tuple && key.tuple != reverse)
                || !flow.live_at(time.instant, time.now_ms)
            {
                continue;
            }
            let mut cap = flow.cap;
            match (&key.grant, legacy) {
                (FlowGrant::Legacy(demand), Some(cfg)) => {
                    let Some(permit) = cfg.permits.get(demand) else {
                        continue;
                    };
                    if time.now_ms >= permit.expires_at_ms {
                        continue;
                    }
                    cap.ms = cap.ms.min(permit.expires_at_ms);
                }
                (FlowGrant::Netmap(_), None) => {}
                _ => continue,
            }
            let local = match key.role {
                Role::Origin => key.tuple == tuple && key.path.first() == Some(&node),
                Role::Terminal => key.tuple == reverse && key.path.last() == Some(&node),
                Role::Relay => false,
            };
            if !local || found.is_some() {
                return Err(Error::Flow);
            }
            if let Some(guard) = time.guard {
                cap = cap.min(guard);
            }
            found = Some(LocalMatch {
                route: LocalFlow::new(key.grant.clone(), key.path.clone(), flow.circuit),
                cap,
            });
        }
        if found.is_none() {
            ProtocolState::new(facts.transport)?;
        }
        Ok(found)
    }

    /// In-place content update. Each live flow is judged once by `keep`; a
    /// retired flow is removed alone (exit flows publish their Retire). No
    /// protocol state, idle time or key changes; a cap may only shrink.
    /// Unacknowledged exit projections and closing exit flows never cross.
    pub fn retain(&mut self, time: &FlowTime, mut keep: impl FnMut(&Retained<'_>) -> Keep) {
        let mut retire = Vec::new();
        let mut shorten = Vec::new();
        for (key, flow) in &self.entries {
            let pending = flow
                .exit
                .as_ref()
                .and_then(|exit| exit.gate.as_ref())
                .is_some_and(|gate| !gate.acknowledged());
            if !flow.live_at(time.instant, time.now_ms) || flow.closing || pending {
                retire.push((key.clone(), "deadline_or_projection"));
                continue;
            }
            match keep(&Retained {
                grant: &key.grant,
                path: &key.path,
                forward: &flow.forward,
                selection: flow.selection,
            }) {
                Keep::Retire => retire.push((key.clone(), "authorization_changed")),
                Keep::Retain(Some(cap)) if cap.min(flow.cap) != flow.cap => {
                    shorten.push((key.clone(), cap));
                }
                Keep::Retain(_) => {}
            }
        }
        for (key, cause) in retire {
            if crate::diagnostics::event() {
                tracing::debug!(operation = "update", cause, grant = ?key.grant,
                    "Dataplane flow retired by content update");
            }
            self.remove(&key);
        }
        for (key, cap) in shorten {
            let mut flow = self.entries[&key].clone();
            flow.cap = flow.cap.min(cap);
            self.insert(key, flow);
        }
    }

    pub fn prepare(
        &mut self,
        policy: Policy<'_, '_>,
        node: NodeId,
        entry: &Entry<'_>,
        facts: &PacketFacts,
        time: FlowTime,
    ) -> Result<Prepared, Error> {
        facts.validate()?;
        self.expire(time.instant, time.now_ms);
        let path = entry.path;
        // Match the compiler's bound before retaining any caller-owned path.
        if path.is_empty() || path.len() > meshway_policy::compile::MAX_HOPS {
            return Err(Error::Ingress);
        }
        let position = path
            .iter()
            .position(|other| *other == node)
            .ok_or(Error::Ingress)?;
        let tuple = Tuple::facts(facts);
        if tuple == tuple.reversed() {
            return Err(Error::Flow);
        }
        let mut key = Key {
            grant: entry.grant.clone(),
            node,
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
        // A second grant or candidate is not permission to move a live session
        // or duplicate its protocol state. Bind both grant and original path
        // across the node's entire live tuple/role scope.
        if self.index.matching(&key.tuple).any(|other| {
            other.node == key.node
                && other.role == key.role
                && (other.grant != key.grant || other.path != key.path)
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
                // A gate of replaced content requires a fresh projection before
                // output instead.
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
        if entry.sender != expected_sender {
            return Err(Error::Sender);
        }
        let forward = existing.map_or(facts.traffic, |flow| flow.forward);
        let (hop, permit_expires_at_ms, is_exit, selection) = {
            static PROFILE: meshway_logging::profile::Metric =
                meshway_logging::profile::Metric::new("policy.packet");
            let _profile = PROFILE.enter();
            match (policy, &entry.grant) {
                (Policy::Legacy(cfg), FlowGrant::Legacy(demand)) => {
                    let hop = match direction {
                        Direction::Forward => admit_forward(
                            cfg,
                            *demand,
                            path,
                            entry.origin,
                            entry.previous,
                            &facts.traffic,
                            time.now_ms,
                        )?,
                        Direction::Reply => admit_reply(
                            cfg,
                            *demand,
                            path,
                            entry.origin,
                            entry.previous,
                            &forward,
                            &facts.traffic,
                            time.now_ms,
                        )?,
                    };
                    // Policy owns grant validity. Looking up its expiry here only
                    // bounds retention and delayed output.
                    let permit = cfg.permits.get(demand).ok_or(Error::Ingress)?;
                    (hop, permit.expires_at_ms, permit.exit.is_some(), None)
                }
                (Policy::Netmap { leased, decide }, FlowGrant::Netmap(target)) => {
                    let decision = decide(&forward)?;
                    if decision.target != *target || decision.source != entry.origin {
                        return Err(Error::Flow);
                    }
                    use meshway_policy::admission::netmap;
                    let hop = match direction {
                        Direction::Forward => netmap::admit_forward(
                            leased,
                            &decision,
                            path,
                            entry.origin,
                            entry.previous,
                            &facts.traffic,
                            time.now_ms,
                        )?,
                        Direction::Reply => netmap::admit_reply(
                            leased,
                            &decision,
                            path,
                            entry.origin,
                            entry.previous,
                            &forward,
                            &facts.traffic,
                            time.now_ms,
                        )?,
                    };
                    let is_exit = matches!(target, Target::Exit { .. });
                    if existing.is_some_and(|flow| flow.selection != decision.selection) {
                        return Err(Error::Flow);
                    }
                    (hop, u64::MAX, is_exit, decision.selection)
                }
                _ => return Err(Error::Ingress),
            }
        };
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
        let closing = remove && is_exit;
        let idle = self.limits.idle_timeout.min(Duration::from_millis(
            permit_expires_at_ms.saturating_sub(time.now_ms),
        ));
        let mut cap = time.idle(idle)?;
        cap.ms = cap.ms.min(permit_expires_at_ms);
        if closing {
            cap = cap.min(existing.ok_or(Error::Flow)?.cap);
        }
        let flow = Flow {
            forward,
            state,
            cap,
            circuit: existing.and_then(|flow| flow.circuit),
            exit: self.exit_state(&key, existing, is_exit)?,
            selection,
            closing,
        };
        Ok(Prepared {
            key,
            flow,
            remove: remove && !closing,
            hop,
            direction,
            cap,
            previous_deadline: existing.map(|flow| flow.cap),
        })
    }

    pub fn commit(&mut self, prepared: Prepared) -> CommittedRoute {
        let route = Route {
            demand: match &prepared.key.grant {
                FlowGrant::Legacy(demand) => *demand,
                FlowGrant::Netmap(_) => crate::netmap_demand(),
            },
            grant: prepared.key.grant.clone(),
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

/// Legacy retention predicate, not packet admission. Policy still checks each
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
    Ok(carrier_bindings_unchanged(old, new, selected))
}

pub(super) fn carrier_bindings_unchanged(
    old: &NodeConfiguration,
    new: &NodeConfiguration,
    selected: &meshway_configuration::execution::LogicalPath,
) -> bool {
    let prefix = format!("{}/", old.node_id);
    for (dialer, keys) in &selected.dial_candidates {
        for key in keys {
            if *dialer == old.node_id && old.dialers.get(key) != new.dialers.get(key) {
                return false;
            }
            if let Some(listener) = key.strip_prefix(&prefix)
                && old.listeners.get(listener) != new.listeners.get(listener)
            {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests;
