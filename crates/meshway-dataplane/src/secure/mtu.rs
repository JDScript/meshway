//! Path MTU of this Node's own packets. The connectivity Manager measures
//! each selected Native UDP channel's inner limit (`Snapshot.mtu`); the
//! owner gets them each turn like its carriers. A local packet larger than
//! the limit of the channel to its next hop is dropped before any nonce is
//! used and answered on this Node's TUN with fragmentation needed / packet
//! too big, so its sender's kernel learns the path MTU. The same limits give
//! the TCP MSS clamp the Underlay installs ([`SecureNode::clamps`]).
use super::*;
use std::{collections::VecDeque, net::IpAddr};

/// Locally reported errors waiting for TUN delivery.
const MAX_ERRORS: usize = 32;
/// Locally reported errors per second, all flows together.
const ERRORS_PER_SECOND: u32 = 32;

#[derive(Default)]
pub(super) struct PathMtu {
    /// Inner limit (largest inner packet) of the selected channel to each
    /// adjacent peer whose limit is known.
    limits: BTreeMap<NodeId, u16>,
    errors: VecDeque<crate::PendingAction>,
    window: Option<(std::time::Instant, u32)>,
}

impl PathMtu {
    pub fn limit(&self, next: NodeId) -> Option<usize> {
        self.limits.get(&next).map(|limit| usize::from(*limit))
    }

    /// Queue the error for a packet that does not fit, within the budget.
    pub fn report(&mut self, error: crate::PendingAction, now: std::time::Instant) -> bool {
        let (start, count) = self
            .window
            .filter(|(start, _)| now.duration_since(*start) < Duration::from_secs(1))
            .unwrap_or((now, 0));
        if count >= ERRORS_PER_SECOND || self.errors.len() >= MAX_ERRORS {
            self.window = Some((start, count));
            return false;
        }
        self.window = Some((start, count + 1));
        self.errors.push_back(error);
        true
    }
}

pub use meshway_tun::PathLimits as Clamps;

impl SecureNode {
    /// Replace the measured inner limits of the current channels. Packets
    /// already admitted keep their outputs.
    pub fn set_path_limits(&mut self, limits: BTreeMap<NodeId, u16>) {
        self.mtu.limits = limits;
    }

    /// Errors reported for this Node's own packets since the last call, each
    /// bound to its flow's lifetime; the host delivers them to the TUN like
    /// any admitted packet.
    pub fn take_local_errors(&mut self) -> Vec<crate::PendingAction> {
        self.mtu.errors.drain(..).collect()
    }

    /// The MSS clamp of the current channels. A peer reached through its own
    /// channel gets that channel's limit; any other peer, reached through
    /// some relay, the smallest known limit; destinations outside the Mesh
    /// the smallest limit toward a ready exit. With no limit known nothing is
    /// clamped.
    pub fn clamps(&self) -> Clamps {
        let smallest = self.mtu.limits.values().min().copied();
        let toward = |peer: &NodeId| {
            if self.carriers.contains_key(peer) {
                self.mtu.limits.get(peer).copied()
            } else {
                smallest
            }
        };
        let netmap = self.desired.netmap();
        let local = netmap.node_id;
        let mut peers = BTreeMap::new();
        for (id, node) in &netmap.projection.configuration.nodes {
            if *id == local {
                continue;
            }
            let Some(limit) = toward(id) else {
                continue;
            };
            for address in [
                node.addresses.ipv4.map(IpAddr::V4),
                node.addresses.ipv6.map(IpAddr::V6),
            ]
            .into_iter()
            .flatten()
            {
                peers.insert(address, limit);
            }
        }
        Clamps {
            peers,
            other: self.first.ready_exits.iter().filter_map(toward).min(),
        }
    }
}
