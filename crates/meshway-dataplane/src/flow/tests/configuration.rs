use super::super::*;
use crate::{Action, AuthenticatedPeer, Packet};
use meshway_configuration::execution::*;
use meshway_configuration::*;
use meshway_model::state::{Capability, StreamKind, TransportRole};
use meshway_model::{BoundNodeKey, KeyAlgorithm, NodePublicKey, Revision, Version};
use meshway_policy::{
    SubjectContext,
    compile::{CompilationInput, compile},
};
use meshway_transport_spec::{Listener, NativeUdpListener, PreparedListener};
use std::time::Duration;

pub(super) fn node(i: u8) -> NodeId {
    format!("00000000-0000-4000-8000-{i:012}").parse().unwrap()
}
pub(super) fn request() -> RequestId {
    "00000000-0000-4001-8000-000000000001".parse().unwrap()
}

pub(super) struct Scene {
    pub configs: BTreeMap<NodeId, NodeConfiguration>,
    pub tables: BTreeMap<NodeId, FlowTable>,
    pub instant: Instant,
    pub now_ms: u64,
    pub guard_expires_at: Instant,
    pub guard_expires_at_ms: u64,
}

impl Scene {
    pub fn new(protocol: BusinessProtocol, external: bool, mandatory_relay: bool) -> Self {
        let mut config = new_network(
            AddressPools {
                ipv4: vec!["100.64.0.0/24".into()],
                ipv6: vec!["fd00::/64".into()],
            },
            "test.internal".into(),
        )
        .unwrap();
        config.tags_owners.insert("tag:node".into(), vec![]);
        let mut inventory = BTreeMap::new();
        for i in 1..=3 {
            let listener = Listener::NativeUdp(NativeUdpListener {
                enabled: true,
                remark: String::new(),
                listen: "0.0.0.0".parse().unwrap(),
                listen_port: Port::new(12345).unwrap(),
                endpoints: vec![],
                bind_interface: String::new(),
                socket: Default::default(),
            });
            config.nodes.insert(
                node(i),
                NodeSettings {
                    hostname: format!("n{i}"),
                    remark: String::new(),
                    identity: NodeIdentity::Tag {
                        tags: vec!["tag:node".into()],
                    },
                    enabled: true,
                    addresses: NodeAddresses {
                        ipv4: Some(format!("100.64.0.{i}").parse().unwrap()),
                        ipv6: Some(format!("fd00::{i}").parse().unwrap()),
                    },
                    listeners: BTreeMap::from([(
                        "udp".into(),
                        PreparedListener::prepare(listener).unwrap(),
                    )]),
                    dialers: Input::Omitted,
                    exit_enabled: external && i == 3,
                },
            );
            let key = |offset, algorithm| BoundNodeKey {
                id: format!("00000000-0000-4002-8000-{:012}", i + offset)
                    .parse()
                    .unwrap(),
                generation: Version::MIN,
                public_key: NodePublicKey {
                    algorithm,
                    bytes: [i + offset; 32].into(),
                },
            };
            inventory.insert(
                node(i),
                TrustedNode {
                    control_key: key(0, KeyAlgorithm::Ed25519),
                    dataplane_key: key(10, KeyAlgorithm::X25519),
                    capabilities: BTreeMap::from([(
                        "transport:native_udp".into(),
                        Capability::Transport {
                            roles: vec![TransportRole::Dial, TransportRole::Listen],
                            streams: vec![StreamKind::Udp],
                        },
                    )]),
                },
            );
        }
        config.access = vec![AccessRule {
            remark: String::new(),
            src: vec![format!("node:{}", node(1))],
            dst: if external {
                vec!["1.1.1.1/32".into()]
            } else {
                vec![format!("node:{}", node(3))]
            },
            ip: vec![
                match protocol {
                    BusinessProtocol::Tcp => "tcp:443",
                    BusinessProtocol::Udp => "udp:443",
                    BusinessProtocol::IcmpEcho => "icmp:*",
                }
                .into(),
            ],
            bidirectional: false,
        }];
        let via = if mandatory_relay {
            vec![format!("node:{}", node(2))]
        } else {
            vec![]
        };
        if external {
            config.routing.egress.rules = vec![EgressRule {
                remark: String::new(),
                matcher: RouteMatch {
                    ip_cidr: vec!["1.1.1.1/32".into()],
                    ..Default::default()
                },
                action: RouteAction::Exit {
                    nodes: vec![format!("node:{}", node(3))],
                    via,
                },
            }];
        } else {
            config.routing.mesh = vec![MeshRoute {
                remark: String::new(),
                src: vec![format!("node:{}", node(1))],
                dst: vec![format!("node:{}", node(3))],
                via,
            }];
        }
        let demand = Demand {
            id: request(),
            source: node(1),
            target: if external {
                BusinessTarget::Ip {
                    address: ExecutionAddress("1.1.1.1".parse().unwrap()),
                }
            } else {
                BusinessTarget::Mesh { node: node(3) }
            },
            protocol,
            port: if protocol == BusinessProtocol::IcmpEcho {
                None
            } else {
                Some(Port::new(443).unwrap())
            },
            profile: "mesh".into(),
            expires_at_ms: 100_000,
        };
        let publication = compile(CompilationInput {
            network: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
            revision: Revision::MIN,
            publication: "00000000-0000-4004-8000-000000000001".parse().unwrap(),
            configuration: &config,
            subjects: &SubjectContext::default(),
            inventory: &inventory,
            demands: &BTreeMap::from([(request(), demand)]),
            dns: &[],
            rule_sets: &BTreeMap::new(),
            selected_profiles: &BTreeMap::new(),
            now_ms: 1000,
        })
        .unwrap();
        let instant = Instant::now();
        Self {
            configs: publication
                .nodes
                .into_iter()
                .map(|(id, node)| (id, node.configuration))
                .collect(),
            tables: (1..=3)
                .map(|i| (node(i), FlowTable::new(Limits::default())))
                .collect(),
            instant,
            now_ms: 1000,
            guard_expires_at: instant + Duration::from_secs(300),
            guard_expires_at_ms: 301_000,
        }
    }

    pub fn advance(&mut self, duration: Duration) {
        self.instant += duration;
        self.now_ms += duration.as_millis() as u64;
    }
    pub fn process(
        &mut self,
        local: u8,
        previous: Option<u8>,
        sender: u8,
        path: &[u8],
        packet: Packet,
    ) -> Result<Action, Error> {
        let path = path.iter().map(|i| node(*i)).collect();
        let ingress = match previous {
            Some(previous) => Ingress::Peer {
                demand: request(),
                path,
                identity: AuthenticatedPeer {
                    origin: node(1),
                    sender: node(sender),
                    previous: node(previous),
                },
            },
            None => Ingress::Local {
                demand: request(),
                path,
            },
        };
        self.admit(local, ingress, packet)
    }
    pub fn admit(&mut self, local: u8, ingress: Ingress, packet: Packet) -> Result<Action, Error> {
        let checked = self.admit_facts(local, ingress, packet.facts())?;
        let route = checked.route;
        Ok(match checked.hop {
            Hop::Forward(next) => Action::Forward {
                next,
                packet,
                route,
            },
            Hop::Deliver => Action::Deliver { packet, route },
            Hop::Exit => Action::Exit { packet, route },
        })
    }

    pub fn admit_facts(
        &mut self,
        local: u8,
        ingress: Ingress,
        facts: PacketFacts,
    ) -> Result<CommittedRoute, Error> {
        let table = self.tables.get_mut(&node(local)).unwrap();
        let prepared = table.prepare(
            &self.configs[&node(local)],
            &ingress,
            &facts,
            FlowTime {
                instant: self.instant,
                now_ms: self.now_ms,
                guard_expires_at: self.guard_expires_at,
                guard_expires_at_ms: self.guard_expires_at_ms,
            },
        )?;
        Ok(table.commit(prepared))
    }
}
