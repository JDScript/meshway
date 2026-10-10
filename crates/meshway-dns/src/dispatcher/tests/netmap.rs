//! Netmap scopes. The production Control issues every snapshot from a real
//! enrollment; the sync replies carry netmaps compiled by the shared Policy
//! compiler for that enrolled identity. Only resolver I/O is scripted.
use super::*;
use meshway_configuration::{
    AccessRule, AddressPools, DnsServer, EgressRule, Input, NodeAddresses, NodeIdentity,
    NodeSettings, ProfileAssignment, RouteAction, RouteMatch, RoutingProfile, RuleAction,
    SubjectContext,
    execution::TrustedNode,
    netmap::{CompiledNetmap, DnsPlan, ResolverAction},
};
use meshway_control::{FactsRejected, PeerFactsSink, SyncFuture, SyncTransport};
use meshway_control_client::ClientError;
use meshway_model::{
    BoundNodeKey, KeyAlgorithm, NetmapRef, NodeId, NodePublicKey, Revision, Version,
    state::{Capability, StreamKind, TransportRole},
};
use meshway_policy::compile::netmap::{NetmapInput, PreviousNetmap, compile_netmap};
use meshway_protocol::{PeerFactsDelta, SyncRequest, SyncResponse};

const HOUR: u64 = 60 * 60 * 1000;

#[tokio::test]
async fn netmap_query_capacity_cancellation_and_unsent_answer_deadlines_are_independent() {
    let server = support::Server::start().await;
    let (mut control, script) = enrolled(&server).await;
    let network = Network::new(&control, 'a');
    let netmap = network.compile(None);
    let snapshot = sync(&mut control, &script, reply(&netmap, true)).await;
    let mut owner = Owner::new(snapshot.clone(), limits()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut pending = Vec::new();
    for _ in 0..4 {
        let Begin::Upstream(work) =
            owner.begin_published_until(named("x.direct.example."), deadline)
        else {
            panic!("exact local resolver route")
        };
        pending.push(work);
    }
    assert!(matches!(
        owner.begin_published_until(named("x.direct.example."), deadline),
        Begin::Unavailable(Unavailable::Capacity)
    ));
    assert!(owner.cancel(&pending[0].ticket));
    assert!(!owner.cancel(&pending[0].ticket));
    assert!(pending[0].route().ensure_current().is_err());
    pending[1].route().ensure_current().unwrap();
    let Begin::Upstream(replacement) =
        owner.begin_published_until(named("x.direct.example."), deadline)
    else {
        panic!("cancel returns its slot")
    };
    let mut other = Owner::new(snapshot, limits()).unwrap();
    assert!(!other.cancel(&replacement.ticket));
    replacement.route().ensure_current().unwrap();
    let short = Instant::now() + Duration::from_millis(20);
    let Begin::Answer(answer) = owner.begin_published_until(named("b.test.internal."), short)
    else {
        panic!("MagicDNS is answered locally even when resolver slots are full")
    };
    tokio::time::sleep_until(short.into()).await;
    assert!(answer.ensure_current().is_err());
    owner.stop();
    for work in pending.into_iter().skip(1).chain([replacement]) {
        assert!(work.route().ensure_current().is_err());
    }
    assert!(matches!(
        owner.begin_published_until(named("b.test.internal."), deadline),
        Begin::Unavailable(Unavailable::Inactive)
    ));
    control.shutdown().await.unwrap();
    server.close().await;
}

#[derive(Default)]
struct Script(StdMutex<VecDeque<SyncResponse>>);
impl SyncTransport for Script {
    fn sync<'a>(&'a self, _: &'a SyncRequest, _: &'a str) -> SyncFuture<'a> {
        let reply = self.0.lock().unwrap().pop_front();
        Box::pin(async move { reply.ok_or(ClientError::Network { status: None }) })
    }
}
struct Facts;
impl PeerFactsSink for Facts {
    fn apply(&mut self, _: NetmapRef, _: PeerFactsDelta) -> Result<(), FactsRejected> {
        Ok(())
    }
}

fn node(i: u8) -> NodeId {
    format!("00000000-0000-4000-8000-{i:012}").parse().unwrap()
}
fn sel(id: NodeId) -> String {
    format!("node:{id}")
}

fn trusted(i: u8, exit: bool) -> TrustedNode {
    let key = |offset: u8, algorithm| BoundNodeKey {
        id: format!(
            "00000000-0000-4002-8000-{:012}",
            u16::from(i) + u16::from(offset)
        )
        .parse()
        .unwrap(),
        generation: Version::MIN,
        public_key: NodePublicKey {
            algorithm,
            bytes: [i.wrapping_add(offset); 32].into(),
        },
    };
    let mut capabilities = BTreeMap::from([(
        "transport:native_udp".into(),
        Capability::Transport {
            roles: vec![TransportRole::Dial, TransportRole::Listen],
            streams: vec![StreamKind::Udp],
        },
    )]);
    if exit {
        capabilities.insert(
            "feature:exit-ipv4".into(),
            Capability::Feature { version: 1 },
        );
    }
    TrustedNode {
        control_key: key(0, KeyAlgorithm::Ed25519),
        dataplane_key: key(100, KeyAlgorithm::X25519),
        capabilities,
    }
}

/// Source A (100.64.0.1) may reach B; its ordinary DNS and 203.0.113/24 leave
/// through exit E (100.64.0.3), 192.0.2/24 is direct. `enrolled` is the Node
/// whose keys come from the real enrollment.
struct Network {
    network: meshway_model::NetworkId,
    enrolled: NodeId,
    keys: TrustedNode,
    a: NodeId,
    b: NodeId,
    e: NodeId,
    hostname: String,
}

impl Network {
    fn new(control: &Control, role: char) -> Self {
        let binding = control.status().binding.unwrap();
        let mut keys = trusted(9, role == 'e');
        keys.control_key = binding.keys.control.clone();
        keys.dataplane_key = binding.keys.dataplane.clone();
        let pick = |name, i| {
            if role == name {
                binding.node_id
            } else {
                node(i)
            }
        };
        Self {
            network: binding.network_id,
            enrolled: binding.node_id,
            keys,
            a: pick('a', 1),
            b: pick('b', 2),
            e: pick('e', 3),
            hostname: "a".into(),
        }
    }

    fn compile(&self, previous: Option<&CompiledNetmap>) -> CompiledNetmap {
        let mut cfg = meshway_configuration::new_network(
            AddressPools {
                ipv4: vec!["100.64.0.0/24".into()],
                ipv6: vec![],
            },
            "test.internal".into(),
        )
        .unwrap();
        cfg.access = vec![
            AccessRule {
                remark: String::new(),
                src: vec![sel(self.a)],
                dst: vec![sel(self.b)],
                ip: vec!["*".into()],
                bidirectional: false,
            },
            AccessRule {
                remark: String::new(),
                src: vec![sel(self.a)],
                dst: vec!["autogroup:internet".into()],
                ip: vec!["*".into()],
                bidirectional: false,
            },
        ];
        cfg.tags_owners.insert("tag:server".into(), vec![]);
        let listener: meshway_configuration::transport::Listener =
            serde_json::from_str(r#"{"type":"native_udp","listenPort":12345}"#).unwrap();
        let listener =
            meshway_configuration::transport::PreparedListener::prepare(listener).unwrap();
        for (id, i, hostname) in [
            (self.a, 1, self.hostname.as_str()),
            (self.b, 2, "b"),
            (self.e, 3, "e"),
        ] {
            cfg.nodes.insert(
                id,
                NodeSettings {
                    hostname: hostname.into(),
                    remark: String::new(),
                    identity: NodeIdentity::Tag {
                        tags: vec!["tag:server".into()],
                    },
                    enabled: true,
                    addresses: NodeAddresses {
                        ipv4: Some(format!("100.64.0.{i}").parse().unwrap()),
                        ipv6: None,
                    },
                    listeners: BTreeMap::from([("udp".into(), listener.clone())]),
                    dialers: Input::Omitted,
                    exit_enabled: id == self.e,
                },
            );
        }
        cfg.routing.egress.profiles.insert(
            "exit".into(),
            RoutingProfile {
                rules: vec![EgressRule {
                    remark: String::new(),
                    matcher: RouteMatch {
                        ip_cidr: vec!["192.0.2.0/24".into()],
                        ..Default::default()
                    },
                    action: RuleAction::Direct,
                }],
                fallback: RouteAction::Exit {
                    nodes: vec![sel(self.e)],
                    via: vec![],
                },
            },
        );
        cfg.routing.egress.assignments.push(ProfileAssignment {
            target: vec![sel(self.a)],
            allowed: vec!["exit".into(), "mesh".into()],
            default: "exit".into(),
            locked: false,
        });
        let server = |address: &str| DnsServer {
            transport: DnsTransport::Udp,
            server: address.parse().unwrap(),
            server_port: 53.try_into().unwrap(),
            use_with_exit: true,
        };
        cfg.dns.servers = BTreeMap::from([
            ("direct".into(), server("192.0.2.53")),
            ("routed".into(), server("203.0.113.53")),
            ("mesh".into(), server("100.64.0.2")),
        ]);
        cfg.dns.routes = BTreeMap::from([
            ("direct.example".into(), vec!["direct".into()]),
            ("routed.example".into(), vec!["routed".into()]),
            ("mesh.example".into(), vec!["mesh".into()]),
        ]);
        let mut inventory = BTreeMap::from([
            (self.a, trusted(1, false)),
            (self.b, trusted(2, false)),
            (self.e, trusted(3, true)),
        ]);
        inventory.insert(self.enrolled, self.keys.clone());
        let previous = previous
            .map(|old| BTreeMap::from([(self.enrolled, PreviousNetmap::from(old))]))
            .unwrap_or_default();
        let mut compiled = compile_netmap(NetmapInput {
            network: self.network,
            revision: Revision::MIN,
            configuration: &cfg,
            subjects: &SubjectContext::default(),
            inventory: &inventory,
            rule_sets: &BTreeMap::new(),
            selected_profiles: &BTreeMap::new(),
            selection_versions: &BTreeMap::new(),
            previous: &previous,
        })
        .unwrap();
        compiled.nodes.remove(&self.enrolled).unwrap()
    }
}

fn reply(netmap: &CompiledNetmap, full: bool) -> SyncResponse {
    meshway_netmap_fixture::reply(netmap, HOUR, full)
}

async fn enrolled(server: &support::Server) -> (Control, Arc<Script>) {
    let mut control = server.control();
    server.enroll(&mut control).await;
    let script = Arc::new(Script::default());
    control.set_sync_transport(script.clone());
    (control, script)
}

async fn sync(control: &mut Control, script: &Script, reply: SyncResponse) -> ExecutionSnapshot {
    script.0.lock().unwrap().push_back(reply);
    control.sync(&mut Facts).await.unwrap().snapshot
}

fn named(name: &str) -> Query {
    let mut message = Message::new(77, MessageType::Query, OpCode::Query);
    message.add_query(Question::query(
        Name::from_ascii(name).unwrap(),
        RecordType::A,
    ));
    decode_query(&message.to_vec().unwrap()).unwrap()
}

fn limits() -> Limits {
    Limits {
        max_pending: 4,
        query_timeout: Duration::from_secs(2),
    }
}

#[tokio::test]
async fn netmap_resolvers_follow_their_decisions_and_magic_dns_uses_mesh_hosts() {
    let server = support::Server::start().await;
    let (mut control, script) = enrolled(&server).await;
    let network = Network::new(&control, 'a');
    let netmap = network.compile(None);
    let dns = &netmap.netmap.dns;
    assert_eq!(dns.resolvers["direct"], ResolverAction::Direct);
    assert_eq!(dns.resolvers["routed"], ResolverAction::Exit);
    assert_eq!(dns.resolvers["mesh"], ResolverAction::Rejected);
    assert!(matches!(dns.egress, DnsPlan::ViaExit { .. }));
    let snapshot = sync(&mut control, &script, reply(&netmap, true)).await;
    let mut owner = Owner::new(snapshot.clone(), limits()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);

    // A direct resolver gets its exact local route without any demand ID.
    let Begin::Upstream(work) = owner.begin_published_until(named("x.direct.example."), deadline)
    else {
        panic!("direct resolver must be routed locally")
    };
    let ResolverRoute::Local(route) = work.route() else {
        panic!("a configured resolver is local")
    };
    assert_eq!(route.endpoint(), "192.0.2.53:53".parse().unwrap());
    // Exit-routed and Mesh resolvers never get a local socket, nor a fallback.
    for name in ["x.routed.example.", "x.mesh.example."] {
        assert!(matches!(
            owner.begin_published_until(named(name), deadline),
            Begin::Unavailable(Unavailable::MissingResolverPlan)
        ));
    }
    // Ordinary names use only the exit's Mesh resolver service.
    let Begin::Upstream(work) = owner.begin_published_until(named("ordinary.test."), deadline)
    else {
        panic!("ordinary DNS goes through the exit service")
    };
    let ResolverRoute::Mesh(route) = work.route() else {
        panic!("the exit service is a Mesh route")
    };
    assert_eq!(route.endpoint(), "100.64.0.3:53".parse().unwrap());
    assert_eq!(route.service().exit, network.e);

    // MagicDNS answers from meshHosts; a name outside them is REFUSED locally.
    let answer = match owner.begin_published_until(named("b.test.internal."), deadline) {
        Begin::Answer(answer) => answer,
        other => panic!("{other:?}"),
    };
    let message = Message::from_vec(&answer.encode(ResponseTransport::Udp, 1232).unwrap()).unwrap();
    assert_eq!(message.answers.len(), 1);
    match owner.begin_published_until(named("e.test.internal."), deadline) {
        Begin::Answer(answer) => {
            let message =
                Message::from_vec(&answer.encode(ResponseTransport::Udp, 1232).unwrap()).unwrap();
            assert_eq!(
                message.metadata.response_code,
                hickory_proto::op::ResponseCode::Refused
            );
            assert!(message.answers.is_empty() && message.authorities.is_empty());
        }
        other => panic!("an invisible name must not need discovery: {other:?}"),
    }
    // Readiness fails closed: a declared resolver without a local route.
    assert_eq!(
        Scope::netmap(snapshot, None).validate_resolvers(),
        Err(DispatchError::Unavailable(Unavailable::MissingResolverPlan))
    );
    let _ = control.shutdown().await;
    server.close().await;
}

#[tokio::test]
async fn netmap_renewal_keeps_owner_and_fences_while_a_new_serial_ends_them() {
    let server = support::Server::start().await;
    let (mut control, script) = enrolled(&server).await;
    let mut network = Network::new(&control, 'a');
    let first = network.compile(None);
    let snapshot = sync(&mut control, &script, reply(&first, true)).await;
    let (sender, receiver) = watch::channel(Some(Scope::netmap(snapshot.clone(), None)));
    let dispatcher = OwnerDispatcher::new(receiver, Exchange::new(&[]), limits(), 1232).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let answer = dispatcher
        .dispatch(named("b.test.internal."), deadline)
        .await
        .unwrap();
    answer.ensure_current().unwrap();
    let generation = {
        let state = dispatcher.state.lock().await;
        state.owner.as_ref().unwrap().generation()
    };

    // Same content: the Control renews in place and the scope is equivalent.
    let renewed = sync(&mut control, &script, reply(&first, false)).await;
    assert!(renewed.same_netmap(&snapshot));
    sender.send_replace(Some(Scope::netmap(renewed, None)));
    answer.ensure_current().unwrap();
    dispatcher
        .dispatch(named("b.test.internal."), deadline)
        .await
        .unwrap();
    {
        let state = dispatcher.state.lock().await;
        assert!(Arc::ptr_eq(
            &state.owner.as_ref().unwrap().generation(),
            &generation
        ));
    }

    // A larger serial ends old answers and replaces the owner generation.
    network.hostname = "a2".into();
    let second = network.compile(Some(&first));
    assert!(second.netmap.serial > first.netmap.serial);
    let next = sync(&mut control, &script, reply(&second, true)).await;
    assert!(answer.ensure_current().is_err());
    sender.send_replace(Some(Scope::netmap(next, None)));
    assert!(answer.ensure_current().is_err());
    dispatcher
        .dispatch(named("b.test.internal."), deadline)
        .await
        .unwrap();
    let state = dispatcher.state.lock().await;
    assert!(!Arc::ptr_eq(
        &state.owner.as_ref().unwrap().generation(),
        &generation
    ));
    drop(state);
    let _ = control.shutdown().await;
    server.close().await;
}

#[tokio::test]
async fn served_exit_resolver_admits_only_planned_sources_and_never_falls_back() {
    let server = support::Server::start().await;
    let (mut control, script) = enrolled(&server).await;
    let network = Network::new(&control, 'e');
    let netmap = network.compile(None);
    let DnsPlan::ServeExitResolver { services } = &netmap.netmap.dns.egress else {
        panic!("exit must serve A: {:?}", netmap.netmap.dns.egress)
    };
    assert_eq!(services.len(), 1);
    assert_eq!(services[0].source, network.a);
    let snapshot = sync(&mut control, &script, reply(&netmap, true)).await;
    let system = SystemResolverSnapshot {
        id: "served-provenance-fixture".into(),
        upstreams: Arc::new(vec!["192.0.2.53:53".parse().unwrap()]),
        primary: DnsTransport::Udp,
        invalid: CancellationToken::new(),
        verified_until: Arc::new(StdMutex::new(Instant::now() + Duration::from_secs(30))),
    };
    let scope = Scope::netmap(snapshot, Some(system));
    let exchange = Exchange::new(&[Step::Answer, Step::Fail]);
    let (_sender, receiver) = watch::channel(Some(scope.clone()));
    let dispatcher = OwnerDispatcher::new(receiver, exchange.clone(), limits(), 1232).unwrap();
    // Readiness still requires authenticated DNS-purpose ingress.
    assert!(dispatcher.validate_scope(&scope).is_err());
    let local: SocketAddr = "100.64.0.3:53".parse().unwrap();
    let deadline = || Instant::now() + Duration::from_secs(2);
    let source: SocketAddr = "100.64.0.1:40000".parse().unwrap();

    let answer = dispatcher
        .dispatch_from(
            source,
            local,
            DnsTransport::Udp,
            named("ordinary.test."),
            deadline(),
        )
        .await
        .unwrap();
    answer.ensure_current().unwrap();
    assert_eq!(
        exchange.endpoints(),
        [("192.0.2.53:53".parse().unwrap(), DnsTransport::Udp)]
    );
    // Not a planned source, or a planned source on another address: refused
    // before any upstream I/O.
    for (peer, at) in [
        ("100.64.0.2:40000", local),
        ("100.64.0.1:40000", "100.64.0.4:53".parse().unwrap()),
    ] {
        assert!(matches!(
            dispatcher
                .dispatch_from(
                    peer.parse().unwrap(),
                    at,
                    DnsTransport::Udp,
                    named("x.test."),
                    deadline()
                )
                .await,
            Err(DispatchError::Scope)
        ));
    }
    assert_eq!(exchange.endpoints().len(), 1);
    // An unreachable OS upstream fails the query; nothing else is tried.
    assert!(matches!(
        dispatcher
            .dispatch_from(
                source,
                local,
                DnsTransport::Udp,
                named("ordinary.test."),
                deadline()
            )
            .await,
        Err(DispatchError::Gateway)
    ));
    assert_eq!(exchange.endpoints().len(), 2);
    let _ = control.shutdown().await;
    server.close().await;
}
