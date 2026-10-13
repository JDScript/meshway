//! Netmap scopes. The production Control issues every snapshot from a real
//! enrollment; the sync replies carry netmaps compiled by the shared Policy
//! compiler for that enrolled identity. Only resolver I/O is scripted.
use super::*;
use meshway_configuration::{
    AccessRule, DnsServer, EgressRule, ProfileAssignment, RouteAction, RouteMatch, RoutingProfile,
    RuleAction,
    execution::TrustedNode,
    netmap::{CompiledNetmap, DnsPlan, ResolverAction},
};
use meshway_model::NodeId;
use meshway_protocol::SyncResponse;
use meshway_test_server::{
    Facts, Script,
    configuration::{network, node, sel, tag_node, trusted},
};

const HOUR: u64 = 60 * 60 * 1000;

mod associations;
mod served;

#[tokio::test]
async fn netmap_query_capacity_cancellation_and_unsent_answer_deadlines_are_independent() {
    let server = support::Server::start().await;
    let (mut control, script) = enrolled(&server).await;
    let network = Network::new(&control, 'a');
    let netmap = network.compile(None);
    let snapshot = sync(&mut control, &script, reply(&netmap, true)).await;
    let mut owner = Owner::new(&snapshot, limits(), &CancellationToken::new()).unwrap();
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
    let mut other = Owner::new(&snapshot, limits(), &CancellationToken::new()).unwrap();
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
    include_mesh_resolver: bool,
    override_local: bool,
    /// `chat.example` and its subdomains exit by name ahead of every other rule.
    name_rule: bool,
    peer_keys: BTreeMap<NodeId, TrustedNode>,
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
            include_mesh_resolver: true,
            override_local: false,
            name_rule: false,
            peer_keys: BTreeMap::new(),
        }
    }

    fn compile(&self, previous: Option<&CompiledNetmap>) -> CompiledNetmap {
        let mut cfg = network("test.internal");
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
        let listener = meshway_test_server::configuration::listener(
            serde_json::json!({"type": "native_udp", "listenPort": 12345}),
        );
        for (id, i, hostname) in [
            (self.a, 1, self.hostname.as_str()),
            (self.b, 2, "b"),
            (self.e, 3, "e"),
        ] {
            cfg.nodes.insert(
                id,
                tag_node(
                    hostname,
                    i,
                    BTreeMap::from([("udp".into(), listener.clone())]),
                    id == self.e,
                ),
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
        if self.name_rule {
            let profile = cfg.routing.egress.profiles.get_mut("exit").unwrap();
            profile.rules.insert(
                0,
                EgressRule {
                    remark: String::new(),
                    matcher: RouteMatch {
                        domain_suffix: vec!["chat.example".into()],
                        ..Default::default()
                    },
                    action: RuleAction::Exit {
                        nodes: vec![sel(self.e)],
                        via: vec![],
                    },
                },
            );
            profile.fallback = RouteAction::Direct;
        }
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
        if self.override_local {
            cfg.dns.override_local = true;
            cfg.dns.default = vec!["direct".into()];
        }
        if !self.include_mesh_resolver {
            cfg.dns.servers.remove("mesh");
            cfg.dns.routes.remove("mesh.example");
        }
        let mut inventory = BTreeMap::from([
            (self.a, trusted(1, false)),
            (self.b, trusted(2, false)),
            (self.e, trusted(3, true)),
        ]);
        inventory.insert(self.enrolled, self.keys.clone());
        inventory.extend(self.peer_keys.clone());
        meshway_test_server::configuration::compile_one(
            self.network,
            &cfg,
            &inventory,
            self.enrolled,
            previous,
        )
    }
}

fn reply(netmap: &CompiledNetmap, full: bool) -> SyncResponse {
    meshway_test_server::reply(netmap, HOUR, full)
}

async fn enrolled(server: &support::Server) -> (Control, Arc<Script>) {
    let mut control = server.control();
    server.enroll(&mut control).await;
    let script = Arc::new(Script::default());
    control.set_sync_transport(script.clone());
    (control, script)
}

/// One sync; Control's verified netmap as Engine's desired content.
async fn sync(control: &mut Control, script: &Script, reply: SyncResponse) -> Desired {
    script.push(Ok(reply));
    control.sync(&mut Facts::default()).await.unwrap();
    Desired::new(&control.netmap().unwrap().content, &Default::default()).unwrap()
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
    // Kept under the exit (`useWithExit`), an address the exit would route
    // is this Node's direct exception.
    assert_eq!(dns.resolvers["routed"], ResolverAction::Direct);
    assert_eq!(dns.resolvers["mesh"], ResolverAction::Rejected);
    assert!(matches!(dns.egress, DnsPlan::ViaExit { .. }));
    let snapshot = sync(&mut control, &script, reply(&netmap, true)).await;
    let mut owner = Owner::new(&snapshot, limits(), &CancellationToken::new()).unwrap();
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
    let Begin::Upstream(work) = owner.begin_published_until(named("x.routed.example."), deadline)
    else {
        panic!("a kept resolver is routed locally")
    };
    let ResolverRoute::Local(route) = work.route() else {
        panic!("a configured resolver is local")
    };
    assert_eq!(route.endpoint(), "203.0.113.53:53".parse().unwrap());
    // A Mesh resolver never gets a local socket, nor a fallback.
    assert!(matches!(
        owner.begin_published_until(named("x.mesh.example."), deadline),
        Begin::Unavailable(Unavailable::MissingResolverPlan)
    ));
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
    // `meshway ping` reports the same routes without a ticket.
    use crate::{NameRoute, name_route};
    assert_eq!(
        name_route(&snapshot, "x.direct.example"),
        NameRoute::Resolvers(vec!["direct".into()])
    );
    assert_eq!(
        name_route(&snapshot, "x.routed.example"),
        NameRoute::Resolvers(vec!["routed".into()])
    );
    assert_eq!(
        name_route(&snapshot, "x.mesh.example"),
        NameRoute::Unavailable
    );
    assert_eq!(
        name_route(&snapshot, "ordinary.test"),
        NameRoute::Exit(network.e)
    );
    assert_eq!(
        name_route(&snapshot, "b.test.internal."),
        NameRoute::MagicDns
    );
    // Readiness fails closed: a declared resolver without a local route.
    assert_eq!(
        Scope::netmap(snapshot, None).validate_resolvers(),
        Err(DispatchError::Unavailable(Unavailable::MissingResolverPlan))
    );
    let _ = control.shutdown().await;
    server.close().await;
}

#[tokio::test]
async fn same_content_keeps_owner_while_new_content_ends_its_answers() {
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

    // Same content: the scope is equivalent and keeps its owner.
    let renewed = sync(&mut control, &script, reply(&first, false)).await;
    assert_eq!(renewed.reference(), snapshot.reference());
    crate::publish_scope(&sender, Some(Scope::netmap(renewed.clone(), None)));
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

    // Withdrawn and republished with the same content before any query saw
    // the withdrawal: the withdrawn owner never serves the new scope.
    crate::publish_scope(&sender, None);
    assert!(answer.ensure_current().is_err());
    crate::publish_scope(&sender, Some(Scope::netmap(renewed, None)));
    let answer = dispatcher
        .dispatch(named("b.test.internal."), deadline)
        .await
        .unwrap();
    answer.ensure_current().unwrap();
    let generation = {
        let state = dispatcher.state.lock().await;
        let current = state.owner.as_ref().unwrap().generation();
        assert!(!Arc::ptr_eq(&current, &generation));
        current
    };

    // A larger serial ends old answers and replaces the owner generation.
    network.hostname = "a2".into();
    let second = network.compile(Some(&first));
    assert!(second.netmap.serial > first.netmap.serial);
    let next = sync(&mut control, &script, reply(&second, true)).await;
    answer.ensure_current().unwrap();
    // Publishing new content ends the old scope's answers at once.
    crate::publish_scope(&sender, Some(Scope::netmap(next, None)));
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
async fn exit_resolver_serves_only_authenticated_requests_of_its_current_plan() {
    let server = support::Server::start().await;
    let (mut control, script) = enrolled(&server).await;
    let mut network = Network::new(&control, 'e');
    network.include_mesh_resolver = false;
    let mut source = server.control_named("source");
    server.enroll_peer(&control, &mut source).await;
    let source_script = Arc::new(Script::default());
    source.set_sync_transport(source_script.clone());
    let mut source_network = Network::new(&source, 'a');
    source_network.e = network.e;
    source_network.include_mesh_resolver = false;
    network.a = source_network.a;
    network
        .peer_keys
        .insert(source_network.a, source_network.keys.clone());
    source_network
        .peer_keys
        .insert(network.e, network.keys.clone());
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
        provenance: crate::system::Provenance::Original,
        invalid: CancellationToken::new(),
        verified_until: Arc::new(StdMutex::new(Instant::now() + Duration::from_secs(30))),
    };
    let scope = Scope::netmap(snapshot.clone(), None).serving(Some(system.clone()));
    let exchange = Exchange::new(&[Step::Answer, Step::Fail]);
    let (sender, receiver) = watch::channel(Some(scope.clone()));
    let dispatcher = OwnerDispatcher::new(receiver, exchange.clone(), limits(), 1232).unwrap();
    dispatcher.validate_scope(&scope).unwrap();
    let deadline = || Instant::now() + Duration::from_secs(2);
    // The only Mesh DNS entry is a request authenticated by SecureNode on a
    // DNS-purpose circuit; it reaches only the verified system resolver.
    // Both endpoint identities are issued by real Controls.
    let source_map = source_network.compile(None);
    sync(&mut source, &source_script, reply(&source_map, true)).await;
    let DnsPlan::ViaExit { service } = &source_map.netmap.dns.egress else {
        panic!("source DNS service");
    };
    let carrier = meshway_dataplane::secure::Carrier {
        dialer: network.a,
        listener: network.e,
        listener_key: "udp".into(),
        protocol: meshway_configuration::TransportProtocol::NativeUdp,
    };
    let packet = |control: &Control| {
        (
            meshway_engine::Desired::new(&control.netmap().unwrap().content, &Default::default())
                .unwrap(),
            control.dataplane_keys().unwrap(),
        )
    };
    let (desired, keys) = packet(&source);
    let mut origin = meshway_dataplane::secure::SecureNode::new(
        &desired,
        keys,
        Default::default(),
        BTreeMap::from([(network.e, carrier.clone())]),
    )
    .unwrap();
    let grant = meshway_dataplane::secure::CircuitGrant {
        purpose: meshway_dataplane::Purpose::Dns,
        target: meshway_dataplane::Target::Mesh { node: network.e },
    };
    // Readiness is a selection fact: no resolver circuit to a not-ready exit.
    assert!(
        origin
            .start_grant(grant.clone(), service.path.nodes.clone(), &carrier)
            .is_err()
    );
    origin.set_ready_exits([network.e].into());
    let (desired, keys) = packet(&control);
    let mut exit = meshway_dataplane::secure::SecureNode::new(
        &desired,
        keys,
        Default::default(),
        BTreeMap::from([(network.a, carrier.clone())]),
    )
    .unwrap();
    let init = origin
        .start_grant(grant, service.path.nodes.clone(), &carrier)
        .unwrap();
    let cid = init.circuit();
    use meshway_dataplane::secure::Received;
    let Received::Established {
        response: Some(response),
        ..
    } = exit
        .receive(&carrier, init.release(|b| b).unwrap())
        .unwrap()
    else {
        panic!("resolver handshake");
    };
    assert!(matches!(
        origin
            .receive(&carrier, response.release(|b| b).unwrap())
            .unwrap(),
        Received::Established { response: None, .. }
    ));
    for succeeds in [true, false] {
        let id =
            meshway_model::EventId::new(uuid::Uuid::from_u128(if succeeds { 100 } else { 101 }))
                .unwrap();
        let wire = origin
            .query_dns(
                cid,
                id,
                DnsTransport::Udp,
                &named("ordinary.test.").encode_upstream(1232).unwrap(),
                deadline(),
            )
            .unwrap();
        let Received::DnsRequest(request) = exit
            .receive(&carrier, wire.release(|b| b).unwrap())
            .unwrap()
        else {
            panic!("authenticated request");
        };
        // A failed upstream is answered SERVFAIL on the original path; the
        // source never tries another resolver.
        let bytes = dispatcher.serve_authenticated(&request).await.unwrap();
        let code = Message::from_vec(&bytes).unwrap().metadata.response_code;
        assert_eq!(
            code,
            if succeeds {
                hickory_proto::op::ResponseCode::NoError
            } else {
                hickory_proto::op::ResponseCode::ServFail
            }
        );
        let reply = exit.answer_dns(request, &bytes).unwrap();
        assert!(matches!(
            origin
                .receive(&carrier, reply.release(|b| b).unwrap())
                .unwrap(),
            Received::DnsReply(_)
        ));
    }
    assert_eq!(
        exchange.endpoints(),
        [("192.0.2.53:53".parse().unwrap(), DnsTransport::Udp); 2]
    );
    // An authenticated request is still answered SERVFAIL without any
    // upstream when the exit has no verified system snapshot, or once the
    // exit's netmap content is no longer the one the request is bound to.
    let wire = origin
        .query_dns(
            cid,
            meshway_model::EventId::new(uuid::Uuid::from_u128(102)).unwrap(),
            DnsTransport::Udp,
            &named("ordinary.test.").encode_upstream(1232).unwrap(),
            deadline(),
        )
        .unwrap();
    let Received::DnsRequest(request) = exit
        .receive(&carrier, wire.release(|b| b).unwrap())
        .unwrap()
    else {
        panic!("authenticated request");
    };
    let servfail = |bytes: Option<bytes::Bytes>| {
        Message::from_vec(&bytes.expect("SERVFAIL reply"))
            .unwrap()
            .metadata
            .response_code
            == hickory_proto::op::ResponseCode::ServFail
    };
    let (_unserved, unserved) = watch::channel(Some(Scope::netmap(snapshot.clone(), None)));
    let unserving = OwnerDispatcher::new(unserved, exchange.clone(), limits(), 1232).unwrap();
    assert!(servfail(unserving.serve_authenticated(&request).await));
    network.hostname = "e2".into();
    let next = network.compile(Some(&netmap));
    let next = sync(&mut control, &script, reply(&next, true)).await;
    crate::publish_scope(
        &sender,
        Some(Scope::netmap(next, None).serving(Some(system))),
    );
    assert!(servfail(dispatcher.serve_authenticated(&request).await));
    assert_eq!(exchange.endpoints().len(), 2);
    source.shutdown().await.unwrap();
    let _ = control.shutdown().await;
    server.close().await;
}
