//! Real enrolled Controls installing real compiled netmaps through the sync
//! contract, and real SecureNodes over native TCP: A→R→B, with B also an
//! IPv4 exit reached via R. No demand, publication or permit exists here.
use super::{Node, Owner, Server, secure::Link};
use meshway_configuration::{
    AccessRule, AddressPools, Input, MeshRoute, NetworkConfiguration, NodeAddresses, NodeIdentity,
    NodeSettings, ProfileAssignment, RouteAction, RoutingProfile, SubjectContext,
    execution::TrustedNode, netmap::CompiledNetmap,
};
use meshway_control::{FactsRejected, PeerFactsSink, SyncFuture, SyncTransport};
use meshway_control_client::ClientError;
use meshway_dataplane::{
    Packet,
    secure::{Carrier, CircuitGrant, PendingFrame, Received, SecureNode, Target, Update},
};
use meshway_model::{
    NetmapRef, NodeId, Revision,
    state::{Capability, StreamKind, TransportRole},
};
use meshway_policy::compile::netmap::{NetmapInput, PreviousNetmap, compile_netmap};
use meshway_protocol::{ErrorResponse, PeerFactsDelta, SyncRequest, SyncResponse};
use meshway_transport_spec::{Listener, PreparedListener, TransportProtocol};
use std::{
    collections::{BTreeMap, VecDeque},
    net::IpAddr,
    sync::{Arc, Mutex},
};
use tokio::net::TcpListener;

pub const DAY_MS: u64 = 86_400_000;
/// Public IPv4 destination behind exit B.
pub const INTERNET: &str = "93.184.216.34";

type Reply = Result<SyncResponse, ClientError<ErrorResponse>>;

/// Scripted sync replies; an empty script behaves like an unreachable Server.
#[derive(Default)]
pub struct Script {
    replies: Mutex<VecDeque<Reply>>,
    pub calls: Mutex<usize>,
}
impl SyncTransport for Script {
    fn sync<'a>(&'a self, _: &'a SyncRequest, token: &'a str) -> SyncFuture<'a> {
        assert!(!token.is_empty());
        *self.calls.lock().unwrap() += 1;
        let reply = self.replies.lock().unwrap().pop_front();
        Box::pin(async move { reply.unwrap_or(Err(ClientError::Network { status: None })) })
    }
}

#[derive(Default)]
struct Facts;
impl PeerFactsSink for Facts {
    fn apply(&mut self, _: NetmapRef, _: PeerFactsDelta) -> Result<(), FactsRejected> {
        Ok(())
    }
}

fn listener(port: u16) -> PreparedListener {
    let declaration: Listener = serde_json::from_value(serde_json::json!({
        "type": "native_tcp", "listen": "127.0.0.1", "listenPort": port,
        "endpoints": [{"host": "127.0.0.1", "port": port}],
    }))
    .unwrap();
    PreparedListener::prepare(declaration).unwrap()
}

pub fn sel(node: NodeId) -> String {
    format!("node:{node}")
}

pub fn rule(src: NodeId, dst: &str, ip: &[&str]) -> AccessRule {
    AccessRule {
        remark: String::new(),
        src: vec![sel(src)],
        dst: vec![dst.into()],
        ip: ip.iter().map(|value| (*value).into()).collect(),
        bidirectional: false,
    }
}

pub struct NetmapMesh {
    pub server: Server,
    pub owner: Owner,
    /// Controls of A, R and B.
    pub nodes: [Node; 3],
    pub cfg: NetworkConfiguration,
    pub inventory: BTreeMap<NodeId, TrustedNode>,
    pub compiled: BTreeMap<NodeId, CompiledNetmap>,
    scripts: [Arc<Script>; 3],
    pub validity_ms: u64,
    pub a: SecureNode,
    pub r: SecureNode,
    pub b: SecureNode,
    pub ar: Link,
    pub rb: Link,
}

impl NetmapMesh {
    pub async fn start() -> Self {
        Self::with(DAY_MS, |_, _| {}).await
    }

    /// `edit` adjusts the base configuration: A may reach B on udp:42424,
    /// tcp:443 and icmp, and the Internet on udp via exit B; every A→B and
    /// exit path goes through R.
    pub async fn with(
        validity_ms: u64,
        edit: impl FnOnce(&mut NetworkConfiguration, [NodeId; 3]),
    ) -> Self {
        let server = Server::start().await;
        let owner = server.owner().await;
        let caps = || {
            BTreeMap::from([
                (
                    "feature:exit-ipv4".into(),
                    Capability::Feature { version: 1 },
                ),
                (
                    "feature:path-observation".into(),
                    Capability::Feature { version: 1 },
                ),
                (
                    "transport:native_tcp".into(),
                    Capability::Transport {
                        roles: vec![TransportRole::Dial, TransportRole::Listen],
                        streams: vec![StreamKind::Tcp],
                    },
                ),
            ])
        };
        let a = server.enroll_with_capabilities(&owner, "a", caps()).await;
        let r = server.enroll_with_capabilities(&owner, "r", caps()).await;
        let b = server.enroll_with_capabilities(&owner, "b", caps()).await;
        let ids = [a.id, r.id, b.id];
        let r_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let b_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = meshway_configuration::new_network(
            AddressPools {
                ipv4: vec!["100.64.0.0/24".into()],
                ipv6: vec![],
            },
            "test.internal".into(),
        )
        .unwrap();
        cfg.tags_owners.insert("tag:server".into(), vec![]);
        for (index, node) in [&a, &r, &b].into_iter().enumerate() {
            let listeners = match index {
                1 => BTreeMap::from([(
                    "tcp".into(),
                    listener(r_listener.local_addr().unwrap().port()),
                )]),
                2 => BTreeMap::from([(
                    "tcp".into(),
                    listener(b_listener.local_addr().unwrap().port()),
                )]),
                _ => BTreeMap::new(),
            };
            cfg.nodes.insert(
                node.id,
                NodeSettings {
                    hostname: ["a", "r", "b"][index].into(),
                    remark: String::new(),
                    identity: NodeIdentity::Tag {
                        tags: vec!["tag:server".into()],
                    },
                    enabled: true,
                    addresses: NodeAddresses {
                        ipv4: Some(format!("100.64.0.{}", index + 1).parse().unwrap()),
                        ipv6: None,
                    },
                    listeners,
                    dialers: Input::Omitted,
                    exit_enabled: index == 2,
                },
            );
        }
        cfg.access = vec![
            rule(a.id, &sel(b.id), &["udp:42424", "tcp:443", "icmp:*"]),
            rule(a.id, "autogroup:internet", &["udp:*"]),
        ];
        cfg.routing.mesh = vec![MeshRoute {
            remark: String::new(),
            src: vec![sel(a.id)],
            dst: vec![sel(b.id)],
            via: vec![sel(r.id)],
        }];
        cfg.routing.egress.profiles.insert(
            "exit".into(),
            RoutingProfile {
                rules: vec![],
                fallback: RouteAction::Exit {
                    nodes: vec![sel(b.id)],
                    via: vec![sel(r.id)],
                },
            },
        );
        cfg.routing.egress.assignments.push(ProfileAssignment {
            target: vec![sel(a.id)],
            allowed: vec!["exit".into(), "mesh".into()],
            default: "exit".into(),
            locked: false,
        });
        edit(&mut cfg, ids);
        let inventory = [&a, &r, &b]
            .into_iter()
            .map(|node| {
                (
                    node.id,
                    TrustedNode {
                        control_key: node.keys.control.clone(),
                        dataplane_key: node.keys.dataplane.clone(),
                        capabilities: caps(),
                    },
                )
            })
            .collect();
        let scripts: [Arc<Script>; 3] = Default::default();
        let mut nodes = [a, r, b];
        for (node, script) in nodes.iter_mut().zip(&scripts) {
            node.control.set_sync_transport(script.clone());
        }
        let ar_carrier = Carrier {
            dialer: ids[0],
            listener: ids[1],
            listener_key: "tcp".into(),
            protocol: TransportProtocol::NativeTcp,
        };
        let rb_carrier = Carrier {
            dialer: ids[1],
            listener: ids[2],
            listener_key: "tcp".into(),
            protocol: TransportProtocol::NativeTcp,
        };
        let compiled = compile(owner.network, &cfg, &inventory, ids, &BTreeMap::new());
        let mut identities = Vec::new();
        for index in 0..3 {
            let snapshot = install(
                &mut nodes[index],
                &scripts[index],
                &compiled[&ids[index]],
                validity_ms,
                true,
            )
            .await;
            identities.push(nodes[index].control.dataplane_identity(snapshot).unwrap());
        }
        let mut identities = identities.into_iter();
        let mut secure = |carriers| {
            SecureNode::new(identities.next().unwrap(), Default::default(), carriers).unwrap()
        };
        let a = secure(BTreeMap::from([(ids[1], ar_carrier.clone())]));
        let r = secure(BTreeMap::from([
            (ids[0], ar_carrier.clone()),
            (ids[2], rb_carrier.clone()),
        ]));
        let b = secure(BTreeMap::from([(ids[1], rb_carrier.clone())]));
        Self {
            server,
            owner,
            nodes,
            cfg,
            inventory,
            compiled,
            scripts,
            validity_ms,
            a,
            r,
            b,
            ar: Link::connect(r_listener, ar_carrier).await,
            rb: Link::connect(b_listener, rb_carrier).await,
        }
    }

    /// The current Control-issued identity of one Node.
    pub fn identity(&self, index: usize) -> meshway_authority::DataPlaneIdentity {
        let control = &self.nodes[index].control;
        control
            .dataplane_identity(control.netmap_snapshot().unwrap())
            .unwrap()
    }

    /// Replace every SecureNode with explicit limits on the same identities.
    pub fn rebuild(
        &mut self,
        limits: meshway_dataplane::secure::Limits,
        exit: meshway_dataplane::exit::Limits,
    ) {
        let [a, r, b] = self.ids();
        let node = |identity, carriers| {
            SecureNode::with_exit_limits(identity, limits, carriers, exit).unwrap()
        };
        self.a = node(
            self.identity(0),
            BTreeMap::from([(r, self.ar.carrier.clone())]),
        );
        self.r = node(
            self.identity(1),
            BTreeMap::from([(a, self.ar.carrier.clone()), (b, self.rb.carrier.clone())]),
        );
        self.b = node(
            self.identity(2),
            BTreeMap::from([(r, self.rb.carrier.clone())]),
        );
    }

    pub fn ids(&self) -> [NodeId; 3] {
        [self.nodes[0].id, self.nodes[1].id, self.nodes[2].id]
    }

    pub fn path(&self) -> Vec<NodeId> {
        self.ids().to_vec()
    }

    pub fn mesh_target(&self) -> Target {
        Target::Mesh {
            node: self.nodes[2].id,
        }
    }

    pub fn exit_target(&self) -> Target {
        Target::Exit {
            nodes: vec![self.nodes[2].id],
            via: vec![self.nodes[1].id],
        }
    }

    pub fn address(&self, index: usize) -> IpAddr {
        format!("100.64.0.{}", index + 1).parse().unwrap()
    }

    /// Sync calls each Control issued; first packets must not add any.
    pub fn sync_calls(&self) -> usize {
        self.scripts
            .iter()
            .map(|script| *script.calls.lock().unwrap())
            .sum()
    }

    fn secure(&mut self, index: usize) -> &mut SecureNode {
        match index {
            0 => &mut self.a,
            1 => &mut self.r,
            _ => &mut self.b,
        }
    }

    /// Recompile after `edit` and install the new content on every Node;
    /// returns each SecureNode's update result.
    pub async fn publish(
        &mut self,
        edit: impl FnOnce(&mut NetworkConfiguration, [NodeId; 3]),
    ) -> [Update; 3] {
        self.recompile(edit);
        self.update_all(true).await
    }

    /// Same-content lease renewal on every Node.
    pub async fn renew(&mut self) -> [Update; 3] {
        self.update_all(false).await
    }

    async fn update_all(&mut self, full: bool) -> [Update; 3] {
        let mut updates = Vec::new();
        for index in 0..3 {
            updates.push(self.install_on(index, full).await);
        }
        updates.try_into().unwrap()
    }

    /// Install the current compilation (`full`) or renew the held content on
    /// one Node only, and update its SecureNode.
    pub async fn install_on(&mut self, index: usize, full: bool) -> Update {
        let id = self.nodes[index].id;
        let snapshot = install(
            &mut self.nodes[index],
            &self.scripts[index],
            &self.compiled[&id],
            self.validity_ms,
            full,
        )
        .await;
        let identity = self.nodes[index]
            .control
            .dataplane_identity(snapshot)
            .unwrap();
        self.secure(index).update(identity).unwrap()
    }

    /// Recompile without installing.
    pub fn recompile(&mut self, edit: impl FnOnce(&mut NetworkConfiguration, [NodeId; 3])) {
        let ids = self.ids();
        edit(&mut self.cfg, ids);
        self.compiled = compile(
            self.owner.network,
            &self.cfg,
            &self.inventory,
            ids,
            &self.compiled,
        );
    }

    /// Open the business circuit A→R→B toward `target`.
    pub async fn handshake(&mut self, target: Target) -> [u8; 32] {
        let first = self
            .a
            .start_grant(
                CircuitGrant::Netmap {
                    purpose: meshway_dataplane::Purpose::Business,
                    target,
                },
                self.path(),
                &self.ar.carrier.clone(),
            )
            .unwrap();
        self.finish_handshake(first).await
    }

    pub async fn finish_handshake(&mut self, first: PendingFrame) -> [u8; 32] {
        let circuit = first.circuit();
        self.ar.dial.send(first).await;
        let first = self.ar.listen.recv().await;
        let Received::Forward(first) = self.r.receive(&self.ar.carrier, first).unwrap() else {
            panic!("relay must stay opaque")
        };
        self.rb.dial.send(first).await;
        let first = self.rb.listen.recv().await;
        let Received::Established {
            circuit: terminal,
            response: Some(second),
        } = self.b.receive(&self.rb.carrier, first).unwrap()
        else {
            panic!("terminal handshake")
        };
        assert_eq!(terminal, circuit);
        self.rb.listen.send(second).await;
        let second = self.rb.dial.recv().await;
        let Received::Forward(second) = self.r.receive(&self.rb.carrier, second).unwrap() else {
            panic!("relay must stay opaque")
        };
        self.ar.listen.send(second).await;
        let second = self.ar.dial.recv().await;
        assert!(matches!(
            self.a.receive(&self.ar.carrier, second).unwrap(),
            Received::Established { circuit: source, response: None } if source == circuit
        ));
        circuit
    }

    /// A UDP datagram between A and B on port 42424 (`reply` reverses it).
    pub fn mesh_udp(&self, reply: bool, port: u16) -> Packet {
        let (a, b) = (self.address(0), self.address(2));
        if reply {
            super::packet::udp(b, a, port, 50000, b"netmap mesh reply")
        } else {
            super::packet::udp(a, b, 50000, port, b"netmap mesh forward")
        }
    }

    /// A UDP datagram between A and the Internet destination behind exit B.
    pub fn exit_udp(&self, reply: bool, port: u16) -> Packet {
        let (a, e) = (self.address(0), INTERNET.parse().unwrap());
        if reply {
            super::packet::udp(e, a, 42424, port, b"exit reply")
        } else {
            super::packet::udp(a, e, port, 42424, b"exit forward")
        }
    }

    pub async fn close(mut self) {
        self.ar.dial.close().await;
        self.ar.listen.close().await;
        self.rb.dial.close().await;
        self.rb.listen.close().await;
        for node in self.nodes {
            node.control.shutdown().await.unwrap();
        }
        self.server.close().await;
    }
}

fn compile(
    network: meshway_model::NetworkId,
    cfg: &NetworkConfiguration,
    inventory: &BTreeMap<NodeId, TrustedNode>,
    ids: [NodeId; 3],
    previous: &BTreeMap<NodeId, CompiledNetmap>,
) -> BTreeMap<NodeId, CompiledNetmap> {
    let previous = previous
        .iter()
        .map(|(id, netmap)| (*id, PreviousNetmap::from(netmap)))
        .collect();
    let compiled = compile_netmap(NetmapInput {
        network,
        revision: Revision::MIN,
        configuration: cfg,
        subjects: &SubjectContext::default(),
        inventory,
        rule_sets: &BTreeMap::new(),
        selected_profiles: &BTreeMap::new(),
        selection_versions: &BTreeMap::new(),
        previous: &previous,
    })
    .unwrap();
    ids.iter()
        .map(|id| (*id, compiled.nodes[id].clone()))
        .collect()
}

/// Install `compiled` (or renew the held content) through one sync reply.
async fn install(
    node: &mut Node,
    script: &Script,
    compiled: &CompiledNetmap,
    validity_ms: u64,
    full: bool,
) -> meshway_authority::ExecutionSnapshot {
    let full = full && node.control.netmap_reference() != Some(compiled.reference());
    script
        .replies
        .lock()
        .unwrap()
        .push_back(Ok(meshway_netmap_fixture::reply(
            compiled,
            validity_ms,
            full,
        )));
    node.control.sync(&mut Facts).await.unwrap().snapshot
}
