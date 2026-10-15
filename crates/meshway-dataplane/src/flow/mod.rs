//! One bounded owner of forward facts. Preparation is read-only apart from
//! expired-entry collection; commit happens only after final guard validation.
//!
//! A flow is keyed by its (origin, target) pair, local role, complete
//! original path and tuple. The installed content is not part of the key: a content update
//! checks each flow in place against the new content and either keeps it with
//! all protocol state, or retires it alone. No table is cloned or relabeled.
mod exit;
mod index;
mod tcp;

use crate::{Direction, Error, LocalFlow, PacketFacts, Quoted, Route, Transport, authority::Cap};
use meshway_configuration::execution::BusinessProtocol;
use meshway_model::{Digest, NodeId};
use meshway_policy::{
    admission::{Hop, Traffic},
    compile::netmap::{Decision, Target},
};
use meshway_transport::spec::Port;
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
    /// The forward tuple of the packet an ICMP error quotes.
    fn quoted(quoted: &Quoted) -> Self {
        let traffic = quoted.traffic;
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
            echo: quoted.echo,
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
    /// Target of the (origin = `path[0]`, target) pair; never installed content.
    grant: Target,
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
    /// `resumable`: this tuple's TCP state was reclaimed while idle.
    fn new(transport: Transport, resumable: bool) -> Result<Self, Error> {
        match transport {
            Transport::Udp => Ok(Self::Udp),
            Transport::Echo { reply: false, .. } => Ok(Self::Echo),
            Transport::Tcp(segment) => Ok(Self::Tcp(TcpState::new(segment, resumable)?)),
            _ => Err(Error::Flow),
        }
    }
}

#[derive(Clone)]
struct Flow {
    forward: Traffic,
    state: ProtocolState,
    /// Idle bound at the last admission; additionally bounded by the live lease.
    cap: Cap,
    circuit: Option<[u8; 32]>,
    exit: Option<exit::ExitState>,
    /// Source egress selection of an exit flow; a change ends the flow.
    selection: Option<Digest>,
    /// Exit TCP completion drains its final guarded output within the already
    /// installed deadline. It accepts no further packets and never renews.
    closing: bool,
    /// The decision this flow was admitted with and the compiler (executed
    /// content and artifacts) that made it. Under the same compiler the flow
    /// keeps it: a trusted DNS association that ends changes only new
    /// flows. Other content decides again (and `SecureNode::apply` already
    /// re-judged the flow in place).
    decided: (
        std::sync::Weak<meshway_policy::compile::netmap::LocalCompiler>,
        Arc<Decision>,
    ),
    /// ICMP errors relating to this flow in the current one-second window.
    related: RelatedBudget,
}

/// An echo request's flow (one identifier and sequence) waits this long for
/// its reply or an ICMP error about it; ordinary flows idle much longer. At
/// an exit every echo flow holds one projection of the owner's bounded
/// capacity, so a pinging or tracerouting source holds only its last few.
pub(crate) const ECHO_UNANSWERED: Duration = Duration::from_secs(10);
/// After its reply an echo flow ends this soon: a duplicate reply still
/// returns, and a late one meets the retired sequence's quarantine.
pub(crate) const ECHO_ANSWERED: Duration = Duration::from_secs(2);

/// At most [`RELATED_PER_SECOND`] ICMP errors per flow and second at each
/// endpoint: one error answers one packet, the bound stops a flood of them.
pub(crate) const RELATED_PER_SECOND: u8 = 10;

#[derive(Clone, Copy, Default)]
struct RelatedBudget {
    since: Option<Instant>,
    count: u8,
}

impl RelatedBudget {
    fn take(&mut self, now: Instant) -> bool {
        if self
            .since
            .is_none_or(|since| now.saturating_duration_since(since) >= Duration::from_secs(1))
        {
            *self = Self {
                since: Some(now),
                count: 0,
            };
        }
        if self.count >= RELATED_PER_SECOND {
            return false;
        }
        self.count += 1;
        true
    }
}

/// The checked route of an ICMP error on its flow's reply direction.
pub(crate) struct Related {
    pub cap: Cap,
    pub flow: Option<tokio_util::sync::CancellationToken>,
    pub projection: Option<Arc<crate::exit::Gate>>,
    pub checked: CommittedRoute,
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

/// One entry reading. The lease is checked at every use and renews in
/// place, so it is never copied into a flow.
pub(super) struct FlowTime {
    pub instant: Instant,
    pub now_ms: u64,
}

impl FlowTime {
    fn idle(&self, idle: Duration) -> Result<Cap, Error> {
        let at = self.instant.checked_add(idle).ok_or(Error::Limits)?;
        Ok(Cap { at, ms: u64::MAX })
    }
}

/// Authenticated ingress facts for one packet; never decoded from plaintext.
pub(crate) struct Entry<'a> {
    pub grant: Target,
    pub path: &'a [NodeId],
    pub origin: NodeId,
    pub previous: Option<NodeId>,
    pub sender: NodeId,
}

/// The policy view a packet is admitted under. `decide` returns the
/// Permitted decision of the forward traffic's (origin, destination, service)
/// on the same content, or an error.
pub(crate) struct Policy<'a, 'd> {
    pub netmap: &'a meshway_configuration::netmap::NodeNetmap,
    /// The compiler of the executed content (`Desired::compiler`): a flow
    /// keeps the decision it was admitted with while this stays the same.
    pub compiler: &'a Arc<meshway_policy::compile::netmap::LocalCompiler>,
    pub decide: &'d mut dyn FnMut(&Traffic) -> Result<Arc<Decision>, Error>,
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

/// Read-only facts of one kept flow for an in-place content update.
pub(crate) struct Kept<'a> {
    pub grant: &'a Target,
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
    /// Terminal tombstones whose Retire has not been handed out yet.
    unclaimed_retires: usize,
    /// When the next sweep of dead exit flows and ended quarantines is due.
    exit_sweep_at: Option<Instant>,
    retired_circuits_pending: bool,
    /// Endpoint TCP tuples whose state was reclaimed while idle, under the
    /// current content only; at most `max_flows`.
    idle: std::collections::BTreeSet<(NodeId, Tuple)>,
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
            unclaimed_retires: 0,
            exit_sweep_at: None,
            retired_circuits_pending: false,
            idle: Default::default(),
        }
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn clear(&mut self) {
        if !self.entries.is_empty() && crate::log_budget::event() {
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
        self.idle.clear();
    }
    pub fn expire(&mut self, now: Instant, now_ms: u64) {
        self.expire_exits(now, now_ms);
        while let Some(key) = self.index.expired(now, now_ms).cloned() {
            let flow = self.remove(&key).expect("indexed flow");
            let idle = now >= flow.cap.at;
            if idle && matches!(flow.state, ProtocolState::Tcp(_)) && key.role != Role::Relay {
                if self.idle.len() >= self.limits.max_flows {
                    self.idle.pop_first();
                }
                self.idle.insert((key.node, key.tuple.clone()));
            }
            if crate::log_budget::event() {
                let cause = if idle {
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
        self.idle.remove(&(key.node, key.tuple.clone()));
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

    /// Destinations of the live flows `node` originated toward an exit: what
    /// its capture must keep reaching this owner while those flows last.
    pub fn exit_destinations(
        &self,
        node: NodeId,
        time: FlowTime,
    ) -> std::collections::BTreeSet<IpAddr> {
        self.entries
            .iter()
            .filter(|(key, flow)| {
                key.node == node
                    && key.role == Role::Origin
                    && matches!(key.grant, Target::Exit { .. })
                    && !flow.closing
                    && flow.live_at(time.instant, time.now_ms)
            })
            .map(|(key, _)| key.tuple.destination)
            .collect()
    }

    /// Exact tuple lookup in the established table; never refresh or advance it.
    /// Relay traffic and the remote direction of endpoint traffic cannot be
    /// borrowed to originate a local packet. Ambiguous metadata fails closed.
    pub fn local_flow(
        &self,
        node: NodeId,
        facts: &PacketFacts,
        time: FlowTime,
    ) -> Result<Option<LocalMatch>, Error> {
        facts.validate()?;
        let tuple = Tuple::facts(facts);
        let reverse = tuple.reversed();
        if tuple == reverse {
            return Err(Error::Flow);
        }
        if let Transport::Error(error) = facts.transport {
            let key = self.related_key(node, &error.quoted, false, &time)?;
            let flow = &self.entries[&key];
            return Ok(Some(LocalMatch {
                route: LocalFlow::new(key.grant.clone(), key.path.clone(), flow.circuit),
                cap: flow.cap,
            }));
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
            let cap = flow.cap;
            let local = match key.role {
                Role::Origin => key.tuple == tuple && key.path.first() == Some(&node),
                Role::Terminal => key.tuple == reverse && key.path.last() == Some(&node),
                Role::Relay => false,
            };
            if !local || found.is_some() {
                return Err(Error::Flow);
            }
            found = Some(LocalMatch {
                route: LocalFlow::new(key.grant.clone(), key.path.clone(), flow.circuit),
                cap,
            });
        }
        if found.is_none() {
            ProtocolState::new(facts.transport, self.idle.contains(&(node, tuple)))?;
        }
        Ok(found)
    }

    /// The one live flow an ICMP error quoting `quoted` relates to: the flow
    /// whose forward packet it quotes, at its terminal (which sends the error
    /// back along the reply direction) or at its origin (which receives it).
    /// Closing, expired, unacknowledged exit and ambiguous flows never match.
    fn related_key(
        &self,
        node: NodeId,
        quoted: &Quoted,
        receiving: bool,
        time: &FlowTime,
    ) -> Result<Key, Error> {
        let tuple = Tuple::quoted(quoted);
        let role = if receiving {
            Role::Origin
        } else {
            Role::Terminal
        };
        let mut found = None;
        for key in self.index.matching(&tuple) {
            let flow = &self.entries[key];
            if key.node != node
                || key.role != role
                || key.tuple != tuple
                || flow.closing
                || !flow.live_at(time.instant, time.now_ms)
                || flow
                    .exit
                    .as_ref()
                    .and_then(|exit| exit.gate.as_ref())
                    .is_some_and(|gate| gate.ready().is_err())
            {
                continue;
            }
            if found.replace(key.clone()).is_some() {
                return Err(Error::Flow);
            }
        }
        found.ok_or(Error::Flow)
    }

    /// Admit an ICMP error on the reply direction of the flow it relates to
    /// ([`Self::related_key`]) for the circuit and path it arrived on or is
    /// sent on. `admit` is the Policy check over the flow's own decision and
    /// forward traffic. It neither refreshes nor advances the flow; a flow
    /// over its per-second budget answers `Capacity`, any mismatch `Flow`.
    pub fn related(
        &mut self,
        node: NodeId,
        entry: &Entry<'_>,
        quoted: &Quoted,
        circuit: Option<[u8; 32]>,
        time: FlowTime,
        admit: impl FnOnce(&Decision, &Traffic) -> Result<Hop, Error>,
    ) -> Result<Related, Error> {
        self.expire(time.instant, time.now_ms);
        let path = entry.path;
        let (origin, terminal) = match (path.first(), path.last()) {
            (Some(origin), Some(terminal)) if path.len() >= 2 => (*origin, *terminal),
            _ => return Err(Error::Ingress),
        };
        if entry.sender != terminal || (node != origin && node != terminal) {
            return Err(Error::Sender);
        }
        let key = self.related_key(node, quoted, node == origin, &time)?;
        let flow = &self.entries[&key];
        if key.grant != entry.grant || key.path != path || flow.circuit != circuit {
            return Err(Error::Flow);
        }
        let hop = admit(&flow.decided.1, &flow.forward)?;
        let flow = self.entries.get_mut(&key).ok_or(Error::Flow)?;
        if !flow.related.take(time.instant) {
            return Err(Error::Capacity);
        }
        let (cancellation, projection) = flow.exit.as_ref().map_or((None, None), |exit| {
            (Some(exit.cancellation.clone()), exit.gate.clone())
        });
        Ok(Related {
            cap: flow.cap,
            flow: cancellation,
            projection,
            checked: CommittedRoute {
                hop,
                route: Route {
                    grant: key.grant.clone(),
                    origin,
                    original_path: key.path.clone(),
                    direction: Direction::Reply,
                },
            },
        })
    }

    /// In-place content update. Each live flow is judged once by `keep`; a
    /// retired flow is removed alone (exit flows publish their Retire). No
    /// protocol state, idle time or key changes; a cap may only shrink.
    /// Unacknowledged exit projections and closing exit flows never cross.
    pub fn retain(&mut self, time: &FlowTime, mut keep: impl FnMut(&Kept<'_>) -> Keep) {
        // Idle resumption is valid under the content that admitted the
        // stream only; a content change ends every reclaimed stream.
        self.idle.clear();
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
            match keep(&Kept {
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
            if crate::log_budget::event() {
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
        let (hop, is_exit, selection, decided) = {
            static PROFILE: meshway_logging::profile::Metric =
                meshway_logging::profile::Metric::new("policy.packet");
            let _profile = PROFILE.enter();
            let Policy {
                netmap: leased,
                compiler,
                decide,
            } = policy;
            let target = &entry.grant;
            let kept = existing
                .map(|flow| &flow.decided)
                .filter(|(decided, _)| std::sync::Weak::ptr_eq(decided, &Arc::downgrade(compiler)))
                .map(|(_, decision)| decision.clone());
            let decision = match kept {
                Some(decision) => decision,
                None => decide(&forward)?,
            };
            let decided = (Arc::downgrade(compiler), decision.clone());
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
                )?,
                Direction::Reply => netmap::admit_reply(
                    leased,
                    &decision,
                    path,
                    entry.origin,
                    entry.previous,
                    &forward,
                    &facts.traffic,
                )?,
            };
            if existing.is_some_and(|flow| flow.selection != decision.selection) {
                return Err(Error::Flow);
            }
            (
                hop,
                matches!(target, Target::Exit { .. }),
                decision.selection,
                decided,
            )
        };
        let mut state = match existing {
            Some(flow) => flow.state.clone(),
            None => {
                if self.entries.len() == self.limits.max_flows {
                    return Err(Error::Capacity);
                }
                ProtocolState::new(
                    facts.transport,
                    direction == Direction::Forward
                        && self.idle.contains(&(key.node, key.tuple.clone())),
                )?
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
        let echo = matches!(facts.transport, Transport::Echo { .. });
        let mut cap = time.idle(if echo {
            self.limits.idle_timeout.min(ECHO_UNANSWERED)
        } else {
            self.limits.idle_timeout
        })?;
        if closing {
            cap = cap.min(existing.ok_or(Error::Flow)?.cap);
        }
        // Its reply completes one echo exchange: the flow (and an exit
        // projection) ends shortly after, never later than before it.
        if echo && direction == Direction::Reply {
            cap = time
                .idle(self.limits.idle_timeout.min(ECHO_ANSWERED))?
                .min(existing.ok_or(Error::Flow)?.cap);
        }
        let flow = Flow {
            forward,
            state,
            cap,
            circuit: existing.and_then(|flow| flow.circuit),
            exit: self.exit_state(&key, existing, is_exit)?,
            selection,
            closing,
            decided,
            related: existing.map_or_else(Default::default, |flow| flow.related),
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

#[cfg(test)]
mod tests;
