//! Adjacency input of one authority epoch: registered keys, own resources,
//! carrier admission and order. Built once per snapshot content, never per
//! peer worker. A legacy publication keeps its time-bounded per-permit
//! admission; a netmap is admitted only by its projection (topology and both
//! sides' listener/dialer declarations) and bounded by its lease.
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
#[derive(Clone, Debug, PartialEq, Eq)]
enum PeerBinding {
    Legacy(Box<meshway_configuration::execution::PeerConfiguration>),
    Netmap(TrustedNode),
}

#[derive(Clone, Debug)]
struct Peer {
    control: BoundNodeKey,
    dataplane: BoundNodeKey,
    binding: PeerBinding,
}

type AdmissionKey = (NodeId, NodeId, String, TransportProtocol);

struct Projection {
    netmap: NodeNetmap,
    keepalive: BTreeSet<NodeId>,
    // Lazy per-epoch caches, bounded by the visible Nodes of this projection.
    outgoing: Mutex<BTreeMap<NodeId, Arc<Vec<String>>>>,
    admitted: Mutex<BTreeMap<AdmissionKey, bool>>,
}

enum Source {
    Legacy,
    Netmap(Box<Projection>),
}

pub(crate) struct Plan {
    pub network: NetworkId,
    pub node: NodeId,
    pub self_keys: TrustedNode,
    pub mesh_address_pools: AddressPools,
    pub listeners: BTreeMap<String, PreparedListener>,
    pub dialers: BTreeMap<String, DialCandidate>,
    peers: BTreeMap<NodeId, Peer>,
    source: Source,
}

/// Placeholder for the per-edge `keepalive` flag of a topology allow rule,
/// which the configuration model does not carry yet. Production returns
/// false (every adjacency is on demand); unit tests mark a rule through its
/// remark. Replace with the configuration field when it exists.
fn keepalive_rule(rule: &TopologyRule) -> bool {
    cfg!(test) && rule.remark == "test:keepalive"
}

fn keepalive_peers(netmap: &NodeNetmap) -> BTreeSet<NodeId> {
    let own = format!("node:{}", netmap.node_id);
    netmap
        .projection
        .configuration
        .topology
        .iter()
        .filter(|rule| rule.action == TopologyAction::Allow && keepalive_rule(rule))
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
        if snapshot.is_netmap() {
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
                            binding: PeerBinding::Netmap(keys.clone()),
                        },
                    )
                })
                .collect();
            return Ok(Arc::new(Self {
                network: netmap.network_id,
                node: netmap.node_id,
                self_keys: netmap.self_.self_keys.clone(),
                mesh_address_pools: netmap.self_.mesh_address_pools.clone(),
                listeners: netmap.self_.listeners.clone(),
                // Configured endpoints only; local endpoints are peer facts.
                dialers: netmap.dialers.clone(),
                peers,
                source: Source::Netmap(Box::new(Projection {
                    keepalive: keepalive_peers(netmap),
                    netmap: netmap.clone(),
                    outgoing: Mutex::default(),
                    admitted: Mutex::default(),
                })),
            }));
        }
        let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
        Ok(Arc::new(Self {
            network: cfg.network_id,
            node: cfg.node_id,
            self_keys: cfg.self_keys.clone(),
            mesh_address_pools: cfg.mesh_address_pools.clone(),
            listeners: cfg.listeners.clone(),
            dialers: cfg.dialers.clone(),
            peers: cfg
                .peers
                .iter()
                .map(|(id, peer)| {
                    (
                        *id,
                        Peer {
                            control: peer.control_key.clone(),
                            dataplane: peer.dataplane_key.clone(),
                            binding: PeerBinding::Legacy(Box::new(peer.clone())),
                        },
                    )
                })
                .collect(),
            source: Source::Legacy,
        }))
    }

    /// Adjacencies are acquired only for interested peers and close when idle.
    pub fn on_demand(&self) -> bool {
        matches!(self.source, Source::Netmap(_))
    }

    /// Resident regardless of interest: every legacy peer, and netmap peers
    /// whose topology edge asks for keepalive.
    pub fn keepalive(&self, peer: NodeId) -> bool {
        match &self.source {
            Source::Legacy => true,
            Source::Netmap(projection) => projection.keepalive.contains(&peer),
        }
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

    /// Same registered keys and, for a netmap, the same outgoing carrier order.
    pub fn peer_unchanged(&self, next: &Self, peer: NodeId) -> bool {
        let same = self.peers.get(&peer).map(|p| &p.binding)
            == next.peers.get(&peer).map(|p| &p.binding)
            && next.peers.contains_key(&peer);
        same && (!self.on_demand()
            || self.outgoing_keys(peer).ok() == next.outgoing_keys(peer).ok())
    }

    fn outgoing_keys(&self, peer: NodeId) -> Result<Arc<Vec<String>>, Error> {
        let Source::Netmap(projection) = &self.source else {
            return Err(Error::Configuration);
        };
        if let Some(keys) = projection
            .outgoing
            .lock()
            .map_err(|_| Error::Worker)?
            .get(&peer)
        {
            return Ok(keys.clone());
        }
        let compiler = meshway_policy::compile::netmap::LocalCompiler::new(&projection.netmap);
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

    /// Dial candidates toward `peer`, in carrier order: the edge's transport
    /// order for a netmap, `(priority, key)` for a publication.
    pub fn outgoing(&self, peer: NodeId) -> Result<Vec<(&String, &DialCandidate)>, Error> {
        if self.on_demand() {
            let keys = self.outgoing_keys(peer)?;
            return Ok(keys
                .iter()
                .filter_map(|key| self.dialers.get_key_value(&format!("{peer}/{key}")))
                .filter(|(_, dial)| dial.peer == peer)
                .collect());
        }
        let mut candidates: Vec<_> = self
            .dialers
            .iter()
            .filter(|(_, dial)| dial.peer == peer)
            .collect();
        candidates.sort_by(|a, b| (a.1.priority, a.0).cmp(&(b.1.priority, b.0)));
        Ok(candidates)
    }

    /// Peers this Node may dial. Publications list endpoints in the
    /// candidate; a netmap candidate may gain local endpoints from facts.
    pub fn dial_peers(&self) -> BTreeSet<NodeId> {
        self.dialers
            .values()
            .filter(|dial| self.on_demand() || !dial.endpoints.is_empty())
            .map(|dial| dial.peer)
            .collect()
    }

    /// Adjacency admission of `carrier` with `peer` under `snapshot`, which
    /// must be this plan's own snapshot. Returns the cutoff in UTC ms.
    pub fn admit(
        &self,
        snapshot: &ExecutionSnapshot,
        peer: NodeId,
        carrier: &Carrier,
    ) -> Result<u64, Error> {
        match &self.source {
            Source::Legacy => meshway_policy::admission::admit_adjacency(
                snapshot.configuration().map_err(|_| Error::Authority)?,
                peer,
                carrier,
                snapshot.now_ms().map_err(|_| Error::Authority)?,
            )
            .map_err(|_| Error::Authority),
            Source::Netmap(projection) => {
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
                        let compiler =
                            meshway_policy::compile::netmap::LocalCompiler::new(&projection.netmap);
                        let admitted = meshway_policy::admission::netmap::admit_adjacency(
                            &compiler,
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

    /// Netmap resource retention: equal declaration, client material,
    /// configured endpoints and peer keys. Publications use the underlay rule.
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
/// ones. Facts never admit a carrier; they only add addresses to try.
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
    listener
        .local_endpoints
        .iter()
        .take(meshway_model::state::MAX_LOCAL_ENDPOINTS)
        .filter_map(|endpoint| {
            Some(DialEndpoint {
                host: endpoint.address.ip().to_string(),
                port: endpoint.address.port().try_into().ok()?,
                source: DialEndpointSource::Local,
            })
        })
        .collect()
}
