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
use meshway_policy::compile::{
    MAX_PATHS,
    netmap::{Target, observable},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    net::IpAddr,
    time::Instant,
};

/// Packets one target may queue while its circuit or adjacency is pending.
const MAX_QUEUED: usize = 32;
/// Targets that may queue first packets at once.
const MAX_TARGETS: usize = 64;
/// Distinct peers of outstanding adjacency interest.
const MAX_INTERESTS: usize = 256;
/// Paths whose last origin handshake ended unanswered.
const MAX_UNANSWERED: usize = 256;
/// Longest pause of a path whose handshakes keep ending unanswered.
pub const MAX_PATH_PAUSE: std::time::Duration = std::time::Duration::from_secs(60);

/// What a path's reachability depends on in the content: the key set of its
/// Nodes and the carriers between each hop. A pause outlives content changes
/// that leave it equal (another Node's remark, Access, DNS, exits).
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct PathContent {
    keys: meshway_model::Digest,
    hops: Vec<(Vec<String>, Vec<String>)>,
}

impl PathContent {
    pub(super) fn of(
        decisions: &mut crate::netmap::Decisions,
        reading: &View<'_>,
        path: &[NodeId],
    ) -> Option<Self> {
        Some(Self {
            keys: crypto::key_digest(reading, path).ok()?,
            hops: decisions.hop_carriers(reading.netmap(), path)?,
        })
    }
}

/// One paused path: consecutive unanswered handshakes, the end of the pause
/// and the content it was paused on.
struct Paused {
    count: u32,
    until: Instant,
    content: Option<PathContent>,
}

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
    /// Paths whose origin handshake ended unanswered: consecutive count and
    /// the end of the pause during which another carried path is preferred.
    unanswered: BTreeMap<Vec<NodeId>, Paused>,
    /// Peers' reported authenticated channels (peer facts): ranking only.
    pub peer_channels: BTreeMap<NodeId, BTreeSet<NodeId>>,
}

impl FirstPackets {
    pub fn interest(&mut self, peer: NodeId) {
        if self.interests.len() < MAX_INTERESTS {
            self.interests.insert(peer);
        }
    }
    pub fn clear(&mut self) {
        self.waiting.clear();
        self.unanswered.clear();
    }
    /// An origin handshake on `path` ended at its deadline without an
    /// answer: a relay could not reach the next hop, or the path is dead.
    /// The pause doubles from `base` up to [`MAX_PATH_PAUSE`].
    pub(super) fn unanswered(
        &mut self,
        path: &[NodeId],
        content: Option<PathContent>,
        now: Instant,
        base: std::time::Duration,
    ) {
        self.unanswered.retain(|_, paused| paused.until > now);
        if self.unanswered.len() >= MAX_UNANSWERED && !self.unanswered.contains_key(path) {
            return;
        }
        let entry = self.unanswered.entry(path.to_vec()).or_insert(Paused {
            count: 0,
            until: now,
            content: content.clone(),
        });
        if entry.content != content {
            entry.count = 0;
            entry.content = content;
        }
        entry.count = entry.count.saturating_add(1);
        entry.until = now
            + base
                .saturating_mul(1 << (entry.count - 1).min(16))
                .min(MAX_PATH_PAUSE);
    }
    /// New content keeps the pause of every path whose own content is equal.
    fn retain_paths(&mut self, mut content: impl FnMut(&[NodeId]) -> Option<PathContent>) {
        self.unanswered
            .retain(|path, paused| paused.content.is_some() && content(path) == paused.content);
    }
    pub fn answered(&mut self, path: &[NodeId]) {
        self.unanswered.remove(path);
    }
    fn paused(&self, path: &[NodeId], now: Instant) -> bool {
        self.unanswered
            .get(path)
            .is_some_and(|paused| paused.until > now)
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

/// The candidate order of [`SecureNode::choose_path`]: indices into `paths`
/// (in Policy order, the direct path first), at most [`MAX_PATHS`], and how
/// many relayed paths were cut. A relayed path ranks by whether this Node
/// has a channel with its next hop, then whether that relay reports a
/// channel with the Node after it; ties keep Policy order.
fn ranked(
    paths: &[&[NodeId]],
    carriers: &BTreeMap<NodeId, super::Carrier>,
    channels: &BTreeMap<NodeId, BTreeSet<NodeId>>,
) -> (Vec<usize>, usize) {
    let rank = |path: &[NodeId]| -> (bool, bool, bool) {
        let relayed = path.len() > 2;
        let carried = path.get(1).is_some_and(|next| carriers.contains_key(next));
        let onward = path.get(1).zip(path.get(2)).is_some_and(|(relay, after)| {
            channels
                .get(relay)
                .is_some_and(|connected| connected.contains(after))
        });
        (relayed, !(relayed && carried), !(relayed && onward))
    };
    let mut order: Vec<usize> = (0..paths.len()).collect();
    order.sort_by_key(|&index| rank(paths[index]));
    let truncated = order.len().saturating_sub(MAX_PATHS);
    order.truncate(MAX_PATHS);
    (order, truncated)
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
        if desired.same_content(&self.desired) {
            // Only the trusted DNS associations changed: they decide new
            // flows; established flows and circuits keep their decisions.
            self.engine.decisions = crate::netmap::Decisions::for_desired(desired);
            self.engine.desired = desired.clone();
            self.desired = desired.clone();
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
        // A path stays paused only while its own content is unchanged.
        let decisions = &mut self.engine.decisions;
        self.first
            .retain_paths(|path| PathContent::of(decisions, reading, path));
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

    /// Each peer's reported authenticated channels, from fresh peer facts:
    /// they only rank relay candidates (see [`Self::choose_path`]).
    pub fn set_peer_channels(&mut self, channels: BTreeMap<NodeId, BTreeSet<NodeId>>) {
        self.first.peer_channels = channels;
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
        if let Some(info) = self.joinable(&grant, &paths, &reading) {
            return self
                .prepare_send_at(info.id, packet, &reading)
                .map(Outbound::Send);
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

    /// The established circuit a new flow joins: the first in the candidate
    /// order of [`Self::choose_path`], up to the path a new circuit would
    /// take there. A circuit on a lower-ranked path (a relay that lost the far
    /// end, while another one has channels to both) is not joined.
    fn joinable(
        &self,
        grant: &CircuitGrant,
        paths: &[LogicalPath],
        reading: &View<'_>,
    ) -> Option<super::CircuitInfo> {
        let refs: Vec<&[NodeId]> = paths.iter().map(|path| path.nodes.as_slice()).collect();
        let (order, _) = ranked(&refs, &self.carriers, &self.first.peer_channels);
        for index in order {
            let path = refs[index];
            if let Some(info) = self.established(grant, path, reading) {
                return Some(info);
            }
            if path
                .get(1)
                .is_some_and(|next| self.carriers.contains_key(next))
                && !self.first.paused(path, reading.instant)
            {
                return None;
            }
        }
        None
    }

    /// Start a handshake on the path [`Self::choose_path`] selects, or wait
    /// for an adjacency of some next hop (interest is registered).
    fn open(
        &mut self,
        grant: &CircuitGrant,
        paths: &[LogicalPath],
        reading: &View<'_>,
    ) -> Result<Option<PendingFrame>, Error> {
        let Some(index) = self.choose_at(paths.iter().map(|path| path.nodes.as_slice()), reading)
        else {
            return Ok(None);
        };
        let path = &paths[index].nodes;
        let carrier = self.carriers.get(&path[1]).cloned().ok_or(Error::State)?;
        self.start_at(grant.clone(), path.clone(), &carrier, reading)
            .map(Some)
    }

    /// The one rule choosing which of a decision's paths a new circuit uses.
    /// The candidates are ranked by what this Node knows of connectivity:
    /// the direct path stays first; every relayed path is ordered by (this
    /// Node has a channel with its next hop, that relay reports a channel with
    /// the Node after it), stably in Policy order otherwise; the first
    /// [`MAX_PATHS`] are kept. Among them it takes the first whose next hop has a carrier
    /// and whose last handshake did not end unanswered within its pause, else
    /// the first with a carrier. A preferred path whose next hop lacks a
    /// carrier (e.g. the direct one while a relay carries the circuit) still
    /// gets interest, so the next circuit can use it; with no carrier at all
    /// every kept next hop gets interest and `None` is returned. Established
    /// circuits keep their own path.
    pub fn choose_path<'p>(
        &mut self,
        paths: impl IntoIterator<Item = &'p [NodeId]>,
    ) -> Option<usize> {
        let desired = self.desired.clone();
        self.choose_at(paths, &View::of(&desired))
    }

    /// The relayed path a new circuit toward `peer` would take now, though
    /// none is established: the first ranked path of the content that relays,
    /// whose next hop has a carrier here and whose every relay reports a
    /// channel with the Node after it. Reporting only; a circuit is still
    /// admitted and chosen per flow.
    pub fn relay_route(&self, peer: NodeId) -> Option<Vec<NodeId>> {
        let paths = self
            .desired
            .compiler()
            .transit(self.desired.netmap().node_id, &Target::Mesh { node: peer })
            .ok()?;
        let paths: Vec<&[NodeId]> = paths.iter().map(|path| path.nodes.as_slice()).collect();
        let (ranked, _) = ranked(&paths, &self.carriers, &self.first.peer_channels);
        ranked
            .into_iter()
            .map(|index| paths[index])
            .find(|path| {
                path.len() > 2
                    && self.carriers.contains_key(&path[1])
                    && path[1..].windows(2).all(|hop| {
                        self.first
                            .peer_channels
                            .get(&hop[0])
                            .is_some_and(|connected| connected.contains(&hop[1]))
                    })
            })
            .map(<[NodeId]>::to_vec)
    }

    fn choose_at<'p>(
        &mut self,
        paths: impl IntoIterator<Item = &'p [NodeId]>,
        reading: &View<'_>,
    ) -> Option<usize> {
        let paths: Vec<&[NodeId]> = paths.into_iter().collect();
        let carried = |path: &[NodeId]| {
            path.get(1)
                .is_some_and(|next| self.carriers.contains_key(next))
        };
        let (ranked, truncated) = ranked(&paths, &self.carriers, &self.first.peer_channels);
        if truncated > 0 {
            tracing::debug!(operation = "path_candidates", target = ?paths[0].last(),
                kept = ranked.len(), truncated_relays = truncated,
                "Relay candidates beyond the kept ones are not tried");
        }
        let chosen = ranked
            .iter()
            .position(|&index| {
                carried(paths[index]) && !self.first.paused(paths[index], reading.instant)
            })
            .or_else(|| ranked.iter().position(|&index| carried(paths[index])));
        let preferred: Vec<NodeId> = ranked[..chosen.unwrap_or(ranked.len())]
            .iter()
            .map(|&index| paths[index])
            .filter(|path| !carried(path))
            .filter_map(|path| path.get(1).copied())
            .collect();
        for next in preferred {
            self.first.interest(next);
        }
        chosen.map(|position| ranked[position])
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
            let established = self.joinable(&grant, &waiting.paths, &reading);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn node(i: u8) -> NodeId {
        format!("00000000-0000-4000-8000-{i:012}").parse().unwrap()
    }

    fn carriers(local: NodeId, peers: &[NodeId]) -> BTreeMap<NodeId, super::super::Carrier> {
        peers
            .iter()
            .map(|peer| {
                (
                    *peer,
                    super::super::Carrier {
                        dialer: local,
                        listener: *peer,
                        listener_key: "udp".into(),
                        protocol: meshway_model::TransportProtocol::NativeUdp,
                    },
                )
            })
            .collect()
    }

    /// Policy order: the direct path, then one relay each in Node ID order.
    fn paths(source: NodeId, relays: &[NodeId], target: NodeId) -> Vec<Vec<NodeId>> {
        std::iter::once(vec![source, target])
            .chain(relays.iter().map(|relay| vec![source, *relay, target]))
            .collect()
    }

    #[test]
    fn a_relay_with_channels_to_both_ends_ranks_first_whatever_its_id() {
        // lax → planck: lax has channels with every relay; only it7 (the
        // highest ID, cut by the former four-path limit) reaches planck.
        let (lax, sha, vm, nyc, it7, planck) =
            (node(1), node(2), node(3), node(4), node(5), node(6));
        let paths = paths(lax, &[sha, vm, nyc, it7], planck);
        let refs: Vec<&[NodeId]> = paths.iter().map(Vec::as_slice).collect();
        let carriers = carriers(lax, &[sha, vm, nyc, it7]);
        let channels = BTreeMap::from([
            (sha, BTreeSet::from([lax, vm, nyc, it7])),
            (vm, BTreeSet::from([lax, sha, nyc, it7])),
            (nyc, BTreeSet::from([lax, sha, vm, it7])),
            (it7, BTreeSet::from([lax, sha, vm, nyc, planck])),
        ]);
        let (order, truncated) = ranked(&refs, &carriers, &channels);
        assert_eq!(truncated, 0);
        let order: Vec<&[NodeId]> = order.iter().map(|&index| refs[index]).collect();
        assert_eq!(
            order,
            [
                [lax, planck].as_slice(),
                &[lax, it7, planck],
                &[lax, sha, planck],
                &[lax, vm, planck],
                &[lax, nyc, planck],
            ]
        );
        // Without any reported channel the relays keep Policy order.
        let (order, _) = ranked(&refs, &carriers, &BTreeMap::new());
        assert_eq!(order, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn candidates_beyond_eight_are_cut_after_ranking_and_counted() {
        let (source, target) = (node(1), node(20));
        let relays: Vec<NodeId> = (2..=12).map(node).collect();
        let paths = paths(source, &relays, target);
        let refs: Vec<&[NodeId]> = paths.iter().map(Vec::as_slice).collect();
        // Channels with relays 10 and 11; relay 11 reports one with the
        // target, relay 12 too but this Node has no channel with it.
        let carriers = carriers(source, &[node(10), node(11)]);
        let channels = BTreeMap::from([
            (node(11), BTreeSet::from([target])),
            (node(12), BTreeSet::from([target])),
        ]);
        let (order, truncated) = ranked(&refs, &carriers, &channels);
        let next: Vec<NodeId> = order.iter().map(|&index| refs[index][1]).collect();
        assert_eq!(
            next,
            [
                target,
                node(11),
                node(10),
                node(12),
                node(2),
                node(3),
                node(4),
                node(5)
            ]
        );
        assert_eq!(order.len(), MAX_PATHS);
        assert_eq!(truncated, 4, "relays 6 to 9 are not tried");
    }
}
