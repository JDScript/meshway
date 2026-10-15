//! Status view of the installed netmap's visible peers, derived from the
//! network owner's own facts: selected channels, established circuits,
//! adjacency needs, Control's last known peer facts and past measurements.
use chrono::{DateTime, Utc};
use meshway_configuration::netmap::NodeNetmap;
use meshway_model::{NodeId, state::AdjacencyStatus};
use meshway_protocol::ipc::{
    IdlePath, IdleRoute, NodeSelf, PeerMtu, PeerRtt, PeerState, PeerStatus,
};
use std::collections::BTreeMap;

pub(super) struct Inputs<'a> {
    pub netmap: &'a NodeNetmap,
    pub facts: Option<&'a meshway_connectivity::Snapshot>,
    pub routes: &'a BTreeMap<NodeId, Vec<NodeId>>,
    pub relays: &'a BTreeMap<NodeId, Vec<NodeId>>,
    pub idle_paths: &'a BTreeMap<NodeId, Option<Vec<NodeId>>>,
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
                idle_path: None,
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
            peer.idle_path = match peer.state {
                // The actual path is the channel's or the circuit's.
                PeerState::Direct | PeerState::Relay => None,
                // The relay a new circuit takes, the one `via` names.
                PeerState::RelayIdle => inputs
                    .relays
                    .get(id)
                    .map(|path| idle_path(Some(path.as_slice()))),
                // A failed adjacency without a relay reporting the peer, or
                // no current facts of the peer: no path a circuit can take.
                PeerState::Unreachable | PeerState::Offline => Some(idle_path(None)),
                PeerState::Idle => inputs
                    .idle_paths
                    .get(id)
                    .map(|path| idle_path(path.as_deref())),
            };
            peer
        })
        .collect();
    peers.sort_by(|a, b| a.hostname.cmp(&b.hostname));
    peers
}

/// A complete path from this Node (or none) as the Status reports it.
fn idle_path(path: Option<&[NodeId]>) -> IdlePath {
    match path {
        Some(path) if path.len() > 2 => IdlePath {
            route: IdleRoute::Relay,
            via: path[1..path.len() - 1].to_vec(),
        },
        Some(path) if path.len() == 2 => IdlePath {
            route: IdleRoute::Direct,
            via: Vec::new(),
        },
        _ => IdlePath {
            route: IdleRoute::None,
            via: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshway_test_server::configuration::{compile_one, network, node, sel, tag_node, trusted};

    fn netmap() -> std::sync::Arc<NodeNetmap> {
        let mut cfg = network("peers.internal");
        for (i, name) in [(1, "a"), (2, "b"), (3, "c"), (4, "d")] {
            cfg.nodes
                .insert(node(i), tag_node(name, i, BTreeMap::new(), false));
        }
        cfg.access.push(meshway_configuration::AccessRule {
            remark: String::new(),
            src: vec![sel(node(1))],
            dst: vec!["tag:server".into()],
            ip: vec!["*".into()],
            bidirectional: true,
        });
        let inventory = (1..=4).map(|i| (node(i), trusted(i, false))).collect();
        compile_one(
            "00000000-0000-4003-8000-000000000001".parse().unwrap(),
            &cfg,
            &inventory,
            node(1),
            None,
        )
        .netmap
    }

    /// Without a channel or circuit each peer shows the path a new circuit
    /// would take; a peer with a circuit shows only the circuit.
    #[test]
    fn peers_without_a_channel_or_circuit_show_the_new_circuit_path() {
        let netmap = netmap();
        let (a, b, c, d) = (node(1), node(2), node(3), node(4));
        let known = fresh(&[b, c, d], Utc::now());
        let idle_paths =
            BTreeMap::from([(b, Some(vec![a, b])), (c, Some(vec![a, b, c])), (d, None)]);
        let routes = BTreeMap::from([(d, vec![a, b, d])]);
        let status = peers(&Inputs {
            netmap: &netmap,
            facts: None,
            routes: &routes,
            relays: &BTreeMap::new(),
            idle_paths: &idle_paths,
            rtts: &BTreeMap::new(),
            known: Some(&known),
            now: Utc::now(),
        });
        let by = |id: NodeId| status.iter().find(|peer| peer.node_id == id).unwrap();
        assert_eq!(
            by(b).idle_path,
            Some(IdlePath {
                route: IdleRoute::Direct,
                via: vec![]
            })
        );
        assert_eq!(
            by(c).idle_path,
            Some(IdlePath {
                route: IdleRoute::Relay,
                via: vec![b]
            })
        );
        // d has an established circuit: its actual path, no idle path.
        assert_eq!(by(d).state, PeerState::Relay);
        assert_eq!(by(d).via, vec![b]);
        assert_eq!(by(d).idle_path, None);
        let routes = BTreeMap::new();
        let status = peers(&Inputs {
            netmap: &netmap,
            facts: None,
            routes: &routes,
            relays: &BTreeMap::new(),
            idle_paths: &idle_paths,
            rtts: &BTreeMap::new(),
            known: Some(&known),
            now: Utc::now(),
        });
        let d = status.iter().find(|peer| peer.node_id == d).unwrap();
        assert_eq!(
            d.idle_path,
            Some(IdlePath {
                route: IdleRoute::None,
                via: vec![]
            })
        );
        let json = serde_json::to_value(d).unwrap();
        assert_eq!(json["idlePath"]["route"], "none");
    }

    fn fresh(peers: &[NodeId], now: DateTime<Utc>) -> meshway_control::KnownPeers {
        meshway_control::KnownPeers {
            netmap: meshway_model::NetmapRef {
                revision: meshway_model::Revision::MIN,
                serial: meshway_model::Version::MIN,
                digest: meshway_model::Digest::from_bytes([1; 32]),
            },
            peers: std::sync::Arc::new(
                peers
                    .iter()
                    .map(|peer| {
                        (
                            *peer,
                            meshway_protocol::PeerFacts {
                                node_id: *peer,
                                listeners: BTreeMap::new(),
                                exit: None,
                                connected: Vec::new(),
                                observed_at: now,
                                expires_at: now + chrono::Duration::seconds(60),
                            },
                        )
                    })
                    .collect(),
            ),
            traversal_requests: Default::default(),
        }
    }

    /// Each state allows only the idle paths it means: none while a channel
    /// or circuit carries the peer, the relay `via` names for `relayIdle`,
    /// `none` for a peer no circuit can reach (unreachable, offline) whatever
    /// SecureNode would try, and SecureNode's path otherwise.
    #[test]
    fn every_state_shows_only_the_idle_paths_it_allows() {
        let netmap = netmap();
        let (a, b, c, d) = (node(1), node(2), node(3), node(4));
        let now = Utc::now();
        // SecureNode would open a relayed circuit toward each of them.
        let idle_paths = BTreeMap::from([
            (b, Some(vec![a, c, b])),
            (c, Some(vec![a, d, b, c])),
            (d, Some(vec![a, b, d])),
        ]);
        let routes = BTreeMap::new();
        // b: unreachable with fresh facts; c: offline (no current facts);
        // d: a relay reports a channel with it.
        let relays = BTreeMap::from([(d, vec![a, c, d])]);
        let facts = meshway_connectivity::Snapshot {
            listeners: Vec::new(),
            links: Vec::new(),
            sockets: Vec::new(),
            adjacencies: BTreeMap::from([
                (b, AdjacencyStatus::Unreachable),
                (d, AdjacencyStatus::Unreachable),
            ]),
            mtu: BTreeMap::new(),
            unavailable: Vec::new(),
        };
        let known = fresh(&[b, d], now);
        let status = peers(&Inputs {
            netmap: &netmap,
            facts: Some(&facts),
            routes: &routes,
            relays: &relays,
            idle_paths: &idle_paths,
            rtts: &BTreeMap::new(),
            known: Some(&known),
            now,
        });
        let by = |id: NodeId| status.iter().find(|peer| peer.node_id == id).unwrap();
        let none = Some(IdlePath {
            route: IdleRoute::None,
            via: vec![],
        });
        assert_eq!(
            (by(b).state, &by(b).idle_path),
            (PeerState::Unreachable, &none)
        );
        assert_eq!((by(c).state, &by(c).idle_path), (PeerState::Offline, &none));
        assert_eq!(by(d).state, PeerState::RelayIdle);
        assert_eq!(
            by(d).idle_path,
            Some(IdlePath {
                route: IdleRoute::Relay,
                via: by(d).via.clone(),
            })
        );
        assert_eq!(by(d).via, vec![c]);
        // Fresh facts and no failed adjacency: idle, SecureNode's path.
        let known = fresh(&[b, c, d], now);
        let status = peers(&Inputs {
            netmap: &netmap,
            facts: None,
            routes: &routes,
            relays: &BTreeMap::new(),
            idle_paths: &idle_paths,
            rtts: &BTreeMap::new(),
            known: Some(&known),
            now,
        });
        // Past their `expiresAt` (the reporter stopped, or no sync renewed
        // them) the same facts show `offline`.
        let stale = peers(&Inputs {
            netmap: &netmap,
            facts: None,
            routes: &routes,
            relays: &BTreeMap::new(),
            idle_paths: &idle_paths,
            rtts: &BTreeMap::new(),
            known: Some(&known),
            now: now + chrono::Duration::seconds(61),
        });
        assert!(
            stale
                .iter()
                .filter(|peer| peer.node_id != a)
                .all(|peer| peer.state == PeerState::Offline),
            "{stale:?}"
        );
        let c = status.iter().find(|peer| peer.node_id == c).unwrap();
        assert_eq!(c.state, PeerState::Idle);
        assert_eq!(
            c.idle_path,
            Some(IdlePath {
                route: IdleRoute::Relay,
                via: vec![d, b],
            })
        );
        for peer in &status {
            let allowed = match peer.state {
                PeerState::Direct | PeerState::Relay => peer.idle_path.is_none(),
                PeerState::Unreachable | PeerState::Offline => peer.idle_path == none,
                PeerState::RelayIdle => peer
                    .idle_path
                    .as_ref()
                    .is_some_and(|path| path.route == IdleRoute::Relay && path.via == peer.via),
                PeerState::Idle => peer.idle_path.is_some(),
            };
            assert!(allowed, "{peer:?}");
        }
    }
}
