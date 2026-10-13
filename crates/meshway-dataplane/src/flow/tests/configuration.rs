use super::super::*;
use crate::{Action, AuthenticatedPeer, Ingress, Limits, Packet};
use meshway_configuration::execution::*;
use meshway_configuration::netmap::CompiledNetmap;
use meshway_configuration::*;
use meshway_model::state::{Capability, StreamKind, TransportRole};
use meshway_model::{BoundNodeKey, KeyAlgorithm, NodePublicKey, Revision, Version};
use meshway_policy::{
    SubjectContext,
    compile::netmap::{NetmapInput, Target, compile_netmap},
};
use meshway_transport::spec::{Listener, PreparedListener};
use std::time::Duration;

pub(super) fn node(i: u8) -> NodeId {
    format!("00000000-0000-4000-8000-{i:012}").parse().unwrap()
}

/// Each Node's compiled netmap and lease, its flow table and a scripted
/// clock. Decisions go through the same per-netmap cache SecureNode uses.
pub(super) struct Scene {
    pub configs: BTreeMap<NodeId, CompiledNetmap>,
    pub decisions: BTreeMap<NodeId, crate::netmap::Decisions>,
    pub tables: BTreeMap<NodeId, FlowTable>,
    pub target: Target,
    pub instant: Instant,
    pub now_ms: u64,
    /// Every fresh decision is refused, as for an address whose trusted
    /// association ended under the same content.
    pub refuse_fresh: bool,
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
            // Published endpoints give peers dial candidates: netmap paths
            // only use adjacencies a carrier can actually be acquired on.
            let listener: Listener = serde_json::from_value(serde_json::json!({
                "type": "native_udp", "listenPort": 12345,
                "endpoints": [{"host": format!("192.0.2.{i}"), "port": 12345}],
            }))
            .unwrap();
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
                    capabilities: {
                        let mut capabilities = BTreeMap::from([(
                            "transport:native_udp".into(),
                            Capability::Transport {
                                roles: vec![TransportRole::Dial, TransportRole::Listen],
                                streams: vec![StreamKind::Udp],
                            },
                        )]);
                        if external && i == 3 {
                            capabilities.insert(
                                meshway_model::exit::EXIT_IPV4_FEATURE.into(),
                                Capability::Feature {
                                    version: meshway_model::exit::EXIT_IPV4_VERSION,
                                },
                            );
                        }
                        capabilities
                    },
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
                action: RuleAction::Exit {
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
        let compiled = compile_netmap(NetmapInput {
            network: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
            revision: Revision::MIN,
            configuration: &config,
            subjects: &SubjectContext::default(),
            inventory: &inventory,
            rule_sets: &BTreeMap::new(),
            selected_profiles: &BTreeMap::new(),
            selection_versions: &BTreeMap::new(),
            previous: &BTreeMap::new(),
        })
        .unwrap();
        let instant = Instant::now();
        let now_ms = 1000;
        Self {
            configs: compiled.nodes,
            decisions: (1..=3).map(|i| (node(i), Default::default())).collect(),
            tables: (1..=3)
                .map(|i| (node(i), FlowTable::new(Limits::default())))
                .collect(),
            target: if external {
                Target::Exit {
                    nodes: vec![node(3)],
                    via: if mandatory_relay {
                        vec![node(2)]
                    } else {
                        vec![]
                    },
                }
            } else {
                Target::Mesh { node: node(3) }
            },
            instant,
            now_ms,
            refuse_fresh: false,
        }
    }

    pub fn time(&self) -> FlowTime {
        FlowTime {
            instant: self.instant,
            now_ms: self.now_ms,
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
                target: self.target.clone(),
                path,
                identity: AuthenticatedPeer {
                    origin: node(1),
                    sender: node(sender),
                    previous: node(previous),
                },
            },
            None => Ingress::Local {
                target: self.target.clone(),
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
        let time = self.time();
        let id = node(local);
        let entry = ingress.entry(id)?;
        let origin = entry.origin;
        let netmap = &self.configs[&id];
        let decisions = self.decisions.get_mut(&id).unwrap();
        let compiler = decisions.compiler_for(netmap);
        let refuse_fresh = self.refuse_fresh;
        let mut decide = |traffic: &meshway_policy::admission::Traffic| match (!refuse_fresh)
            .then_some(())
            .ok_or(Error::Flow)
            .and_then(|()| {
                decisions.decide(
                    netmap,
                    origin,
                    traffic.destination,
                    (traffic.protocol, traffic.destination_port),
                )
            })? {
            crate::netmap::Outcome::Permitted(decision) => Ok(decision),
            crate::netmap::Outcome::Denied(_) => Err(Error::Admission(
                meshway_policy::admission::AdmissionError::Traffic,
            )),
            _ => Err(Error::Flow),
        };
        let table = self.tables.get_mut(&id).unwrap();
        if let crate::Transport::Error(error) = facts.transport {
            // The Engine's related admission: the flow table lookup with the
            // Policy check over the flow's own decision.
            return table
                .related(
                    id,
                    &entry,
                    &error.quoted,
                    None,
                    time,
                    |decision, forward| {
                        meshway_policy::admission::netmap::admit_related(
                            &netmap.netmap,
                            decision,
                            entry.path,
                            entry.origin,
                            entry.previous,
                            forward,
                            &error.quoted.traffic,
                            facts.traffic.source,
                            facts.traffic.destination,
                        )
                        .map_err(Error::from)
                    },
                )
                .map(|related| related.checked);
        }
        let prepared = table.prepare(
            Policy {
                netmap: &netmap.netmap,
                compiler: &compiler,
                decide: &mut decide,
            },
            id,
            &entry,
            &facts,
            time,
        )?;
        Ok(table.commit(prepared))
    }
}
