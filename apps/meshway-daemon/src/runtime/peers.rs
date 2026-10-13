//! Status view of the installed netmap's visible peers, derived from the
//! network owner's own facts: selected channels, established circuits,
//! adjacency needs, Control's last known peer facts and past measurements.
use chrono::{DateTime, Utc};
use meshway_configuration::netmap::NodeNetmap;
use meshway_model::{NodeId, state::AdjacencyStatus};
use meshway_protocol::ipc::{NodeSelf, PeerMtu, PeerRtt, PeerState, PeerStatus};
use std::collections::BTreeMap;

pub(super) struct Inputs<'a> {
    pub netmap: &'a NodeNetmap,
    pub facts: Option<&'a meshway_connectivity::Snapshot>,
    pub routes: &'a BTreeMap<NodeId, Vec<NodeId>>,
    pub relays: &'a BTreeMap<NodeId, Vec<NodeId>>,
    pub rtts: &'a BTreeMap<NodeId, PeerRtt>,
    pub known: Option<&'a meshway_control::KnownPeers>,
    pub now: DateTime<Utc>,
}

pub(super) fn own(netmap: &NodeNetmap) -> NodeSelf {
    NodeSelf {
        hostname: netmap.self_.hostname.clone(),
        addresses: netmap.self_.addresses.clone(),
    }
}

/// Visible peers in hostname order: the same set `ping` resolves names in.
pub(super) fn peers(inputs: &Inputs<'_>) -> Vec<PeerStatus> {
    let netmap = inputs.netmap;
    let link = |peer: NodeId| {
        inputs
            .facts
            .and_then(|facts| facts.links.iter().find(|link| link.peer == peer))
    };
    let mut peers: Vec<PeerStatus> = netmap
        .projection
        .configuration
        .nodes
        .iter()
        .filter(|(id, _)| **id != netmap.node_id && netmap.projection.inventory.contains_key(id))
        .map(|(id, node)| {
            let mut peer = PeerStatus {
                node_id: *id,
                hostname: node.hostname.clone(),
                addresses: node.addresses.clone(),
                state: PeerState::Idle,
                transport: None,
                endpoint: None,
                mtu: None,
                via: Vec::new(),
                rtt: inputs.rtts.get(id).cloned(),
            };
            if let Some(link) = link(*id) {
                peer.state = PeerState::Direct;
                peer.transport = Some(link.carrier.protocol);
                peer.endpoint = meshway_model::SocketAddress::try_from(link.remote).ok();
                peer.mtu = inputs
                    .facts
                    .and_then(|facts| facts.mtu.get(id))
                    .map(|mtu| PeerMtu {
                        path: mtu.path,
                        inner: mtu.inner,
                    });
            } else if let Some(path) = inputs.routes.get(id).filter(|path| path.len() > 2) {
                peer.state = PeerState::Relay;
                peer.via = path[1..path.len() - 1].to_vec();
                peer.transport = link(path[1]).map(|link| link.carrier.protocol);
            } else if let Some(path) = inputs.relays.get(id).filter(|path| path.len() > 2) {
                peer.state = PeerState::RelayIdle;
                peer.via = path[1..path.len() - 1].to_vec();
                peer.transport = link(path[1]).map(|link| link.carrier.protocol);
            } else if !inputs.known.is_some_and(|known| {
                known
                    .peers
                    .get(id)
                    .is_some_and(|facts| facts.expires_at > inputs.now)
            }) {
                peer.state = PeerState::Offline;
            } else if inputs.facts.is_some_and(|facts| {
                facts.adjacencies.get(id) == Some(&AdjacencyStatus::Unreachable)
            }) {
                peer.state = PeerState::Unreachable;
            }
            peer
        })
        .collect();
    peers.sort_by(|a, b| a.hostname.cmp(&b.hostname));
    peers
}
