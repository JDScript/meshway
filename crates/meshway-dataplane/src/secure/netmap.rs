//! Desired netmap content: its idempotent application, the local
//! first-packet path and adjacency interest. No Server round trip.
use super::{
    CircuitGrant, CircuitState, Error, PendingFrame, SecureNode, State, View, authorization,
    crypto, pipeline,
};
use crate::{
    Packet, Purpose,
    authority::netmap_addresses,
    flow::{FlowTime, Keep},
    netmap::Outcome,
};
use meshway_configuration::execution::{BusinessProtocol, LogicalPath, UnavailableReason};
use meshway_model::NodeId;
use meshway_policy::compile::netmap::{Target, observable};
use std::{
    collections::{BTreeSet, VecDeque},
    net::IpAddr,
    time::Instant,
};

/// Packets one target may queue while its circuit or adjacency is pending.
const MAX_QUEUED: usize = 32;
/// Targets that may queue first packets at once.
const MAX_TARGETS: usize = 64;
/// Distinct peers of outstanding adjacency interest.
const MAX_INTERESTS: usize = 256;

/// Circuits and flows an [`SecureNode::apply`] retired; reporting only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub retired_circuits: usize,
    pub retired_flows: usize,
}

/// A local need for an authenticated adjacency to `peer`, from a first packet
/// or a relayed handshake. Connectivity acquires it only if its own adjacency
/// admission allows; interest grants nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AdjacencyInterest {
    pub peer: NodeId,
}

/// Result of a local TUN packet.
#[derive(Debug)]
pub enum Outbound {
    /// Admitted and committed on an established circuit; seal and send.
    Send(pipeline::SendJob),
    /// Queued until its circuit is established. `init` is a new handshake to send.
    Queued {
        init: Option<PendingFrame>,
    },
    /// Queued; no path's next hop has an admitted carrier. Interest is registered.
    AwaitingAdjacency,
    /// Local egress outside the Mesh, per the netmap's egress plan.
    Direct,
    Unmanaged,
    /// Dropped: the netmap denies this flow.
    Denied(UnavailableReason),
}

/// Output of [`SecureNode::flush`].
#[derive(Debug)]
pub enum Flushed {
    Init(PendingFrame),
    Send(pipeline::SendJob),
}

struct Waiting {
    target: Target,
    paths: Vec<LogicalPath>,
    packets: VecDeque<Packet>,
    until: Instant,
}

#[derive(Default)]
pub(super) struct FirstPackets {
    waiting: Vec<Waiting>,
    interests: BTreeSet<NodeId>,
    pub ready_exits: BTreeSet<NodeId>,
    pub dropped: u64,
}

impl FirstPackets {
    pub fn interest(&mut self, peer: NodeId) {
        if self.interests.len() < MAX_INTERESTS {
            self.interests.insert(peer);
        }
    }
    pub fn clear(&mut self) {
        self.waiting.clear();
    }
    pub fn maintain(&mut self, now: Instant) {
        let before: usize = self.waiting.iter().map(|w| w.packets.len()).sum();
        self.waiting.retain(|waiting| now < waiting.until);
        let after: usize = self.waiting.iter().map(|w| w.packets.len()).sum();
        self.dropped = self.dropped.saturating_add((before - after) as u64);
    }
    fn queue(&mut self, target: &Target, paths: &[LogicalPath], packet: Packet, until: Instant) {
        if let Some(waiting) = self.waiting.iter_mut().find(|w| w.target == *target) {
            if waiting.packets.len() >= MAX_QUEUED {
                self.dropped = self.dropped.saturating_add(1);
                return;
            }
            waiting.packets.push_back(packet);
            return;
        }
        if self.waiting.len() >= MAX_TARGETS {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.waiting.push(Waiting {
            target: target.clone(),
            paths: paths.to_vec(),
            packets: VecDeque::from([packet]),
            until,
        });
    }
}

/// Paths of a permitted `icmpEcho` decision toward `target` that every Node
/// on them can observe; `Policy` when denied, `Unsupported` when none is.
fn observable_paths(
    netmap: &meshway_configuration::netmap::CompiledNetmap,
    target: NodeId,
    outcome: &Outcome,
    path: Option<&[NodeId]>,
) -> Result<Vec<LogicalPath>, Error> {
    let Outcome::Permitted(decision) = outcome else {
        return Err(Error::Policy);
    };
    if decision.target != (Target::Mesh { node: target }) {
        return Err(Error::Policy);
    }
    let candidates: Vec<&LogicalPath> = decision
        .paths
        .iter()
        .filter(|candidate| path.is_none_or(|path| candidate.nodes == path))
        .collect();
    if candidates.is_empty() {
        return Err(Error::Policy);
    }
    let inventory = &netmap.netmap.projection.inventory;
    let paths: Vec<LogicalPath> = candidates
        .into_iter()
        .filter(|candidate| observable(inventory, &candidate.nodes))
        .cloned()
        .collect();
    if paths.is_empty() {
        return Err(Error::Unsupported);
    }
    Ok(paths)
}

/// The complete paths a `meshway ping` / observation of `target` may measure
/// on the desired netmap, decided locally like a first packet with the
/// `icmpEcho` service: no Server round trip. Denied is [`Error::Policy`];
/// permitted but with no path every Node can observe is [`Error::Unsupported`].
pub fn diagnostic_paths(
    desired: &meshway_engine::Desired,
    target: NodeId,
    path: Option<&[NodeId]>,
) -> Result<Vec<Vec<NodeId>>, Error> {
    let reading = View::of(desired);
    let netmap = reading.netmap();
    let address = mesh_address(&reading, target).ok_or(Error::Policy)?;
    let outcome = crate::netmap::Decisions::for_desired(desired)
        .decide(
            netmap,
            reading.node(),
            address,
            (BusinessProtocol::IcmpEcho, None),
        )
        .map_err(|_| Error::Policy)?;
    Ok(observable_paths(netmap, target, &outcome, path)?
        .into_iter()
        .map(|path| path.nodes)
        .collect())
}

fn mesh_address(view: &View<'_>, node: NodeId) -> Option<IpAddr> {
    let netmap = view.netmap();
    netmap_addresses(&netmap.netmap, node)
        .and_then(|a| a.ipv4.map(IpAddr::V4).or(a.ipv6.map(IpAddr::V6)))
}

impl SecureNode {
    /// Apply new desired content, idempotently. The same content changes
    /// nothing. Otherwise every circuit is compared in place: it keeps its
    /// Noise state only while its path authorization digest and key set are
    /// unchanged; a flow only while its exact service, path and exit selection
    /// are still decided. Exit flows retired here publish Retire. A changed own
    /// identity, address or key set retires everything. Retired circuits
    /// cancel their queued outputs before this returns.
    pub fn apply(&mut self, desired: &meshway_engine::Desired) -> Result<Applied, Error> {
        if desired.same(&self.desired) {
            return Ok(Applied::default());
        }
        if !self.keys.matches(&desired.local().self_keys) {
            return Err(Error::Authority);
        }
        let (old, new) = (self.desired.local(), desired.local());
        let same_self = self.desired.netmap().network_id == desired.netmap().network_id
            && self.desired.netmap().node_id == desired.netmap().node_id
            && old.enabled
            && new.enabled
            && old.identity == new.identity
            && old.addresses == new.addresses
            && old.self_keys == new.self_keys;
        self.engine.decisions = crate::netmap::Decisions::for_desired(desired);
        let reading = View::of(desired);
        if !same_self {
            let applied = Applied {
                retired_circuits: self.circuits.len(),
                retired_flows: self.engine.flows.len(),
            };
            self.terminate("reset");
            self.install(desired, &reading);
            return Ok(applied);
        }
        let netmap = reading.netmap();
        let decisions = &mut self.engine.decisions;
        let local = netmap.netmap.node_id;
        let mut retired_circuits = 0;
        for circuit in self.circuits.values_mut() {
            let CircuitGrant { purpose, target } = &circuit.context.grant;
            let path = &circuit.context.path;
            let position = path.iter().position(|node| *node == local);
            let admitted = position.and_then(|position| {
                authorization::admit_netmap(
                    decisions,
                    netmap,
                    *purpose,
                    target,
                    path,
                    position.checked_sub(1).map(|i| path[i]),
                    false,
                )
                .ok()
            });
            match admitted {
                Some(admitted)
                    if circuit.check_at(&reading).is_ok()
                        && admitted.authorization == circuit.context.authorization
                        && crypto::key_digest(&reading, path).ok()
                            == Some(circuit.context.keys) =>
                {
                    circuit.selected = Some(admitted.selected);
                }
                _ => {
                    circuit.retire_for("apply", "path_authorization_changed");
                    retired_circuits += 1;
                }
            }
        }
        let before = self.engine.flows.len();
        let time = FlowTime::from(&reading);
        let flows = &mut self.engine.flows;
        flows.retain(&time, |flow| {
            let target = flow.grant;
            let origin = flow.path[0];
            let forward = flow.forward;
            let kept = match decisions.decide(
                netmap,
                origin,
                forward.destination,
                (forward.protocol, forward.destination_port),
            ) {
                Ok(Outcome::Permitted(decision)) => {
                    decision.target == *target
                        && decision.selection == flow.selection
                        && decision.paths.iter().any(|path| path.nodes == flow.path)
                }
                _ => false,
            };
            if kept {
                Keep::Retain(None)
            } else {
                Keep::Retire
            }
        });
        let retired_flows = before.saturating_sub(self.engine.flows.len());
        self.install(desired, &reading);
        Ok(Applied {
            retired_circuits,
            retired_flows,
        })
    }

    /// Take the applied content: drop correlations of the old one and keep
    /// only carriers the new content admits.
    fn install(&mut self, desired: &meshway_engine::Desired, reading: &View<'_>) {
        let mut carriers = std::mem::take(&mut self.carriers);
        carriers.retain(|peer, carrier| {
            authorization::adjacency(&mut self.engine.decisions, reading, *peer, carrier)
        });
        self.carriers = carriers;
        self.probes.retain_current();
        self.engine.desired = desired.clone();
        self.desired = desired.clone();
        let closed: Vec<_> = self
            .circuits
            .iter()
            .filter(|(_, circuit)| matches!(circuit.state, State::Closed))
            .map(|(id, _)| *id)
            .collect();
        for id in closed {
            if let Some(circuit) = self.circuits.remove(&id) {
                self.bury(id, &circuit, reading.now_ms);
            }
        }
        self.maintenance.changed();
    }

    /// Peers this owner needs an adjacency with; drains the set.
    pub fn take_adjacency_interests(&mut self) -> Vec<AdjacencyInterest> {
        std::mem::take(&mut self.first.interests)
            .into_iter()
            .map(|peer| AdjacencyInterest { peer })
            .collect()
    }

    /// Exits whose readiness fact is fresh and ready; selection input only.
    /// Exit flows go to the first ready exit of their ordered candidates and
    /// are dropped (never sent direct) when none is ready; a DNS query to a
    /// not-ready exit resolver fails.
    pub fn set_ready_exits(&mut self, exits: BTreeSet<NodeId>) {
        self.first.ready_exits = exits;
    }

    /// First packets dropped by the bounded queue or its deadline.
    pub fn dropped_first_packets(&self) -> u64 {
        self.first.dropped
    }

    /// Admit a local TUN packet: an existing flow continues on its original
    /// path and circuit; a new flow is decided locally on the installed
    /// netmap. Nothing waits for a Server. Queued packets commit no flow;
    /// they are admitted again by [`Self::flush`].
    pub fn outbound(&mut self, packet: Packet) -> Result<Outbound, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.outbound");
        let _profile = PROFILE.enter();
        let desired = self.desired.clone();
        let reading = View::of(&desired);
        let netmap = reading.netmap();
        // A local packet originates here: it carries this Node's own address.
        let addresses = &desired.local().addresses;
        let own = match packet.facts().traffic.source {
            std::net::IpAddr::V4(address) => addresses.ipv4 == Some(address),
            std::net::IpAddr::V6(address) => addresses.ipv6 == Some(address),
        };
        if !own {
            return Err(Error::Packet);
        }
        self.maintain_circuits(&reading, false)?;
        let local = reading.node();
        if let Some(flow) = self.local_flow_at(&packet, &reading)? {
            let target = &flow.grant;
            let grant = CircuitGrant {
                purpose: Purpose::Business,
                target: target.clone(),
            };
            let live = flow.circuit.filter(|id| {
                self.circuits.get(id).is_some_and(|circuit| {
                    circuit.state.established() && circuit.check_at(&reading).is_ok()
                })
            });
            let established = live.or_else(|| {
                (flow.path.first() == Some(&local))
                    .then(|| self.established(&grant, &flow.path, &reading))
                    .flatten()
                    .map(|info| info.id)
            });
            if let Some(id) = established {
                if let Some(next) = self.missing_next_hop(id, local) {
                    // A live circuit whose next hop lost its adjacency: ask for
                    // that adjacency again and hold the packet, bounded, until
                    // the carrier is back or the circuit itself ends.
                    self.first.interest(next);
                    let path = LogicalPath {
                        nodes: flow.path.clone(),
                        dial_candidates: Default::default(),
                    };
                    let until = reading.instant + self.limits.handshake_timeout;
                    self.first.queue(target, &[path], packet, until);
                    return Ok(Outbound::AwaitingAdjacency);
                }
                return self
                    .prepare_send_at(id, packet, &reading)
                    .map(Outbound::Send);
            }
            if flow.path.first() != Some(&local) {
                return Err(Error::State);
            }
            // The origin reacquires a cipher only for the flow's unchanged path.
            let path = LogicalPath {
                nodes: flow.path.clone(),
                dial_candidates: Default::default(),
            };
            return self.wait(target.clone(), vec![path], packet, &reading);
        }
        let facts = packet.facts();
        let traffic = facts.traffic;
        let outcome = self
            .engine
            .decisions
            .decide(
                netmap,
                local,
                traffic.destination,
                (traffic.protocol, traffic.destination_port),
            )
            .map_err(|_| Error::Policy)?;
        let decision = match outcome {
            Outcome::Permitted(decision) => decision,
            Outcome::Direct => return Ok(Outbound::Direct),
            Outcome::Unmanaged => return Ok(Outbound::Unmanaged),
            Outcome::Denied(reason) => return Ok(Outbound::Denied(reason)),
        };
        let paths: Vec<LogicalPath> = match &decision.target {
            Target::Mesh { .. } => decision.paths.clone(),
            Target::Exit { nodes, .. } => {
                if traffic.destination.is_ipv6() || traffic.source.is_ipv6() {
                    return Ok(Outbound::Denied(UnavailableReason::UnsupportedFamily));
                }
                let Some(exit) = nodes
                    .iter()
                    .find(|exit| self.first.ready_exits.contains(exit))
                else {
                    self.first.dropped = self.first.dropped.saturating_add(1);
                    return Ok(Outbound::Denied(UnavailableReason::ExitUnavailable));
                };
                decision
                    .paths
                    .iter()
                    .filter(|path| path.nodes.last() == Some(exit))
                    .cloned()
                    .collect()
            }
        };
        let grant = CircuitGrant {
            purpose: Purpose::Business,
            target: decision.target.clone(),
        };
        for path in &paths {
            if let Some(info) = self.established(&grant, &path.nodes, &reading) {
                return self
                    .prepare_send_at(info.id, packet, &reading)
                    .map(Outbound::Send);
            }
        }
        self.wait(decision.target.clone(), paths, packet, &reading)
    }

    /// Queue a first packet behind a pending or new handshake on the first
    /// path whose next hop has an admitted carrier, or behind adjacency interest.
    fn wait(
        &mut self,
        target: Target,
        paths: Vec<LogicalPath>,
        packet: Packet,
        reading: &View<'_>,
    ) -> Result<Outbound, Error> {
        let until = reading.instant + self.limits.handshake_timeout;
        let grant = CircuitGrant {
            purpose: Purpose::Business,
            target: target.clone(),
        };
        let pending = self.circuits.values().any(|circuit| {
            circuit.context.grant == grant
                && !circuit.state.established()
                && !matches!(circuit.state, State::Closed)
                && paths.iter().any(|path| path.nodes == circuit.context.path)
        });
        let init = if pending {
            None
        } else {
            self.open(&grant, &paths, reading)?
        };
        let awaiting = !pending && init.is_none();
        self.first.queue(&target, &paths, packet, until);
        Ok(if awaiting {
            Outbound::AwaitingAdjacency
        } else {
            Outbound::Queued { init }
        })
    }

    /// Start a handshake on the first path with an admitted next-hop carrier,
    /// or register interest in every next hop, in path order.
    fn open(
        &mut self,
        grant: &CircuitGrant,
        paths: &[LogicalPath],
        reading: &View<'_>,
    ) -> Result<Option<PendingFrame>, Error> {
        for path in paths {
            let Some(next) = path.nodes.get(1) else {
                continue;
            };
            if let Some(carrier) = self.carriers.get(next).cloned() {
                return self
                    .start_at(grant.clone(), path.nodes.clone(), &carrier, reading)
                    .map(Some);
            }
        }
        for path in paths {
            if let Some(next) = path.nodes.get(1) {
                self.first.interest(*next);
            }
        }
        Ok(None)
    }

    /// Progress queued first packets: start handshakes whose adjacency became
    /// available and release packets whose circuit is established. Each packet
    /// is admitted afresh; expired ones are dropped and counted. At most
    /// `budget` outputs leave the queue, allowing the caller to reserve its
    /// bounded crypto/output capacity before any packet is committed.
    pub fn flush(&mut self, budget: usize) -> Vec<Result<Flushed, Error>> {
        let desired = self.desired.clone();
        let reading = View::of(&desired);
        self.first.maintain(reading.instant);
        let mut outputs = Vec::new();
        for mut waiting in std::mem::take(&mut self.first.waiting) {
            if outputs.len() >= budget {
                self.first.waiting.push(waiting);
                continue;
            }
            let grant = CircuitGrant {
                purpose: Purpose::Business,
                target: waiting.target.clone(),
            };
            let established = waiting
                .paths
                .iter()
                .find_map(|path| self.established(&grant, &path.nodes, &reading));
            if let Some(next) = established
                .as_ref()
                .and_then(|info| self.missing_next_hop(info.id, reading.node()))
            {
                self.first.interest(next);
                self.first.waiting.push(waiting);
                continue;
            }
            if let Some(info) = established {
                while outputs.len() < budget && !waiting.packets.is_empty() {
                    let packet = waiting.packets.pop_front().expect("nonempty queue");
                    outputs.push(
                        self.prepare_send_at(info.id, packet, &reading)
                            .map(Flushed::Send),
                    );
                }
                if !waiting.packets.is_empty() {
                    self.first.waiting.push(waiting);
                }
                continue;
            }
            let pending = self.circuits.values().any(|circuit| {
                circuit.context.grant == grant
                    && !circuit.state.established()
                    && !matches!(circuit.state, State::Closed)
                    && waiting
                        .paths
                        .iter()
                        .any(|path| path.nodes == circuit.context.path)
            });
            if !pending {
                match self.open(&grant, &waiting.paths, &reading) {
                    Ok(Some(init)) => outputs.push(Ok(Flushed::Init(init))),
                    Ok(None) => {}
                    Err(error) => outputs.push(Err(error)),
                }
            }
            self.first.waiting.push(waiting);
        }
        outputs
    }

    /// Open a diagnostic circuit toward a Mesh Node, entirely locally on the
    /// first observable path of [`diagnostic_paths`] (optionally an exact one).
    /// Denied is [`Error::Policy`]; permitted but unobservable is
    /// [`Error::Unsupported`]. `None` means no next hop has an admitted carrier
    /// yet; interest is registered.
    pub fn start_diagnostic(
        &mut self,
        target: NodeId,
        path: Option<&[NodeId]>,
    ) -> Result<Option<PendingFrame>, Error> {
        let desired = self.desired.clone();
        let reading = View::of(&desired);
        let netmap = reading.netmap();
        let address = mesh_address(&reading, target).ok_or(Error::Policy)?;
        let outcome = self
            .engine
            .decisions
            .decide(
                netmap,
                reading.node(),
                address,
                (BusinessProtocol::IcmpEcho, None),
            )
            .map_err(|_| Error::Policy)?;
        let paths = observable_paths(netmap, target, &outcome, path)?;
        let grant = CircuitGrant {
            purpose: Purpose::Diagnostic,
            target: Target::Mesh { node: target },
        };
        self.open(&grant, &paths, &reading)
    }

    /// Established diagnostic circuit toward `target`, if any.
    pub fn diagnostic_circuit(&self, target: NodeId) -> Option<super::CircuitInfo> {
        let reading = View::of(&self.desired);
        let grant = CircuitGrant {
            purpose: Purpose::Diagnostic,
            target: Target::Mesh { node: target },
        };
        self.circuits
            .iter()
            .filter(|(_, circuit)| circuit.context.grant == grant)
            .filter_map(|(id, _)| self.circuit_info_at(*id, &reading))
            .find(|info| info.state == CircuitState::Established)
    }
}
