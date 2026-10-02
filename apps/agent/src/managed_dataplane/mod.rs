//! Managed IPv4 boundaries. Encryption and exact path authentication belong to
//! Runtime. Mesh relays never receive plaintext; a planned final Linux exit
//! decrypts external packets before authorized kernel forwarding and NAT.

use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
};

use ipnet::Ipv4Net;
#[cfg(test)]
use meshway_dataplane::ReceivedPacket;
use meshway_dataplane::packet_io::{Ipv4PacketPlan, RoutePolicy};
use meshway_dataplane::{LabConfig, PeerConfig, native_udp_datagram_len};
use meshway_protocol::{ConnectionPeer, NodeDataplane};

use crate::AgentError;

#[cfg(test)]
pub(crate) mod exit;
pub(crate) mod packet;
#[cfg(test)]
pub(crate) mod tests;
#[cfg(all(test, target_os = "linux"))]
mod throughput;
pub(crate) mod tun;

#[derive(Clone, Debug)]
pub struct TunOptions {
    pub name: String,
    pub mtu: u16,
}

impl TunOptions {
    pub(crate) fn validate(&self) -> Result<(), AgentError> {
        if self.name.is_empty()
            || self.name.len() > 15
            || !self
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || !(576..=1400).contains(&self.mtu)
        {
            return Err(AgentError::Invalid(
                "invalid TUN name or IPv4 MTU (576–1400)",
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct PacketPlan {
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
    pub fn sink(payload: &NodeDataplane, mtu: u16) -> Result<Self, AgentError> {
        let configuration = &payload.configuration;
        Self::address_sink(
            &payload.node_id,
            payload.revision,
            meshway_protocol::dataplane_digest(payload)
                .map_err(|_| AgentError::Invalid("cannot hash managed packet generation"))?,
            &configuration.network.ipv4_prefix,
            &configuration.node.mesh_ipv4,
            mtu,
        )
    }

    pub fn sink_mesh(
        payload: &meshway_protocol::NodeMeshExecution,
        mtu: u16,
    ) -> Result<Self, AgentError> {
        Self::address_sink(
            &payload.node_id,
            payload.revision,
            meshway_protocol::mesh_execution_digest(payload)
                .map_err(|_| AgentError::Invalid("cannot hash Mesh packet generation"))?,
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
    ) -> Result<Self, AgentError> {
        let prefix: Ipv4Net = network
            .parse()
            .map_err(|_| AgentError::Invalid("invalid mesh IPv4 pool"))?;
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
            return Err(AgentError::Invalid(
                "mesh IPv4 pool cannot be safely installed",
            ));
        }
        let local = address
            .parse()
            .map_err(|_| AgentError::Invalid("invalid local mesh IPv4"))?;
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

    /// V3 installs the controller's exact plan directly. No legacy source policy
    /// interpretation, inferred reverse authorization, or adjacency replanning.
    pub fn compile_mesh(
        payload: &meshway_protocol::NodeMeshExecution,
        mtu: u16,
    ) -> Result<Self, AgentError> {
        if !payload.valid || !payload.connections.valid {
            return Err(AgentError::Invalid("Mesh execution withdrawn"));
        }
        let mut plan = Self::sink_mesh(payload, mtu)?;
        let ips: BTreeMap<_, Ipv4Addr> = std::iter::once(&payload.node)
            .chain(&payload.peers)
            .map(|n| {
                Ok((
                    n.id.as_str(),
                    n.mesh_ipv4
                        .parse()
                        .map_err(|_| AgentError::Invalid("invalid Mesh IPv4"))?,
                ))
            })
            .collect::<Result<_, AgentError>>()?;
        let mut outgoing = BTreeMap::new();
        let mut incoming = BTreeMap::new();
        for path in &payload.paths {
            let path = &path.nodes;
            if !(2..=8).contains(&path.len())
                || !path.contains(&payload.node_id)
                || path.iter().collect::<std::collections::BTreeSet<_>>().len() != path.len()
                || path.iter().any(|id| !ips.contains_key(id.as_str()))
            {
                return Err(AgentError::Invalid("invalid exact Mesh path"));
            }
            if !native_udp_datagram_len(path, usize::from(mtu))
                .is_ok_and(|bytes| bytes + 48 <= 1500)
            {
                return Err(AgentError::Invalid("Mesh path exceeds outer MTU budget"));
            }
            if path.first() == Some(&payload.node_id)
                && outgoing
                    .insert(
                        ips[path.last().expect("checked path").as_str()],
                        path.clone(),
                    )
                    .is_some()
            {
                return Err(AgentError::Invalid(
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

    /// Compile once from the coherent bundle, including at relay nodes. This
    /// deliberately never infers a path from physical adjacency or adds reverse grants.
    pub fn compile(payload: &NodeDataplane, mtu: u16) -> Result<Self, AgentError> {
        if !payload.access_valid
            || !payload.configuration.policy_valid
            || !payload.connections.valid
        {
            return Err(AgentError::Invalid("dataplane source intent is invalid"));
        }
        let mut plan = Self::sink(payload, mtu)?;
        let configuration = &payload.configuration;
        let ips: BTreeMap<_, Ipv4Addr> = std::iter::once(&configuration.node)
            .chain(&configuration.peers)
            .map(|node| {
                Ok((
                    node.id.as_str(),
                    node.mesh_ipv4
                        .parse()
                        .map_err(|_| AgentError::Invalid("invalid mesh node IPv4"))?,
                ))
            })
            .collect::<Result<_, AgentError>>()?;
        let paths =
            meshway_policy::access::compile_mesh_paths(configuration, &payload.access_policy)
                .map_err(|_| {
                    AgentError::Invalid("cannot compile mesh access and routing intersection")
                })?;
        let mut outgoing = BTreeMap::new();
        let mut incoming = BTreeMap::new();
        for path in &paths {
            if !path.contains(&payload.node_id) {
                return Err(AgentError::Invalid(
                    "dataplane grant exceeds local projection",
                ));
            }
            // Full enrolled IDs, not short lab labels. Conservatively budget for
            // either outer address family even when today's preferred hop is TCP.
            if !native_udp_datagram_len(path, usize::from(mtu))
                .is_ok_and(|bytes| bytes + 48 <= 1500)
            {
                return Err(AgentError::Invalid(
                    "TUN MTU exceeds the 1500-byte outer IPv6/UDP budget for an authorized full path; choose a smaller explicit MTU",
                ));
            }
            if path.first() == Some(&payload.node_id) {
                let destination = *ips
                    .get(path.last().expect("compiled full path").as_str())
                    .ok_or(AgentError::Invalid(
                        "path destination missing from inventory",
                    ))?;
                if outgoing.insert(destination, path.clone()).is_some() {
                    return Err(AgentError::Invalid("ambiguous mesh IPv4 destination path"));
                }
            }
            if path.last() == Some(&payload.node_id) {
                incoming.insert(
                    path.clone(),
                    *ips.get(path[0].as_str())
                        .ok_or(AgentError::Invalid("path source missing from inventory"))?,
                );
            }
        }
        plan.paths = paths;
        plan.identities = payload.identities.clone();
        plan.admission = Ipv4PacketPlan::new(plan.local, mtu, outgoing, incoming);
        Ok(plan)
    }

    pub fn augment(&self, config: &mut LabConfig) -> Result<(), AgentError> {
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
                    return Err(AgentError::Invalid(
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
            AgentError::Invalid(
                "managed packet plan exceeds runtime authorization or resource limits",
            )
        })
    }

    #[cfg(test)]
    pub fn outgoing(&self, bytes: &[u8]) -> Option<&Vec<String>> {
        self.admission.outgoing(bytes)
    }

    #[cfg(test)]
    pub fn incoming(&self, packet: &ReceivedPacket) -> bool {
        self.admission.incoming(packet)
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
    ) -> Result<(), AgentError> {
        if targets
            .values()
            .flatten()
            .any(|address| in_prefix(address.ip(), self.prefix))
        {
            return Err(AgentError::Invalid(
                "resolved transport endpoint falls inside mesh IPv4 pool",
            ));
        }
        Ok(())
    }
}

pub(crate) fn in_prefix(ip: IpAddr, prefix: Ipv4Net) -> bool {
    match ip {
        IpAddr::V4(ip) => prefix.contains(&ip),
        IpAddr::V6(ip) => ip.to_ipv4_mapped().is_some_and(|ip| prefix.contains(&ip)),
    }
}
