use super::super::{CircuitGrant, Error, SecureNode, State, View};
use super::{Flushed, Outbound, pause::Paused};
use crate::{Packet, Purpose};
use meshway_configuration::execution::LogicalPath;
use meshway_model::NodeId;
use meshway_policy::compile::netmap::Target;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::Instant,
};

/// Packets one target may queue while its circuit or adjacency is pending.
const MAX_QUEUED: usize = 32;
/// Targets that may queue first packets at once.
const MAX_TARGETS: usize = 64;
/// Distinct peers of outstanding adjacency interest.
const MAX_INTERESTS: usize = 256;

struct Waiting {
    target: Target,
    paths: Vec<LogicalPath>,
    packets: VecDeque<Packet>,
    until: Instant,
}

#[derive(Default)]
pub(in crate::secure) struct FirstPackets {
    waiting: Vec<Waiting>,
    pub(super) interests: BTreeSet<NodeId>,
    /// Exits whose readiness fact is current and ready: chosen first.
    pub ready_exits: BTreeSet<NodeId>,
    /// Exits whose fact is past its freshness bound and was last known
    /// ready: chosen only after every current one (selection, never authority).
    pub last_known_exits: BTreeSet<NodeId>,
    /// Paths whose origin handshake ended unanswered: consecutive count and
    /// the end of the pause during which another carried path is preferred.
    pub(super) unanswered: BTreeMap<Vec<NodeId>, Paused>,
    /// Peers' reported authenticated channels (peer facts): ranking only.
    pub peer_channels: BTreeMap<NodeId, BTreeSet<NodeId>>,
}

impl FirstPackets {
    /// The exit a new exit flow takes among the decision's ordered `exits`:
    /// the first currently ready one, else the first last known ready one.
    pub fn choose_exit<'a>(&self, exits: &'a [NodeId]) -> Option<&'a NodeId> {
        exits
            .iter()
            .find(|exit| self.ready_exits.contains(exit))
            .or_else(|| {
                exits
                    .iter()
                    .find(|exit| self.last_known_exits.contains(exit))
            })
    }
    /// The exit may be chosen at all (current or last known ready).
    pub fn exit_selectable(&self, exit: &NodeId) -> bool {
        self.ready_exits.contains(exit) || self.last_known_exits.contains(exit)
    }
    pub fn interest(&mut self, peer: NodeId) {
        if self.interests.len() < MAX_INTERESTS {
            self.interests.insert(peer);
        }
    }
    pub fn clear(&mut self) {
        self.waiting.clear();
        self.unanswered.clear();
    }
    pub fn maintain(&mut self, now: Instant) {
        self.waiting.retain(|waiting| now < waiting.until);
    }
    pub(super) fn queue(
        &mut self,
        target: &Target,
        paths: &[LogicalPath],
        packet: Packet,
        until: Instant,
    ) {
        if let Some(waiting) = self.waiting.iter_mut().find(|w| w.target == *target) {
            if waiting.packets.len() >= MAX_QUEUED {
                return;
            }
            waiting.packets.push_back(packet);
            return;
        }
        if self.waiting.len() >= MAX_TARGETS {
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

impl SecureNode {
    /// Queue a first packet behind a pending or new handshake on the first
    /// path whose next hop has an admitted carrier, or behind adjacency interest.
    pub(super) fn wait(
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

    /// Progress queued first packets: start handshakes whose adjacency became
    /// available and release packets whose circuit is established. Each packet
    /// is admitted afresh; expired ones are dropped. At most
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_currently_ready_exit_is_chosen_before_a_last_known_one() {
        let node = |n: u128| NodeId::new(uuid::Uuid::from_u128(n)).unwrap();
        let (e1, e2, e3) = (node(1), node(2), node(3));
        let mut first = FirstPackets {
            last_known_exits: [e1].into(),
            ..Default::default()
        };
        // e1 stopped reporting (stale), e2 is ready: failover to e2.
        first.ready_exits = [e2].into();
        assert_eq!(first.choose_exit(&[e1, e2]), Some(&e2));
        // Every fact is stale (no sync): the decision's order among them.
        first.ready_exits.clear();
        first.last_known_exits = [e1, e2].into();
        assert_eq!(first.choose_exit(&[e2, e1]), Some(&e2));
        // An exit known not ready, or never known, is not chosen.
        assert_eq!(first.choose_exit(&[e3]), None);
        assert!(!first.exit_selectable(&e3));
        assert!(first.exit_selectable(&e1));
    }
}
