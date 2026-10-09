use super::*;

pub(super) fn node(i: u8) -> NodeId {
    format!("00000000-0000-4000-8000-{i:012}").parse().unwrap()
}
pub(super) fn network() -> NetworkId {
    "00000000-0000-4003-8000-000000000001".parse().unwrap()
}
pub(super) fn sel(i: u8) -> String {
    format!("node:{}", node(i))
}
pub(super) fn port(value: u16) -> Option<Port> {
    Some(value.try_into().unwrap())
}

/// Deterministic splitmix64; no external randomness enters the suite.
pub(super) struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0x1234_5678)
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

pub(super) struct Scenario {
    pub cfg: NetworkConfiguration,
    pub subjects: SubjectContext,
    pub inventory: BTreeMap<NodeId, TrustedNode>,
    pub selection_versions: BTreeMap<NodeId, u64>,
}

pub(super) const ALICE: &str = "alice@example.test";
pub(super) const BOB: &str = "bob@example.test";

fn binding(i: u8) -> UserBinding {
    UserBinding {
        user_id: format!("00000000-0000-4006-8000-{i:012}").parse().unwrap(),
        membership_id: format!("00000000-0000-4007-8000-{i:012}").parse().unwrap(),
    }
}

pub(super) fn listener(kind: &str, port: u16) -> PreparedListener {
    let l: Listener =
        serde_json::from_str(&format!(r#"{{"type":"{kind}","listenPort":{port}}}"#)).unwrap();
    PreparedListener::prepare(l).unwrap()
}

pub(super) fn trusted(i: u8, exit: bool, observe: bool) -> TrustedNode {
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
    let transport = |stream| Capability::Transport {
        roles: vec![TransportRole::Dial, TransportRole::Listen],
        streams: vec![stream],
    };
    let mut capabilities = BTreeMap::from([
        ("transport:native_udp".into(), transport(StreamKind::Udp)),
        ("transport:native_tcp".into(), transport(StreamKind::Tcp)),
    ]);
    if exit {
        capabilities.insert(
            "feature:exit-ipv4".into(),
            Capability::Feature { version: 1 },
        );
    }
    if observe {
        capabilities.insert(
            "feature:path-observation".into(),
            Capability::Feature { version: 1 },
        );
    }
    TrustedNode {
        control_key: key(0, KeyAlgorithm::Ed25519),
        dataplane_key: key(100, KeyAlgorithm::X25519),
        capabilities,
    }
}

/// `count` registered nodes; node 1 and 2 are user devices of Alice and Bob,
/// the rest carry tags. Exit-enabled: nodes 4 and 5.
pub(super) fn base(count: u8) -> Scenario {
    let mut cfg = new_network(
        AddressPools {
            ipv4: vec!["100.64.0.0/24".into()],
            ipv6: vec![],
        },
        "test.internal".into(),
    )
    .unwrap();
    // The network template allows all Mesh traffic; scenarios add their own.
    cfg.access.clear();
    for tag in ["tag:server", "tag:edge"] {
        cfg.tags_owners.insert(tag.into(), vec![]);
    }
    cfg.groups.insert("group:ops".into(), vec![ALICE.into()]);
    let mut subjects = SubjectContext::default();
    for (i, user) in [(1, ALICE), (2, BOB)] {
        subjects.users.insert(user.into(), binding(i));
        subjects.members.insert(user.into(), binding(i));
        subjects.node_users.insert(node(i), binding(i));
    }
    let mut inventory = BTreeMap::new();
    for i in 1..=count {
        let identity = match i {
            1 => NodeIdentity::User { user: ALICE.into() },
            2 => NodeIdentity::User { user: BOB.into() },
            i if i % 2 == 1 => NodeIdentity::Tag {
                tags: vec!["tag:server".into()],
            },
            _ => NodeIdentity::Tag {
                tags: vec!["tag:edge".into()],
            },
        };
        let mut listeners = BTreeMap::from([("udp".into(), listener("native_udp", 12345))]);
        if i % 2 == 0 {
            listeners.insert("tcp".into(), listener("native_tcp", 12346));
        }
        cfg.nodes.insert(
            node(i),
            NodeSettings {
                hostname: format!("n{i}"),
                remark: String::new(),
                identity,
                enabled: true,
                addresses: NodeAddresses {
                    ipv4: Some(format!("100.64.0.{i}").parse().unwrap()),
                    ipv6: None,
                },
                listeners,
                dialers: Input::Omitted,
                exit_enabled: i == 4 || i == 5,
            },
        );
        inventory.insert(node(i), trusted(i, i >= 4, i != 3));
    }
    Scenario {
        cfg,
        subjects,
        inventory,
        selection_versions: BTreeMap::new(),
    }
}

pub(super) fn access(
    src: &[String],
    dst: &[String],
    ip: &[&str],
    bidirectional: bool,
) -> AccessRule {
    AccessRule {
        remark: String::new(),
        src: src.to_vec(),
        dst: dst.to_vec(),
        ip: ip.iter().map(|s| (*s).into()).collect(),
        bidirectional,
    }
}

pub(super) fn egress_rule(prefix: &str, action: RuleAction) -> EgressRule {
    EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            ip_cidr: vec![prefix.into()],
            domain: vec![],
            domain_suffix: vec![],
            rule_sets: vec![],
        },
        action,
    }
}

pub(super) fn exit_action(nodes: &[u8], via: &[u8]) -> RuleAction {
    RuleAction::Exit {
        nodes: nodes.iter().map(|i| sel(*i)).collect(),
        via: via.iter().map(|i| sel(*i)).collect(),
    }
}

/// Configuration shapes: via, bidirectional, CIDR overlap, exits with
/// primary/backup order, group/tag/user/node selectors, disabled and
/// unregistered nodes, topology blocks and carrier restrictions.
pub(super) fn generated(seed: u64) -> Scenario {
    let mut rng = Rng::new(seed);
    let mut s = base(5);
    if rng.chance(30) {
        // Configured but never registered: addresses exist, no inventory.
        let mut extra = s.cfg.nodes[&node(3)].clone();
        extra.hostname = "n6".into();
        extra.addresses.ipv4 = Some("100.64.0.6".parse().unwrap());
        s.cfg.nodes.insert(node(6), extra);
    }
    if rng.chance(25) {
        s.cfg.nodes.get_mut(&node(5)).unwrap().enabled = false;
    }
    if rng.chance(20) {
        s.cfg.nodes.get_mut(&node(2)).unwrap().enabled = false;
    }
    if rng.chance(20) {
        s.inventory
            .get_mut(&node(4))
            .unwrap()
            .capabilities
            .remove("feature:exit-ipv4");
    }
    let mesh = [
        "*".to_owned(),
        "tag:server".into(),
        "tag:edge".into(),
        "group:ops".into(),
        BOB.into(),
        sel(1),
        sel(2),
        sel(3),
        sel(4),
        sel(5),
    ];
    let external = [
        "autogroup:internet".to_owned(),
        "10.20.0.0/16".into(),
        "8.0.0.0/8".into(),
        "192.168.1.0/24".into(),
    ];
    let services = [
        "tcp:443",
        "tcp:8000-8100",
        "udp:*",
        "icmp:*",
        "*",
        "tcp:22",
        "udp:53",
    ];
    for _ in 0..(2 + rng.below(4)) {
        let src: Vec<String> = (0..1 + rng.below(2))
            .map(|_| rng.pick(&mesh).clone())
            .collect();
        let to_external = rng.chance(35);
        let dst: Vec<String> = (0..1 + rng.below(2))
            .map(|_| {
                if to_external {
                    rng.pick(&external).clone()
                } else {
                    rng.pick(&mesh).clone()
                }
            })
            .collect();
        let ip: Vec<&str> = (0..1 + rng.below(2))
            .map(|_| *rng.pick(&services))
            .collect();
        let bidirectional = !to_external && rng.chance(40);
        s.cfg.access.push(access(&src, &dst, &ip, bidirectional));
    }
    if rng.chance(25) {
        // Two islands, sometimes bridged by 3-4: projections become partial.
        s.cfg.defaults.topology = TopologyDefault::Deny;
        let mut edges = vec![(1, 2), (2, 3), (1, 3), (4, 5)];
        if rng.chance(50) {
            edges.push((3, 4));
        }
        for (a, b) in edges {
            s.cfg.topology.push(TopologyRule {
                remark: String::new(),
                action: TopologyAction::Allow,
                a: sel(a),
                b: sel(b),
                transports: Input::Omitted,
            });
        }
    }
    if rng.chance(50) {
        let (a, b, via) = (
            rng.below(5) as u8 + 1,
            rng.below(5) as u8 + 1,
            rng.below(5) as u8 + 1,
        );
        s.cfg.routing.mesh.push(MeshRoute {
            remark: String::new(),
            src: vec![sel(a)],
            dst: vec![if rng.chance(50) { sel(b) } else { "*".into() }],
            via: vec![sel(via)],
        });
    }
    if rng.chance(25) {
        s.cfg.routing.mesh.push(MeshRoute {
            remark: String::new(),
            src: vec!["tag:edge".into()],
            dst: vec!["tag:server".into()],
            via: vec![sel(3)],
        });
    }
    if rng.chance(30) {
        let (a, b) = (rng.below(5) as u8 + 1, rng.below(5) as u8 + 1);
        s.cfg.topology.push(TopologyRule {
            remark: String::new(),
            action: TopologyAction::Block,
            a: sel(a),
            b: sel(b),
            transports: Input::Omitted,
        });
    }
    if rng.chance(30) {
        s.cfg.topology.push(TopologyRule {
            remark: String::new(),
            action: TopologyAction::Allow,
            a: "tag:edge".into(),
            b: "*".into(),
            transports: Input::Value(vec![
                TransportProtocol::NativeTcp,
                TransportProtocol::NativeUdp,
            ]),
        });
    }
    let candidates = [
        egress_rule("8.0.0.0/8", exit_action(&[4], &[3])),
        egress_rule("8.8.0.0/16", RuleAction::Direct),
        egress_rule("10.0.0.0/8", RuleAction::Direct),
        egress_rule("10.20.0.0/16", exit_action(&[5, 4], &[])),
        egress_rule("192.168.0.0/16", exit_action(&[4, 5], &[2])),
    ];
    let mut rules: Vec<EgressRule> = candidates
        .iter()
        .filter(|_| rng.chance(50))
        .cloned()
        .collect();
    if rng.chance(50) {
        rules.reverse();
    }
    let fallback = match rng.below(3) {
        0 => RouteAction::Unmanaged,
        1 => RouteAction::Exit {
            nodes: vec![sel(4), sel(5)],
            via: vec![],
        },
        _ => RouteAction::Exit {
            nodes: vec![sel(5)],
            via: vec![sel(3)],
        },
    };
    s.cfg
        .routing
        .egress
        .profiles
        .insert("exit".into(), RoutingProfile { rules, fallback });
    let target = if rng.chance(50) {
        vec!["group:ops".to_owned(), "tag:edge".into()]
    } else {
        vec!["*".to_owned()]
    };
    s.cfg.routing.egress.assignments.push(ProfileAssignment {
        target,
        allowed: vec!["exit".into(), "mesh".into()],
        default: if rng.chance(70) { "exit" } else { "mesh" }.into(),
        locked: false,
    });
    s
}

pub(super) fn netmaps(s: &Scenario) -> CompiledNetmaps {
    compile_with(s, &BTreeMap::new(), Revision::MIN)
}

pub(super) fn compile_with(
    s: &Scenario,
    previous: &BTreeMap<NodeId, PreviousNetmap>,
    revision: Revision,
) -> CompiledNetmaps {
    compile_netmap(NetmapInput {
        network: network(),
        revision,
        configuration: &s.cfg,
        subjects: &s.subjects,
        inventory: &s.inventory,
        rule_sets: &BTreeMap::new(),
        selected_profiles: &BTreeMap::new(),
        selection_versions: &s.selection_versions,
        previous,
    })
    .unwrap()
}

pub(super) fn previous(compiled: &CompiledNetmaps) -> BTreeMap<NodeId, PreviousNetmap> {
    compiled
        .nodes
        .iter()
        .map(|(id, netmap)| (*id, PreviousNetmap::from(netmap)))
        .collect()
}

pub(super) fn lease(netmap: &CompiledNetmap) -> NetmapLease {
    NetmapLease::new(netmap.reference(), 0, 1_000_000).unwrap()
}
