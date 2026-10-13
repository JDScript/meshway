//! Adjacency input of one desired netmap: registered keys, own resources,
//! carrier admission and order. Built once per content, never per peer
//! worker. A carrier is admitted only by the projection (topology and both
//! sides' listener/dialer declarations) through Engine's shared compiler.
use crate::{Carrier, Error};
use meshway_configuration::{
    AddressPools,
    execution::{DialCandidate, DialEndpoint, DialEndpointSource, TrustedNode},
    netmap::NodeNetmap,
};
use meshway_engine::DataPlaneKeys;
use meshway_model::TransportProtocol;
use meshway_model::{BoundNodeKey, NetworkId, NodeId};
use meshway_transport::spec::PreparedListener;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

/// Peer material compared across contents; any change ends kept channels.
#[derive(Clone, Debug)]
struct Peer {
    control: BoundNodeKey,
    dataplane: BoundNodeKey,
    binding: TrustedNode,
}

type AdmissionKey = (NodeId, NodeId, String, TransportProtocol);

struct Projection {
    netmap: Arc<NodeNetmap>,
    compiler: Arc<meshway_policy::compile::netmap::LocalCompiler>,
    // Lazy per-content caches, bounded by the visible Nodes of this projection.
    directions: Mutex<BTreeMap<NodeId, Option<(bool, bool)>>>,
    keepalive: Mutex<BTreeMap<NodeId, bool>>,
    outgoing: Mutex<BTreeMap<NodeId, Arc<Vec<String>>>>,
    admitted: Mutex<BTreeMap<AdmissionKey, bool>>,
}

pub(crate) struct Plan {
    /// The desired content this plan was built from.
    pub desired: meshway_engine::Desired,
    pub reference: meshway_model::NetmapRef,
    pub network: NetworkId,
    pub node: NodeId,
    pub self_keys: TrustedNode,
    pub mesh_address_pools: AddressPools,
    pub listeners: BTreeMap<String, PreparedListener>,
    pub dialers: BTreeMap<String, DialCandidate>,
    peers: BTreeMap<NodeId, Peer>,
    projection: Projection,
}

impl Plan {
    pub fn new(desired: &meshway_engine::Desired) -> Result<Arc<Self>, Error> {
        let netmap = desired.netmap();
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
        let shared_netmap = netmap.clone();
        Ok(Arc::new(Self {
            desired: desired.clone(),
            reference: desired.reference(),
            network: netmap.network_id,
            node: netmap.node_id,
            self_keys: netmap.self_.self_keys.clone(),
            mesh_address_pools: netmap.self_.mesh_address_pools.clone(),
            listeners: netmap.self_.listeners.clone(),
            // Configured endpoints only; local endpoints are peer facts.
            dialers: netmap.dialers.clone(),
            peers,
            projection: Projection {
                compiler: desired.compiler().clone(),
                netmap: shared_netmap,
                directions: Mutex::default(),
                keepalive: Mutex::default(),
                outgoing: Mutex::default(),
                admitted: Mutex::default(),
            },
        }))
    }

    /// Resident regardless of interest: the topology rule deciding the edge
    /// asks for keepalive (whatever its selectors), or only one of the two
    /// Nodes can dial the other. A listener-only end
    /// has no channel through which to express a need, so the end that can
    /// dial holds the edge and the other keeps (and coordinates) it. Both
    /// ends evaluate the same projection. Every other adjacency is acquired
    /// on demand.
    pub fn resident(&self, peer: NodeId) -> bool {
        self.one_way(peer) || (self.adjacent(peer) && self.keepalive(peer))
    }

    /// The deciding topology rule's `keepalive`, cached per content.
    fn keepalive(&self, peer: NodeId) -> bool {
        let projection = &self.projection;
        if let Some(cached) = projection
            .keepalive
            .lock()
            .ok()
            .and_then(|cache| cache.get(&peer).copied())
        {
            return cached;
        }
        let keepalive = projection.compiler.keepalive(self.node, peer);
        if let Ok(mut cache) = projection.keepalive.lock() {
            cache.insert(peer, keepalive);
        }
        keepalive
    }

    fn one_way(&self, peer: NodeId) -> bool {
        self.directions(peer).is_some_and(|(out, back)| out != back)
    }

    /// Whether this Node and `peer` have an admitted carrier in some direction.
    pub fn adjacent(&self, peer: NodeId) -> bool {
        self.directions(peer).is_some_and(|(out, back)| out || back)
    }

    /// (this Node can dial `peer`, `peer` can dial this Node), cached per
    /// content. A compile error is not evidence of either direction.
    fn directions(&self, peer: NodeId) -> Option<(bool, bool)> {
        let projection = &self.projection;
        if peer == self.node || !self.peers.contains_key(&peer) {
            return None;
        }
        if let Some(cached) = projection
            .directions
            .lock()
            .ok()
            .and_then(|cache| cache.get(&peer).copied())
        {
            return cached;
        }
        let dials = |a, b| {
            projection
                .compiler
                .carriers(a, b)
                .map(|keys| !keys.is_empty())
        };
        let directions = match (dials(self.node, peer), dials(peer, self.node)) {
            (Ok(out), Ok(back)) => Some((out, back)),
            _ => None,
        };
        if let Ok(mut cache) = projection.directions.lock() {
            cache.insert(peer, directions);
        }
        directions
    }

    pub fn peers(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.peers.keys().copied()
    }

    pub fn has_peer(&self, peer: NodeId) -> bool {
        self.peers.contains_key(&peer)
    }

    /// Sign a packet-plane digest as this Node under its registered key.
    pub fn sign(
        &self,
        keys: &DataPlaneKeys,
        digest: meshway_model::Digest,
    ) -> Result<[u8; 64], Error> {
        keys.sign_packet(self.network, self.node, &self.self_keys.control_key, digest)
            .map_err(|_| Error::Authority)
    }

    /// The same own identity, keys, addresses and pools: resources and
    /// channels may survive between the two contents.
    pub fn same_self(&self, next: &Self) -> bool {
        let (old, new) = (&self.projection.netmap.self_, &next.projection.netmap.self_);
        self.network == next.network
            && self.node == next.node
            && self.self_keys == next.self_keys
            && self.mesh_address_pools == next.mesh_address_pools
            && old.identity == new.identity
            && old.addresses == new.addresses
            && old.enabled == new.enabled
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

    /// Adjacency admission of `carrier` with `peer` on this content.
    pub fn admit(&self, peer: NodeId, carrier: &Carrier) -> Result<(), Error> {
        {
            {
                let projection = &self.projection;
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
                Ok(())
            }
        }
    }

    /// The carrier an inbound `peer` may use on listener `key`, if admitted.
    pub fn accepting(
        &self,
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
        self.admit(peer, &carrier).ok().map(|_| carrier)
    }

    /// Every admitted inbound carrier on listener `key` (bounded by peers).
    pub fn incoming(&self, key: &str, protocol: TransportProtocol) -> Vec<(NodeId, Carrier)> {
        self.peers
            .keys()
            .filter_map(|peer| {
                self.accepting(*peer, key, protocol)
                    .map(|carrier| (*peer, carrier))
            })
            .collect()
    }

    /// Resource retention, by the Host's single rule (see
    /// [`meshway_underlay::netmap_resource_compatible`]).
    pub fn netmap_compatible(
        &self,
        next: &Self,
        resource: &meshway_model::state::ResourceRef,
    ) -> bool {
        meshway_underlay::netmap_resource_compatible(
            &self.projection.netmap,
            &next.projection.netmap,
            resource,
        )
    }
}

/// Peer listener endpoints from the last known facts, appended after
/// configured ones: interface endpoints, then for UDP the STUN-observed
/// mappings. Interface endpoints stay usable after the facts' freshness ends
/// (they live with the installed netmap until replaced or removed); a STUN
/// mapping is a NAT binding and is offered only before its own expiry. Facts
/// never admit a carrier; they only add addresses to try.
pub(crate) fn fact_endpoints(
    facts: &meshway_protocol::PeerFacts,
    key: &str,
    network: meshway_model::state::SocketNetwork,
    now_ms: i64,
) -> Vec<DialEndpoint> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use meshway_model::state::{
        EndpointCandidate, EndpointSource, ListenerPhase, LocalEndpoint, LocalEndpointSource,
        SocketNetwork,
    };
    use meshway_protocol::{PeerFacts, PeerListenerFacts};

    #[test]
    fn stale_facts_keep_interface_endpoints_and_drop_only_expired_mappings() {
        let at = |seconds| chrono::DateTime::from_timestamp(seconds, 0).unwrap();
        let mapping = |address: &str, expires| EndpointCandidate {
            address: address.to_owned(),
            origin: EndpointSource::Stun {
                observer: "stun.example:3478".to_owned(),
            },
            observed_at: at(0),
            expires_at: at(expires),
        };
        let facts = PeerFacts {
            node_id: "00000000-0000-4000-8000-000000000001".parse().unwrap(),
            listeners: BTreeMap::from([(
                "udp".to_owned(),
                PeerListenerFacts {
                    phase: ListenerPhase::Listening,
                    network: SocketNetwork::Udp,
                    local_endpoints: vec![LocalEndpoint {
                        address: "10.0.0.2:4000".parse().unwrap(),
                        source: LocalEndpointSource::Local,
                    }],
                    stun_candidates: vec![
                        mapping("192.0.2.1:5000", 30),
                        mapping("192.0.2.2:5000", 600),
                    ],
                },
            )]),
            exit: None,
            connected: Vec::new(),
            observed_at: at(0),
            expires_at: at(60),
        };
        let hosts = |now: i64| {
            fact_endpoints(&facts, "udp", SocketNetwork::Udp, now * 1000)
                .into_iter()
                .map(|endpoint| (endpoint.host, endpoint.source))
                .collect::<Vec<_>>()
        };
        let local = ("10.0.0.2".to_owned(), DialEndpointSource::Local);
        let live = ("192.0.2.2".to_owned(), DialEndpointSource::Stun);
        assert_eq!(
            hosts(10),
            [
                local.clone(),
                ("192.0.2.1".to_owned(), DialEndpointSource::Stun),
                live.clone()
            ]
        );
        assert_eq!(hosts(120), [local.clone(), live]);
        assert_eq!(hosts(3600), [local]);
        assert!(fact_endpoints(&facts, "udp", SocketNetwork::Tcp, 0).is_empty());
    }
}
