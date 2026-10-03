use meshway_configuration::{compile::*, execution::*, *};
use meshway_model::*;
use meshway_transport_spec::*;
use std::collections::BTreeMap;
fn node(i: u8) -> NodeId {
    format!("00000000-0000-4000-8000-{i:012}").parse().unwrap()
}
fn request(i: u8) -> RequestId {
    format!("00000000-0000-4001-8000-{i:012}").parse().unwrap()
}
fn configuration() -> NetworkConfiguration {
    let mut c = new_network(
        AddressPools {
            ipv4: vec!["100.64.0.0/24".into()],
            ipv6: vec![],
        },
        "test.internal".into(),
    )
    .unwrap();
    c.tags_owners.insert("tag:server".into(), vec![]);
    for i in 1..=4 {
        let l: Listener =
            serde_json::from_str(r#"{"type":"native_udp","listenPort":12345}"#).unwrap();
        c.nodes.insert(
            node(i),
            NodeSettings {
                hostname: format!("n{i}"),
                remark: String::new(),
                identity: NodeIdentity::Tag {
                    tags: vec!["tag:server".into()],
                },
                enabled: true,
                addresses: NodeAddresses {
                    ipv4: Some(format!("100.64.0.{i}").parse().unwrap()),
                    ipv6: None,
                },
                listeners: BTreeMap::from([(
                    "udp".into(),
                    PreparedListener::prepare(l, false).unwrap(),
                )]),
                dialers: Input::Omitted,
                exit_enabled: i == 4,
            },
        );
    }
    c
}
fn inventory() -> BTreeMap<NodeId, TrustedNode> {
    (1..=4)
        .map(|i| {
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
            (
                node(i),
                TrustedNode {
                    control_key: key(0, KeyAlgorithm::Ed25519),
                    dataplane_key: key(10, KeyAlgorithm::X25519),
                    capabilities: BTreeMap::from([(
                        "transport:native_udp".into(),
                        TransportCapability {
                            dial: true,
                            listen: true,
                            streams: vec!["udp".into()],
                        },
                    )]),
                },
            )
        })
        .collect()
}
fn demand(i: u8, source: u8, target: u8) -> Demand {
    Demand {
        id: request(i),
        source: node(source),
        target: BusinessTarget::Mesh { node: node(target) },
        protocol: BusinessProtocol::Tcp,
        port: Some(443.try_into().unwrap()),
        profile: "mesh".into(),
        expires_at_ms: 100_000,
    }
}
fn run(
    c: &NetworkConfiguration,
    inv: &BTreeMap<NodeId, TrustedNode>,
    demands: &BTreeMap<RequestId, Demand>,
    dns: &[TrustedDnsAssociation],
) -> CompiledPublication {
    compile(CompilationInput {
        network: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
        revision: Revision::MIN,
        publication: "00000000-0000-4004-8000-000000000001".parse().unwrap(),
        configuration: c,
        inventory: inv,
        demands,
        dns,
        rule_sets: &BTreeMap::new(),
        selected_profiles: &BTreeMap::new(),
        now_ms: 1000,
    })
    .unwrap()
}
#[test]
fn union_demands_exact_reverse_and_mandatory_hop() {
    let mut c = configuration();
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec![format!("node:{}", node(2)), format!("node:{}", node(4))],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    c.routing.mesh = vec![MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec![format!("node:{}", node(2))],
        via: vec![format!("node:{}", node(3))],
    }];
    let ds = BTreeMap::from([
        (request(1), demand(1, 1, 2)),
        (request(2), demand(2, 1, 4)),
        (request(3), demand(3, 2, 1)),
    ]);
    let result = run(&c, &inventory(), &ds, &[]);
    let a = &result.nodes[&node(1)].configuration;
    assert_eq!(a.permits.len(), 2);
    assert!(
        a.permits[&request(1)].paths.iter().all(|p| p
            .nodes
            .iter()
            .position(|n| *n == node(3))
            .unwrap()
            < p.nodes.len() - 1)
    );
    assert_eq!(
        a.permits[&request(1)].replies,
        ReplyPolicy::StatefulExactReversePath
    );
    assert!(matches!(
        result.nodes[&node(2)].configuration.decisions[&request(3)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::Unauthorized
        }
    ));
    assert!(
        result.nodes[&node(3)]
            .configuration
            .permits
            .contains_key(&request(1))
    );
    assert_eq!(
        a.permits[&request(1)],
        result.nodes[&node(2)].configuration.permits[&request(1)]
    );
    let mut inv = inventory();
    inv.remove(&node(3));
    let result = run(&c, &inv, &ds, &[]);
    assert!(matches!(
        result.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::NoPath
        }
    ));
}
#[test]
fn block_wins_reverse_dial_and_expired_demand_removes_permissions() {
    let mut c = configuration();
    c.nodes.get_mut(&node(1)).unwrap().listeners.clear();
    let ds = BTreeMap::from([(request(1), demand(1, 1, 2))]);
    let r = run(&c, &inventory(), &ds, &[]);
    assert!(!r.nodes[&node(1)].configuration.dialers.is_empty());
    assert!(
        r.nodes[&node(2)]
            .configuration
            .dialers
            .values()
            .all(|d| d.peer != node(1))
    );
    c.topology = vec![
        TopologyRule {
            remark: String::new(),
            action: TopologyAction::Block,
            a: format!("node:{}", node(1)),
            b: "*".into(),
        },
        TopologyRule {
            remark: String::new(),
            action: TopologyAction::Allow,
            a: "*".into(),
            b: "*".into(),
        },
    ];
    let r = run(&c, &inventory(), &ds, &[]);
    assert!(r.nodes[&node(1)].configuration.permits.is_empty());
    let mut ds = ds;
    ds.get_mut(&request(1)).unwrap().expires_at_ms = 1000;
    let r = run(&c, &inventory(), &ds, &[]);
    assert!(r.nodes.values().all(|n| n.configuration.permits.is_empty()));
}
#[test]
fn egress_first_match_no_fallback_on_revocation_and_dns_is_trusted() {
    let mut c = configuration();
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["domain:example.com".into()],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    c.routing.egress.rules = vec![EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            domain_suffix: vec!["example.com".into()],
            ..Default::default()
        },
        action: RouteAction::Exit {
            nodes: vec![format!("node:{}", node(4))],
            via: vec![format!("node:{}", node(3))],
        },
    }];
    let mut d = demand(1, 1, 4);
    d.target = BusinessTarget::Domain {
        name: "api.example.com".into(),
        address: ExecutionAddress("1.1.1.1".parse().unwrap()),
    };
    let ds = BTreeMap::from([(d.id, d)]);
    let r = run(&c, &inventory(), &ds, &[]);
    assert!(matches!(
        r.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::MissingDns
        }
    ));
    let dns = vec![TrustedDnsAssociation {
        network_id: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
        configuration_revision: Revision::MIN,
        source: node(1),
        name: "api.example.com".into(),
        cname_chain: vec![],
        addresses: vec![ExecutionAddress("1.1.1.1".parse().unwrap())],
        expires_at_ms: 2000,
    }];
    let r = run(&c, &inventory(), &ds, &dns);
    assert_eq!(
        r.nodes[&node(1)].configuration.permits[&request(1)].expires_at_ms,
        2000
    );
    c.nodes.get_mut(&node(4)).unwrap().exit_enabled = false;
    let r = run(&c, &inventory(), &ds, &dns);
    assert!(matches!(
        r.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::NoPath
        }
    ));
}
#[test]
fn digest_includes_capabilities_destinations_ports_and_dns() {
    let c = configuration();
    let inv = inventory();
    let mut ds = BTreeMap::from([(request(1), demand(1, 1, 2))]);
    let first = run(&c, &inv, &ds, &[]).dependency_digest;
    ds.get_mut(&request(1)).unwrap().port = Some(80.try_into().unwrap());
    assert_ne!(first, run(&c, &inv, &ds, &[]).dependency_digest);
    ds.get_mut(&request(1)).unwrap().port = Some(443.try_into().unwrap());
    let mut inv = inv;
    inv.get_mut(&node(2)).unwrap().capabilities.clear();
    assert_ne!(first, run(&c, &inv, &ds, &[]).dependency_digest);
}
#[test]
fn node_delta_removes_permissions_and_checks_the_exact_base_and_target() {
    let c = configuration();
    let inv = inventory();
    let ds = BTreeMap::from([(request(1), demand(1, 1, 2))]);
    let old = run(&c, &inv, &ds, &[]).nodes.remove(&node(1)).unwrap();
    let new = run(&c, &inv, &BTreeMap::new(), &[])
        .nodes
        .remove(&node(1))
        .unwrap();
    let delta = configuration_delta(&old, &new).unwrap();
    assert_eq!(delta.permits.remove, vec![request(1)]);
    assert!(delta.listeners.upsert.is_empty());
    assert_eq!(apply_configuration_delta(&old, delta.clone()).unwrap(), new);
    let mut bad = delta.clone();
    bad.base.digest = [99; 32].into();
    assert!(apply_configuration_delta(&old, bad).is_err());
    let mut bad = delta;
    bad.target.digest = [99; 32].into();
    assert!(apply_configuration_delta(&old, bad).is_err());
    assert!(!old.configuration.permits.is_empty());
}
