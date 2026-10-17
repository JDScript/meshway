//! Desired netmap content: its idempotent application, the local
//! first-packet path and adjacency interest. No Server round trip.
use super::{
    CircuitGrant, Error, PendingFrame, SecureNode, State, View, authorization, crypto, pipeline,
};
use crate::{
    flow::{FlowTime, Keep},
    netmap::Outcome,
};
use meshway_configuration::execution::UnavailableReason;
use meshway_model::NodeId;
use meshway_policy::compile::netmap::Target;
use std::collections::{BTreeMap, BTreeSet};

mod diagnostic;
mod first;
mod outbound;
mod pause;
mod selection;

pub use diagnostic::diagnostic_paths;
pub(super) use first::FirstPackets;
pub(super) use pause::PathContent;

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

/// Result of [`SecureNode::preview`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    /// Through the Mesh toward `target` on `path` (this Node first);
    /// `established` when it joins an established circuit.
    Mesh {
        target: Target,
        path: Vec<NodeId>,
        established: bool,
    },
    Direct,
    Unmanaged,
    Denied(UnavailableReason),
}

/// Output of [`SecureNode::flush`].
#[derive(Debug)]
pub enum Flushed {
    Init(PendingFrame),
    Send(pipeline::SendJob),
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

    /// Exits whose `family` readiness fact is current and ready, and those
    /// whose fact is past its freshness bound but was last known ready;
    /// selection input only. Exit flows of the family go to the first ready
    /// exit of their ordered candidates, else the first last known ready
    /// one, and are refused (never sent direct) when there is neither; a DNS
    /// query to an exit resolver not so for IPv4 fails.
    pub fn set_ready_exits(
        &mut self,
        family: meshway_model::exit::ExitFamily,
        ready: BTreeSet<NodeId>,
        last_known: BTreeSet<NodeId>,
    ) {
        self.first.ready_exits.insert(family, ready);
        self.first.last_known_exits.insert(family, last_known);
    }

    /// Each peer's reported authenticated channels, from fresh peer facts:
    /// they only rank relay candidates (see [`Self::choose_path`]).
    pub fn set_peer_channels(&mut self, channels: BTreeMap<NodeId, BTreeSet<NodeId>>) {
        self.first.peer_channels = channels;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn node(i: u8) -> NodeId {
        format!("00000000-0000-4000-8000-{i:012}").parse().unwrap()
    }
}
