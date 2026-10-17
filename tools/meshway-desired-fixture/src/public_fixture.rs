//! Public, serializable compiler inputs shared by isolated benchmark processes.
//! There are no seeds here. Each process supplies only its own NodeMaterial.
use crate::{Error, NodeMaterial, StaticAuthority, random_uuid};
use meshway_configuration::{execution::*, netmap::CompiledNetmap, transport::PreparedListener, *};
use meshway_model::*;
use meshway_policy::SubjectContext;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicNode {
    pub id: NodeId,
    pub socket: SocketAddr,
    pub trusted: TrustedNode,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicFixture {
    pub network: NetworkId,
    pub configuration: NetworkConfiguration,
    pub inventory: BTreeMap<NodeId, TrustedNode>,
    pub path: Vec<NodeId>,
    pub sockets: BTreeMap<NodeId, SocketAddr>,
    pub carriers: Vec<Carrier>,
    /// Every Node's compiled netmap, checked again by each process.
    pub netmaps: BTreeMap<NodeId, NetmapRef>,
}

impl PublicFixture {
    /// Exact A→B or A→R→B iperf3 fixture. Input nodes are in traversal order.
    pub fn iperf(nodes: Vec<PublicNode>) -> Result<Self, Error> {
        if !(2..=3).contains(&nodes.len()) {
            return Err("iperf fixture requires 2 or 3 nodes".into());
        }
        let path: Vec<_> = nodes.iter().map(|node| node.id).collect();
        let mut configuration = new_network(
            AddressPools {
                ipv4: vec!["100.127.0.0/24".into()],
                ipv6: vec![],
            },
            "perf.internal".into(),
        )?;
        configuration.tags_owners.insert("tag:perf".into(), vec![]);
        for (i, node) in nodes.iter().enumerate() {
            let last = i + 1 == nodes.len();
            let address = Ipv4Addr::new(
                100,
                127,
                0,
                if i == 0 {
                    1
                } else if last {
                    2
                } else {
                    3
                },
            );
            let listener = serde_json::from_value(
                serde_json::json!({"type":"native_udp", "listen":node.socket.ip().to_string(), "listenPort":node.socket.port(),
                    "endpoints":[{"host":node.socket.ip().to_string(), "port":node.socket.port()}]}),
            )?;
            configuration.nodes.insert(
                node.id,
                NodeSettings {
                    hostname: format!("n{i}"),
                    remark: String::new(),
                    identity: NodeIdentity::Tag {
                        tags: vec!["tag:perf".into()],
                    },
                    enabled: true,
                    addresses: NodeAddresses {
                        ipv4: Some(address),
                        ipv6: None,
                    },
                    listeners: BTreeMap::from([(
                        crate::fixture::LISTENER_KEY.into(),
                        PreparedListener::prepare(listener)?,
                    )]),
                    dialers: Input::Omitted,
                    exit_enabled: false,
                    exit_families: meshway_configuration::default_exit_families(),
                    region: None,
                },
            );
        }
        let last = *path.last().ok_or("missing terminal")?;
        // Every fixture node can dial every other, so each hop would otherwise
        // be acquired on demand by the owner's first packet or a relayed
        // handshake. The benchmark drives SecureNode directly and measures the
        // established path, not acquisition: the measured hops are resident.
        configuration.topology = path
            .windows(2)
            .map(|pair| TopologyRule {
                remark: String::new(),
                action: TopologyAction::Allow,
                a: format!("node:{}", pair[0]),
                b: format!("node:{}", pair[1]),
                transports: Input::Omitted,
                keepalive: true,
            })
            .collect();
        configuration.access = vec![AccessRule {
            remark: String::new(),
            src: vec![format!("node:{}", path[0])],
            dst: vec![format!("node:{last}")],
            ip: vec!["tcp:5201".into(), "udp:5201".into()],
            bidirectional: false,
        }];
        configuration.routing.mesh = vec![MeshRoute {
            remark: String::new(),
            src: vec![format!("node:{}", path[0])],
            dst: vec![format!("node:{last}")],
            via: path[1..path.len() - 1]
                .iter()
                .map(|id| format!("node:{id}"))
                .collect(),
        }];
        let carriers = path
            .windows(2)
            .map(|pair| Carrier {
                dialer: pair[0],
                listener: pair[1],
                listener_key: crate::fixture::LISTENER_KEY.into(),
                protocol: TransportProtocol::NativeUdp,
            })
            .collect();
        let mut fixture = Self {
            network: NetworkId::new(random_uuid()?)?,
            configuration,
            inventory: nodes
                .iter()
                .map(|node| (node.id, node.trusted.clone()))
                .collect(),
            sockets: nodes.iter().map(|node| (node.id, node.socket)).collect(),
            path,
            carriers,
            netmaps: BTreeMap::new(),
        };
        fixture.netmaps = fixture
            .compile()?
            .iter()
            .map(|(id, netmap)| (*id, netmap.reference()))
            .collect();
        Ok(fixture)
    }

    /// The tagged fixture has no user subjects. The production netmap
    /// compiler decides every path; nothing is synthesized here.
    pub fn compile(&self) -> Result<BTreeMap<NodeId, Arc<CompiledNetmap>>, Error> {
        crate::compile_netmaps(
            self.network,
            &self.configuration,
            &SubjectContext::default(),
            &self.inventory,
        )
    }

    /// The terminal's Mesh IPv4 address.
    pub fn terminal_address(&self) -> Result<Ipv4Addr, Error> {
        let last = self.path.last().ok_or("missing terminal")?;
        self.configuration
            .nodes
            .get(last)
            .and_then(|node| node.addresses.ipv4)
            .ok_or_else(|| "terminal has no IPv4 address".into())
    }

    pub fn issue(&self, node: NodeId, material: NodeMaterial) -> Result<StaticAuthority, Error> {
        if self.inventory.get(&node) != Some(material.trusted()) {
            return Err("private material differs from public fixture inventory".into());
        }
        let netmaps = self.compile()?;
        let references: BTreeMap<_, _> = netmaps
            .iter()
            .map(|(id, netmap)| (*id, netmap.reference()))
            .collect();
        if references != self.netmaps {
            return Err("compiled netmaps differ from public fixture".into());
        }
        let origin = netmaps.get(&self.path[0]).ok_or("missing origin")?;
        let terminal = IpAddr::V4(self.terminal_address()?);
        for protocol in [BusinessProtocol::Tcp, BusinessProtocol::Udp] {
            crate::check_path(origin, self.path[0], terminal, (protocol, 5201), &self.path)?;
        }
        StaticAuthority::issue(netmaps, BTreeMap::from([(node, material)]))
    }

    pub fn carriers_for(&self, node: NodeId) -> Result<BTreeMap<NodeId, Carrier>, Error> {
        let index = self
            .path
            .iter()
            .position(|id| *id == node)
            .ok_or("unknown fixture node")?;
        if self.carriers.len() + 1 != self.path.len() {
            return Err("fixture carrier count differs from path".into());
        }
        let mut result = BTreeMap::new();
        if index > 0 {
            result.insert(self.path[index - 1], self.carriers[index - 1].clone());
        }
        if index + 1 < self.path.len() {
            result.insert(self.path[index + 1], self.carriers[index].clone());
        }
        Ok(result)
    }
}
