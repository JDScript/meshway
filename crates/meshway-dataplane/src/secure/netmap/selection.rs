use super::super::{CircuitGrant, Error, PendingFrame, SecureNode, View};
use meshway_configuration::execution::LogicalPath;
use meshway_model::NodeId;
use meshway_policy::compile::{MAX_PATHS, netmap::Target};
use std::collections::{BTreeMap, BTreeSet};

/// The candidate order of [`SecureNode::choose_path`]: indices into `paths`
/// (in Policy order, the direct path first), at most [`MAX_PATHS`], and how
/// many relayed paths were cut. A relayed path ranks by whether this Node
/// has a channel with its next hop, then whether every relay on it reports
/// a channel with the Node after it (the whole way to the target, not only
/// the first relay's next hop); ties keep Policy order.
fn ranked(
    paths: &[&[NodeId]],
    carriers: &BTreeMap<NodeId, super::super::Carrier>,
    channels: &BTreeMap<NodeId, BTreeSet<NodeId>>,
) -> (Vec<usize>, usize) {
    let rank = |path: &[NodeId]| -> (bool, bool, bool) {
        let relayed = path.len() > 2;
        let carried = path.get(1).is_some_and(|next| carriers.contains_key(next));
        let onward = reported(path, channels);
        (relayed, !(relayed && carried), !(relayed && onward))
    };
    let mut order: Vec<usize> = (0..paths.len()).collect();
    order.sort_by_key(|&index| rank(paths[index]));
    let truncated = order.len().saturating_sub(MAX_PATHS);
    order.truncate(MAX_PATHS);
    (order, truncated)
}

/// Whether every relay of `path` reports a channel with the Node after it.
fn reported(path: &[NodeId], channels: &BTreeMap<NodeId, BTreeSet<NodeId>>) -> bool {
    path.len() > 2
        && path[1..].windows(2).all(|hop| {
            channels
                .get(&hop[0])
                .is_some_and(|connected| connected.contains(&hop[1]))
        })
}

impl SecureNode {
    pub(super) fn route_path(
        &self,
        grant: &CircuitGrant,
        paths: &[LogicalPath],
        reading: &View<'_>,
    ) -> Option<(Vec<NodeId>, bool)> {
        if let Some(info) = self.joinable(grant, paths, reading) {
            return Some((info.path, true));
        }
        let refs: Vec<&[NodeId]> = paths.iter().map(|path| path.nodes.as_slice()).collect();
        let index = self.fresh_route(&refs, reading)?;
        Some((refs[index].to_vec(), false))
    }

    /// The path a new circuit takes: [`Self::choose_path`]'s choice, else
    /// the first ranked candidate (no next hop has a carrier yet).
    fn fresh_route(&self, paths: &[&[NodeId]], reading: &View<'_>) -> Option<usize> {
        self.choice(paths, reading).0.or_else(|| {
            ranked(paths, &self.carriers, &self.first.peer_channels)
                .0
                .first()
                .copied()
        })
    }

    /// The path a new circuit toward `peer` would take now, though none is
    /// established ([`Self::fresh_route`] over the content's paths); `None`
    /// when the content gives no path. Whether that path reaches `peer` is
    /// the caller's peer state: an unreachable or offline peer has none.
    /// Reporting only: no interest is registered, and a circuit is still
    /// admitted and chosen per flow.
    pub fn idle_route(&self, peer: NodeId) -> Option<Vec<NodeId>> {
        let paths = self
            .desired
            .compiler()
            .transit(self.desired.netmap().node_id, &Target::Mesh { node: peer })
            .ok()?;
        let refs: Vec<&[NodeId]> = paths.iter().map(|path| path.nodes.as_slice()).collect();
        let index = self.fresh_route(&refs, &View::of(&self.desired))?;
        Some(refs[index].to_vec())
    }

    /// The established circuit a new flow joins: the first in the candidate
    /// order of [`Self::choose_path`], up to the path a new circuit would
    /// take there. A circuit on a lower-ranked path (a relay that lost the far
    /// end, while another one has channels to both) is not joined. Neither is
    /// a circuit whose next hop has no channel now (this Node roamed and is
    /// acquiring it again) while a later path's next hop has one: the new
    /// flow takes that path instead of waiting with the circuit. Without any
    /// carried path it still joins that circuit.
    pub(super) fn joinable(
        &self,
        grant: &CircuitGrant,
        paths: &[LogicalPath],
        reading: &View<'_>,
    ) -> Option<super::super::CircuitInfo> {
        let refs: Vec<&[NodeId]> = paths.iter().map(|path| path.nodes.as_slice()).collect();
        let (order, _) = ranked(&refs, &self.carriers, &self.first.peer_channels);
        let mut uncarried = None;
        for index in order {
            let path = refs[index];
            let carried = path
                .get(1)
                .is_some_and(|next| self.carriers.contains_key(next));
            if let Some(info) = self.established(grant, path, reading) {
                if carried {
                    return Some(info);
                }
                uncarried.get_or_insert(info);
                continue;
            }
            if carried && !self.first.paused(path, reading.instant) {
                return None;
            }
        }
        uncarried
    }

    /// Start a handshake on the path [`Self::choose_path`] selects, or wait
    /// for an adjacency of some next hop (interest is registered).
    pub(super) fn open(
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
                reported(path, &self.first.peer_channels) && self.carriers.contains_key(&path[1])
            })
            .map(<[NodeId]>::to_vec)
    }

    fn choose_at<'p>(
        &mut self,
        paths: impl IntoIterator<Item = &'p [NodeId]>,
        reading: &View<'_>,
    ) -> Option<usize> {
        let paths: Vec<&[NodeId]> = paths.into_iter().collect();
        let (chosen, preferred) = self.choice(&paths, reading);
        for next in preferred {
            self.first.interest(next);
        }
        chosen
    }

    /// [`Self::choose_path`]'s choice and the next hops it wants an
    /// adjacency with, without registering them.
    fn choice(&self, paths: &[&[NodeId]], reading: &View<'_>) -> (Option<usize>, Vec<NodeId>) {
        let carried = |path: &[NodeId]| {
            path.get(1)
                .is_some_and(|next| self.carriers.contains_key(next))
        };
        let (ranked, truncated) = ranked(paths, &self.carriers, &self.first.peer_channels);
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
        (chosen.map(|position| ranked[position]), preferred)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::node;
    use super::*;

    fn carriers(local: NodeId, peers: &[NodeId]) -> BTreeMap<NodeId, super::super::super::Carrier> {
        peers
            .iter()
            .map(|peer| {
                (
                    *peer,
                    super::super::super::Carrier {
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
    fn a_longer_relay_ranks_up_only_when_every_relay_reports_its_next_hop() {
        // it7 → planck while planck is offline: vm reports a channel with
        // lax, nobody one with planck. The two-relay path must not outrank
        // the one-relay ones for its first relay's channel alone.
        let (it7, vm, lax, sha, planck) = (node(5), node(3), node(1), node(2), node(6));
        let paths = [
            vec![it7, planck],
            vec![it7, sha, planck],
            vec![it7, vm, lax, planck],
        ];
        let refs: Vec<&[NodeId]> = paths.iter().map(Vec::as_slice).collect();
        let carriers = carriers(it7, &[vm, sha]);
        let mut channels = BTreeMap::from([(vm, BTreeSet::from([lax, it7]))]);
        let (order, _) = ranked(&refs, &carriers, &channels);
        assert_eq!(order, [0, 1, 2], "Policy order: nothing reaches planck");
        // Once lax reports planck the whole chain is known: it ranks first.
        channels.insert(lax, BTreeSet::from([planck]));
        let (order, _) = ranked(&refs, &carriers, &channels);
        assert_eq!(order, [0, 2, 1]);
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
