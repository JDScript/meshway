//! Netmap execution: in-place content updates, the local first-packet path
//! and adjacency interest. No demand, publication or Server round trip.
use super::{
    CircuitGrant, CircuitState, Error, PendingFrame, SecureNode, State, View, authorization,
    crypto, pipeline,
};
use crate::{
    FlowGrant, Packet, Purpose,
    authority::{Auth, Cap, netmap_addresses},
    engine::retain_legacy,
    flow::{FlowTime, Keep},
    netmap::Outcome,
};
use meshway_authority::DataPlaneIdentity;
use meshway_configuration::execution::{BusinessProtocol, LogicalPath, UnavailableReason};
use meshway_model::NodeId;
use meshway_policy::compile::netmap::{Target, observable};
use std::{
    collections::{BTreeSet, VecDeque},
    net::IpAddr,
    sync::Arc,
    time::Instant,
};

/// Packets one target may queue while its circuit or adjacency is pending.
const MAX_QUEUED: usize = 32;
/// Targets that may queue first packets at once.
const MAX_TARGETS: usize = 64;
/// Distinct peers of outstanding adjacency interest.
const MAX_INTERESTS: usize = 256;

/// What a content update did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Update {
    /// Same content, renewed lease: everything continues, nothing was scanned.
    Renewed,
    /// New content compared in place: unchanged circuits and flows keep all
    /// state; the counted ones were retired alone.
    Updated {
        retired_circuits: usize,
        retired_flows: usize,
    },
    /// The own identity, addresses or keys changed, or the old lease had
    /// ended: every circuit and flow was terminated.
    Reset,
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
    ready_exits: BTreeSet<NodeId>,
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

fn mesh_address(view: &View<'_>, node: NodeId) -> Option<IpAddr> {
    let netmap = view.netmap()?;
    netmap_addresses(&netmap.netmap, node)
        .and_then(|a| a.ipv4.map(IpAddr::V4).or(a.ipv6.map(IpAddr::V6)))
}

impl SecureNode {
    /// Install a Control-issued snapshot. Same netmap content (lease renewal)
    /// swaps the snapshot in O(1): every live circuit and flow keeps going and
    /// follows the renewed lease, no guard is cancelled. New content is
    /// compared in place: a circuit keeps its Noise state only while its path
    /// authorization digest and key set are unchanged; a flow only while its
    /// exact service, path and exit selection are still decided. Exit flows
    /// retired here publish Retire. A changed own identity, address or key set,
    /// or an ended old lease, terminates everything. Legacy targets keep their
    /// per-publication continuation rules.
    pub fn update(&mut self, identity: DataPlaneIdentity) -> Result<Update, Error> {
        let snapshot = identity.snapshot().clone();
        let old = self.identity.snapshot().clone();
        if !snapshot.is_netmap() && !old.is_netmap() {
            return self.update_legacy(identity);
        }
        if snapshot.is_netmap() && snapshot.same_netmap(&old) {
            View::of(&snapshot).map_err(|_| Error::Authority)?;
            self.engine.snapshot = snapshot;
            self.identity = identity;
            return Ok(Update::Renewed);
        }
        let reading = View::of(&snapshot).map_err(|_| Error::Authority)?;
        if !snapshot.is_netmap() || snapshot.netmap_continuation_from(&old).is_err() {
            self.terminate("reset");
            self.install(identity, &reading);
            return Ok(Update::Reset);
        }
        let Auth::Netmap { netmap, .. } = reading.auth else {
            return Err(Error::Authority);
        };
        let leased = reading.leased().ok_or(Error::Authority)?;
        let decisions = &mut self.engine.decisions;
        decisions.bind(netmap);
        let local = netmap.netmap.node_id;
        let mut retired_circuits = 0;
        for circuit in self.circuits.values_mut() {
            let CircuitGrant::Netmap { purpose, target } = &circuit.context.grant else {
                circuit.retire_for("update", "grant_kind");
                retired_circuits += 1;
                continue;
            };
            let path = &circuit.context.path;
            let position = path.iter().position(|node| *node == local);
            let admitted = position.and_then(|position| {
                authorization::admit_netmap(
                    decisions,
                    netmap,
                    leased,
                    reading.now_ms,
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
                    circuit.selected = admitted.selected;
                }
                _ => {
                    circuit.retire_for("update", "path_authorization_changed");
                    retired_circuits += 1;
                }
            }
        }
        let before = self.engine.flows.len();
        let time = FlowTime::from(&reading);
        let flows = &mut self.engine.flows;
        flows.retain(&time, |flow| {
            let FlowGrant::Netmap(target) = flow.grant else {
                return Keep::Retire;
            };
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
        self.install(identity, &reading);
        Ok(Update::Updated {
            retired_circuits,
            retired_flows,
        })
    }

    /// Swap in an accepted snapshot: drop correlations of the old one, keep
    /// only carriers the new content admits, and mint a new output section.
    fn install(&mut self, identity: DataPlaneIdentity, reading: &View<'_>) {
        let mut carriers = std::mem::take(&mut self.carriers);
        carriers.retain(|peer, carrier| {
            authorization::adjacency(&mut self.engine.decisions, reading, *peer, carrier)
        });
        self.carriers = carriers;
        self.probes.retain_current();
        self.engine.snapshot = identity.snapshot().clone();
        self.identity = identity;
        self.output_epoch = Arc::new(());
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

    /// Legacy: carry live compatible circuits and flows through an
    /// owner-authenticated direct publication replacement, in place.
    fn update_legacy(&mut self, identity: DataPlaneIdentity) -> Result<Update, Error> {
        let snapshot = identity.snapshot().clone();
        let reading = snapshot.reading().map_err(|_| Error::Authority)?;
        let old = snapshot
            .continuation_from(self.identity.snapshot())
            .map_err(|_| Error::Authority)?;
        let new = reading.configuration;
        let view = View::from(reading);
        let mut retired_circuits = 0;
        for circuit in self.circuits.values_mut() {
            let CircuitGrant::Legacy(authorization) = circuit.context.grant else {
                circuit.retire_for("reconcile", "grant_kind");
                retired_circuits += 1;
                continue;
            };
            let compatible = authorization::unchanged(
                old,
                new,
                authorization,
                &circuit.context.path,
                view.now_ms,
            )
            .unwrap_or(false);
            let expiry = meshway_policy::admission::circuit_permit(new, authorization)
                .ok()
                .map(|permit| permit.expires_at_ms);
            match expiry {
                Some(expiry) if compatible && circuit.check_at(&view).is_ok() => {
                    circuit.recovery.maintain();
                    circuit.permit_expires_at_ms = circuit.permit_expires_at_ms.max(expiry);
                    circuit.cap = Cap::until_ms(view.instant, view.now_ms, expiry).map(|permit| {
                        permit.min(Cap {
                            at: view.expires_at,
                            ms: view.expires_at_ms,
                        })
                    });
                }
                _ => {
                    circuit.retire_for("reconcile", "authorization_changed");
                    retired_circuits += 1;
                }
            }
        }
        let before = self.engine.flows.len();
        self.engine.flows.retain(&FlowTime::from(&view), |flow| {
            retain_legacy(old, new, flow, &view)
        });
        let retired_flows = before.saturating_sub(self.engine.flows.len());
        snapshot
            .continuation_from(self.identity.snapshot())
            .map_err(|_| Error::Authority)?;
        self.install(identity, &view);
        Ok(Update::Updated {
            retired_circuits,
            retired_flows,
        })
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
    /// are dropped (never sent direct) when none is ready.
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
        let snapshot = self.identity.snapshot().clone();
        let reading = View::of(&snapshot).map_err(|_| Error::Authority)?;
        let netmap = reading.netmap().ok_or(Error::Context)?;
        self.maintain_circuits(&reading, false)?;
        let local = reading.node();
        if let Some(flow) = self.local_flow_at(&packet, &reading)? {
            let FlowGrant::Netmap(target) = &flow.grant else {
                return Err(Error::Flow);
            };
            let grant = CircuitGrant::Netmap {
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
        let grant = CircuitGrant::Netmap {
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
        let grant = CircuitGrant::Netmap {
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
    /// is admitted afresh; expired ones are dropped and counted.
    pub fn flush(&mut self) -> Vec<Result<Flushed, Error>> {
        let snapshot = self.identity.snapshot().clone();
        let Ok(reading) = View::of(&snapshot) else {
            return Vec::new();
        };
        self.first.maintain(reading.instant);
        let mut outputs = Vec::new();
        for mut waiting in std::mem::take(&mut self.first.waiting) {
            let grant = CircuitGrant::Netmap {
                purpose: Purpose::Business,
                target: waiting.target.clone(),
            };
            let established = waiting
                .paths
                .iter()
                .find_map(|path| self.established(&grant, &path.nodes, &reading));
            if let Some(info) = established {
                while let Some(packet) = waiting.packets.pop_front() {
                    outputs.push(
                        self.prepare_send_at(info.id, packet, &reading)
                            .map(Flushed::Send),
                    );
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

    /// Open a diagnostic circuit toward a Mesh Node, entirely locally: the
    /// netmap must permit `icmpEcho` to it (else [`Error::Policy`]) and some
    /// permitted path, optionally an exact one, must be observable by every
    /// Node on it (else [`Error::Unsupported`]). `None` means no next hop has
    /// an admitted carrier yet; interest is registered.
    pub fn start_diagnostic(
        &mut self,
        target: NodeId,
        path: Option<&[NodeId]>,
    ) -> Result<Option<PendingFrame>, Error> {
        let snapshot = self.identity.snapshot().clone();
        let reading = View::of(&snapshot).map_err(|_| Error::Authority)?;
        let netmap = reading.netmap().ok_or(Error::Context)?;
        let local = reading.node();
        let address = mesh_address(&reading, target).ok_or(Error::Policy)?;
        let Outcome::Permitted(decision) = self
            .engine
            .decisions
            .decide(netmap, local, address, (BusinessProtocol::IcmpEcho, None))
            .map_err(|_| Error::Policy)?
        else {
            return Err(Error::Policy);
        };
        if decision.target != (Target::Mesh { node: target }) {
            return Err(Error::Policy);
        }
        let inventory = &netmap.netmap.projection.inventory;
        let paths: Vec<LogicalPath> = decision
            .paths
            .iter()
            .filter(|candidate| path.is_none_or(|path| candidate.nodes == path))
            .filter(|candidate| observable(inventory, &candidate.nodes))
            .cloned()
            .collect();
        if paths.is_empty() {
            return Err(Error::Unsupported);
        }
        let grant = CircuitGrant::Netmap {
            purpose: Purpose::Diagnostic,
            target: Target::Mesh { node: target },
        };
        self.open(&grant, &paths, &reading)
    }

    /// Established diagnostic circuit toward `target`, if any.
    pub fn diagnostic_circuit(&self, target: NodeId) -> Option<super::CircuitInfo> {
        let snapshot = self.identity.snapshot();
        let reading = View::of(snapshot).ok()?;
        let grant = CircuitGrant::Netmap {
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
