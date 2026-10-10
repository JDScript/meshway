//! Adjacency input of one authority epoch: registered keys, own resources,
//! carrier admission and order. Built once per snapshot content, never per
//! peer worker. A netmap is admitted only by its projection (topology and
//! both sides' listener/dialer declarations) and bounded by its lease.
use crate::{Carrier, Error};
use meshway_authority::ExecutionSnapshot;
use meshway_configuration::{
    AddressPools, TopologyAction, TopologyRule,
    execution::{DialCandidate, DialEndpoint, DialEndpointSource, TrustedNode},
    netmap::NodeNetmap,
};
use meshway_model::{BoundNodeKey, NetworkId, NodeId};
use meshway_transport_spec::{PreparedListener, TransportProtocol};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

/// Peer material compared across epochs; any change ends retained channels.
#[derive(Clone, Debug)]
struct Peer {
    control: BoundNodeKey,
    dataplane: BoundNodeKey,
    binding: TrustedNode,
}

type AdmissionKey = (NodeId, NodeId, String, TransportProtocol);

struct Projection {
    netmap: Arc<NodeNetmap>,
    compiler: meshway_policy::compile::netmap::LocalCompiler,
    keepalive: BTreeSet<NodeId>,
    // Lazy per-epoch caches, bounded by the visible Nodes of this projection.
    outgoing: Mutex<BTreeMap<NodeId, Arc<Vec<String>>>>,
    admitted: Mutex<BTreeMap<AdmissionKey, bool>>,
}

pub(crate) struct Plan {
    pub network: NetworkId,
    pub node: NodeId,
    pub self_keys: TrustedNode,
    pub mesh_address_pools: AddressPools,
    pub listeners: BTreeMap<String, PreparedListener>,
    pub dialers: BTreeMap<String, DialCandidate>,
    peers: BTreeMap<NodeId, Peer>,
    projection: Projection,
}

fn keepalive_rule(rule: &TopologyRule) -> bool {
    rule.action == TopologyAction::Allow && rule.keepalive
}

fn keepalive_peers(netmap: &NodeNetmap) -> BTreeSet<NodeId> {
    let own = format!("node:{}", netmap.node_id);
    netmap
        .projection
        .configuration
        .topology
        .iter()
        .filter(|rule| keepalive_rule(rule))
        .filter_map(|rule| {
            let other = if rule.a == own {
                &rule.b
            } else if rule.b == own {
                &rule.a
            } else {
                return None;
            };
            other.strip_prefix("node:")?.parse().ok()
        })
        .filter(|peer| *peer != netmap.node_id)
        .collect()
}

impl Plan {
    pub fn new(snapshot: &ExecutionSnapshot) -> Result<Arc<Self>, Error> {
        let netmap = &snapshot.netmap().map_err(|_| Error::Authority)?.netmap;
        let peers = netmap
            .projection
            .inventory
            .iter()
            .filter(|(id, _)| **id != netmap.node_id)
            .map(|(id, keys)| {
                (
                    *id,
                    Peer {
                        control: keys.control_key.clone(),
                        dataplane: keys.dataplane_key.clone(),
                        binding: keys.clone(),
                    },
                )
            })
            .collect();
        let shared_netmap = Arc::new(netmap.clone());
        Ok(Arc::new(Self {
            network: netmap.network_id,
            node: netmap.node_id,
            self_keys: netmap.self_.self_keys.clone(),
            mesh_address_pools: netmap.self_.mesh_address_pools.clone(),
            listeners: netmap.self_.listeners.clone(),
            // Configured endpoints only; local endpoints are peer facts.
            dialers: netmap.dialers.clone(),
            peers,
            projection: Projection {
                keepalive: keepalive_peers(netmap),
                compiler: meshway_policy::compile::netmap::LocalCompiler::from_shared(
                    shared_netmap.clone(),
                ),
                netmap: shared_netmap,
                outgoing: Mutex::default(),
                admitted: Mutex::default(),
            },
        }))
    }

    /// Resident regardless of interest: peers whose topology edge asks for
    /// keepalive. Every other adjacency is acquired on demand.
    pub fn keepalive(&self, peer: NodeId) -> bool {
        self.projection.keepalive.contains(&peer)
    }

    pub fn peers(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.peers.keys().copied()
    }

    pub fn has_peer(&self, peer: NodeId) -> bool {
        self.peers.contains_key(&peer)
    }

    pub fn keys(&self, node: NodeId) -> Result<(&BoundNodeKey, &BoundNodeKey), Error> {
        if node == self.node {
            return Ok((&self.self_keys.control_key, &self.self_keys.dataplane_key));
        }
        let peer = self.peers.get(&node).ok_or(Error::Authentication)?;
        Ok((&peer.control, &peer.dataplane))
    }

    /// Same registered keys and the same outgoing carrier order.
    pub fn peer_unchanged(&self, next: &Self, peer: NodeId) -> bool {
        let same = self.peers.get(&peer).map(|p| &p.binding)
            == next.peers.get(&peer).map(|p| &p.binding)
            && next.peers.contains_key(&peer);
        same && self.outgoing_keys(peer).ok() == next.outgoing_keys(peer).ok()
    }

    fn outgoing_keys(&self, peer: NodeId) -> Result<Arc<Vec<String>>, Error> {
        let projection = &self.projection;
        if let Some(keys) = projection
            .outgoing
            .lock()
            .map_err(|_| Error::Worker)?
            .get(&peer)
        {
            return Ok(keys.clone());
        }
        let compiler = &projection.compiler;
        let keys = Arc::new(
            compiler
                .carriers(self.node, peer)
                .map_err(|_| Error::Configuration)?,
        );
        projection
            .outgoing
            .lock()
            .map_err(|_| Error::Worker)?
            .insert(peer, keys.clone());
        Ok(keys)
    }

    /// Dial candidates toward `peer`, in the edge's transport order.
    pub fn outgoing(&self, peer: NodeId) -> Result<Vec<(&String, &DialCandidate)>, Error> {
        let keys = self.outgoing_keys(peer)?;
        Ok(keys
            .iter()
            .filter_map(|key| self.dialers.get_key_value(&format!("{peer}/{key}")))
            .filter(|(_, dial)| dial.peer == peer)
            .collect())
    }

    /// Peers this Node may dial; a candidate may gain local endpoints from facts.
    pub fn dial_peers(&self) -> BTreeSet<NodeId> {
        self.dialers.values().map(|dial| dial.peer).collect()
    }

    /// Adjacency admission of `carrier` with `peer` under `snapshot`, which
    /// must be this plan's own snapshot. Returns the cutoff in UTC ms.
    pub fn admit(
        &self,
        snapshot: &ExecutionSnapshot,
        peer: NodeId,
        carrier: &Carrier,
    ) -> Result<u64, Error> {
        {
            {
                let projection = &self.projection;
                snapshot.ensure_current().map_err(|_| Error::Authority)?;
                let key = (
                    peer,
                    carrier.dialer,
                    carrier.listener_key.clone(),
                    carrier.protocol,
                );
                let cached = projection
                    .admitted
                    .lock()
                    .map_err(|_| Error::Worker)?
                    .get(&key)
                    .copied();
                let admitted = match cached {
                    Some(admitted) => admitted,
                    None => {
                        let admitted = meshway_policy::admission::netmap::admit_adjacency(
                            &projection.compiler,
                            &projection.netmap,
                            peer,
                            carrier,
                        )
                        .is_ok();
                        projection
                            .admitted
                            .lock()
                            .map_err(|_| Error::Worker)?
                            .insert(key, admitted);
                        admitted
                    }
                };
                if !admitted || (carrier.dialer != self.node && carrier.listener != self.node) {
                    return Err(Error::Authority);
                }
                Ok(snapshot.expires_at_ms())
            }
        }
    }

    /// The carrier an inbound `peer` may use on listener `key`, if admitted.
    pub fn accepting(
        &self,
        snapshot: &ExecutionSnapshot,
        peer: NodeId,
        key: &str,
        protocol: TransportProtocol,
    ) -> Option<Carrier> {
        let carrier = Carrier {
            dialer: peer,
            listener: self.node,
            listener_key: key.to_owned(),
            protocol,
        };
        self.admit(snapshot, peer, &carrier).ok().map(|_| carrier)
    }

    /// Every admitted inbound carrier on listener `key` (bounded by peers).
    pub fn incoming(
        &self,
        snapshot: &ExecutionSnapshot,
        key: &str,
        protocol: TransportProtocol,
    ) -> Vec<(NodeId, Carrier)> {
        self.peers
            .keys()
            .filter_map(|peer| {
                self.accepting(snapshot, *peer, key, protocol)
                    .map(|carrier| (*peer, carrier))
            })
            .collect()
    }

    /// Resource retention: equal declaration, client material, configured
    /// endpoints and peer keys.
    pub fn netmap_compatible(
        &self,
        next: &Self,
        resource: &meshway_model::state::ResourceRef,
    ) -> bool {
        use meshway_model::state::ResourceRef;
        match resource {
            ResourceRef::Listener { key } => self
                .listeners
                .get(key)
                .is_some_and(|old| next.listeners.get(key) == Some(old)),
            ResourceRef::Dial { key } => self.dialers.get(key).is_some_and(|old| {
                next.dialers.get(key).is_some_and(|new| {
                    old.peer == new.peer
                        && old.listener_key == new.listener_key
                        && old.parameters == new.parameters
                        && old.endpoints == new.endpoints
                        && self.peers.get(&old.peer).map(|p| &p.binding)
                            == next.peers.get(&new.peer).map(|p| &p.binding)
                })
            }),
            ResourceRef::Coordination { .. } => false,
        }
    }
}

/// Peer listener endpoints from the latest facts, appended after configured
/// ones: interface endpoints, then for UDP the STUN-observed mappings. Facts
/// never admit a carrier; they only add addresses to try.
pub(crate) fn fact_endpoints(
    facts: &meshway_protocol::PeerFacts,
    key: &str,
    network: meshway_model::state::SocketNetwork,
    now_ms: i64,
) -> Vec<DialEndpoint> {
    if facts.expires_at.timestamp_millis() <= now_ms {
        return Vec::new();
    }
    let Some(listener) = facts.listeners.get(key) else {
        return Vec::new();
    };
    if listener.network != network {
        return Vec::new();
    }
    let local = listener
        .local_endpoints
        .iter()
        .take(meshway_model::state::MAX_LOCAL_ENDPOINTS)
        .filter_map(|endpoint| {
            Some(DialEndpoint {
                host: endpoint.address.ip().to_string(),
                port: endpoint.address.port().try_into().ok()?,
                source: DialEndpointSource::Local,
            })
        });
    let stun = listener
        .stun_candidates
        .iter()
        .filter(|_| network == meshway_model::state::SocketNetwork::Udp)
        .filter(|candidate| candidate.expires_at.timestamp_millis() > now_ms)
        .take(meshway_model::state::MAX_LOCAL_ENDPOINTS)
        .filter_map(|candidate| {
            let address: std::net::SocketAddr = candidate.address.parse().ok()?;
            Some(DialEndpoint {
                host: address.ip().to_string(),
                port: address.port().try_into().ok()?,
                source: DialEndpointSource::Stun,
            })
        });
    local.chain(stun).collect()
}
