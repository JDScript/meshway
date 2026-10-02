//! Managed IPv4 boundaries. Encryption and exact path authentication belong to
//! Runtime. Mesh relays never receive plaintext; a planned final Linux exit
//! decrypts external packets before authorized kernel forwarding and NAT.

use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
};

use ipnet::Ipv4Net;
use meshway_dataplane::ReceivedPacket;
use meshway_dataplane::packet_io::{Ipv4PacketPlan, RoutePolicy};
use meshway_dataplane::{LabConfig, PeerConfig, native_udp_datagram_len};
use meshway_protocol::{ConnectionPeer, NodeMeshExecution};

use crate::PlanError;

#[derive(Clone)]
pub struct PacketPlan {
    pub node_id: String,
    pub revision: u32,
    pub digest: String,
    pub local: Ipv4Addr,
    pub prefix: Ipv4Net,
    pub mtu: u16,
    pub paths: Vec<Vec<String>>,
    identities: Vec<ConnectionPeer>,
    admission: Ipv4PacketPlan,
}

impl PacketPlan {
    /// A drop sink needs only the validated address binding, never access grants.
    pub fn sink(payload: &NodeMeshExecution, mtu: u16) -> Result<Self, PlanError> {
        Self::address_sink(
            &payload.node_id,
            payload.revision,
            meshway_protocol::mesh_execution_digest(payload)
                .map_err(|_| PlanError::Invalid("cannot hash Mesh packet generation"))?,
            &payload.network.ipv4_prefix,
            &payload.node.mesh_ipv4,
            mtu,
        )
    }

    fn address_sink(
        node_id: &str,
        revision: u32,
        digest: String,
        network: &str,
        address: &str,
        mtu: u16,
    ) -> Result<Self, PlanError> {
        let prefix: Ipv4Net = network
            .parse()
            .map_err(|_| PlanError::Invalid("invalid mesh IPv4 pool"))?;
        // A managed pool must never capture the Internet, loopback, multicast,
        // link-local control traffic, or a default route.
        let usable = |ip: Ipv4Addr| {
            !ip.is_unspecified()
                && !ip.is_loopback()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && !ip.is_link_local()
                && u32::from(ip) < u32::from(Ipv4Addr::new(224, 0, 0, 0))
        };
        if prefix.prefix_len() == 0 || !usable(prefix.network()) || !usable(prefix.broadcast()) {
            return Err(PlanError::Invalid(
                "mesh IPv4 pool cannot be safely installed",
            ));
        }
        let local = address
            .parse()
            .map_err(|_| PlanError::Invalid("invalid local mesh IPv4"))?;
        Ok(Self {
            node_id: node_id.to_owned(),
            revision,
            digest,
            local,
            prefix,
            mtu,
            paths: vec![],
            identities: vec![],
            admission: Ipv4PacketPlan::new(local, mtu, BTreeMap::new(), BTreeMap::new()),
        })
    }

    /// Install the controller's exact plan, without inferring reverse
    /// authorization or replanning from the inventory or physical adjacency.
    pub fn compile(payload: &NodeMeshExecution, mtu: u16) -> Result<Self, PlanError> {
        if !payload.valid || !payload.connections.valid {
            return Err(PlanError::Invalid("Mesh execution withdrawn"));
        }
        let mut plan = Self::sink(payload, mtu)?;
        let ips: BTreeMap<_, Ipv4Addr> = std::iter::once(&payload.node)
            .chain(&payload.peers)
            .map(|n| {
                Ok((
                    n.id.as_str(),
                    n.mesh_ipv4
                        .parse()
                        .map_err(|_| PlanError::Invalid("invalid Mesh IPv4"))?,
                ))
            })
            .collect::<Result<_, PlanError>>()?;
        let mut outgoing = BTreeMap::new();
        let mut incoming = BTreeMap::new();
        for path in &payload.paths {
            let path = &path.nodes;
            if !(2..=8).contains(&path.len())
                || !path.contains(&payload.node_id)
                || path.iter().collect::<std::collections::BTreeSet<_>>().len() != path.len()
                || path.iter().any(|id| !ips.contains_key(id.as_str()))
            {
                return Err(PlanError::Invalid("invalid exact Mesh path"));
            }
            if !native_udp_datagram_len(path, usize::from(mtu))
                .is_ok_and(|bytes| bytes + 48 <= 1500)
            {
                return Err(PlanError::Invalid("Mesh path exceeds outer MTU budget"));
            }
            if path.first() == Some(&payload.node_id)
                && outgoing
                    .insert(
                        ips[path.last().expect("checked path").as_str()],
                        path.clone(),
                    )
                    .is_some()
            {
                return Err(PlanError::Invalid(
                    "multiple Mesh paths for one destination",
                ));
            }
            if path.last() == Some(&payload.node_id) {
                incoming.insert(path.clone(), ips[path[0].as_str()]);
            }
        }
        plan.paths = payload.paths.iter().map(|p| p.nodes.clone()).collect();
        plan.identities = payload.identities.clone();
        plan.admission = Ipv4PacketPlan::new(plan.local, mtu, outgoing, incoming);
        Ok(plan)
    }

    pub fn augment(&self, config: &mut LabConfig) -> Result<(), PlanError> {
        for identity in &self.identities {
            if identity.node_id == self.node_id {
                continue;
            }
            if let Some(peer) = config
                .trusted_peers
                .iter()
                .find(|peer| peer.node_id == identity.node_id)
            {
                if peer.public_key != identity.public_key {
                    return Err(PlanError::Invalid(
                        "packet and adjacency identity bindings differ",
                    ));
                }
            } else {
                config.trusted_peers.push(PeerConfig {
                    node_id: identity.node_id.clone(),
                    public_key: identity.public_key.clone(),
                    transports: vec![],
                });
            }
        }
        config.allowed_paths = self.paths.clone();
        config.validate().map_err(|_| {
            PlanError::Invalid(
                "managed packet plan exceeds runtime authorization or resource limits",
            )
        })
    }

    pub fn route_policy(&self) -> RoutePolicy {
        RoutePolicy {
            local: self.local,
            network: self.prefix.network(),
            prefix_len: self.prefix.prefix_len(),
            mtu: self.mtu,
            retain_fail_closed_sink: true,
        }
    }

    pub fn check_targets(
        &self,
        targets: &BTreeMap<String, Vec<SocketAddr>>,
    ) -> Result<(), PlanError> {
        if targets
            .values()
            .flatten()
            .any(|address| in_prefix(address.ip(), self.prefix))
        {
            return Err(PlanError::Invalid(
                "resolved transport endpoint falls inside mesh IPv4 pool",
            ));
        }
        Ok(())
    }
}

impl meshway_dataplane::packet_io::Admission for PacketPlan {
    fn outgoing(&self, bytes: &[u8]) -> Option<&Vec<String>> {
        self.admission.outgoing(bytes)
    }
    fn incoming(&self, packet: &ReceivedPacket) -> bool {
        self.admission.incoming(packet)
    }
}

pub fn in_prefix(ip: IpAddr, prefix: Ipv4Net) -> bool {
    match ip {
        IpAddr::V4(ip) => prefix.contains(&ip),
        IpAddr::V6(ip) => ip.to_ipv4_mapped().is_some_and(|ip| prefix.contains(&ip)),
    }
}
