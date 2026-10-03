use meshway_configuration::{compile::*, execution::*, *};
use meshway_model::state::{Capability, StreamKind, TransportRole};
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
                        Capability::Transport {
                            roles: vec![TransportRole::Dial, TransportRole::Listen],
                            streams: vec![StreamKind::Udp],
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
    run_with_subjects(c, inv, demands, dns, &SubjectContext::default())
}
fn run_with_subjects(
    c: &NetworkConfiguration,
    inv: &BTreeMap<NodeId, TrustedNode>,
    demands: &BTreeMap<RequestId, Demand>,
    dns: &[TrustedDnsAssociation],
    subjects: &SubjectContext,
) -> CompiledPublication {
    compile(CompilationInput {
        network: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
        revision: Revision::MIN,
        publication: "00000000-0000-4004-8000-000000000001".parse().unwrap(),
        configuration: c,
        subjects,
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
    // TUN packets contain an IP, not the browser's domain string. The same
    // trusted association must drive routing and bound the grant lifetime.
    let mut ips = ds.clone();
    ips.get_mut(&request(1)).unwrap().target = BusinessTarget::Ip {
        address: ExecutionAddress("1.1.1.1".parse().unwrap()),
    };
    let ip_result = run(&c, &inventory(), &ips, &dns);
    let permit = &ip_result.nodes[&node(1)].configuration.permits[&request(1)];
    assert_eq!(permit.expires_at_ms, 2000);
    assert!(permit.paths.iter().all(|p| p.nodes.contains(&node(3))));
    // Even a non-DNS route must not extend a domain-only access grant.
    c.routing.egress.rules[0].matcher.ip_cidr = vec!["1.1.1.1/32".into()];
    assert_eq!(
        run(&c, &inventory(), &ips, &dns).nodes[&node(1)]
            .configuration
            .permits[&request(1)]
            .expires_at_ms,
        2000
    );
    let mut stale = dns.clone();
    stale[0].expires_at_ms = 1000;
    assert!(matches!(
        run(&c, &inventory(), &ips, &stale).nodes[&node(1)]
            .configuration
            .decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::Unauthorized
        }
    ));
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
#[test]
fn mesh_ip_demands_never_fall_through_to_unmanaged_egress() {
    let mut c = configuration();
    let mut d = demand(1, 1, 2);
    d.target = BusinessTarget::Ip {
        address: ExecutionAddress("100.64.0.2".parse().unwrap()),
    };
    let mut ds = BTreeMap::from([(d.id, d)]);
    let result = run(&c, &inventory(), &ds, &[]);
    assert!(
        matches!(result.nodes[&node(1)].configuration.permits[&request(1)].target,BusinessTarget::Mesh{node:n} if n==node(2))
    );
    c.access.clear();
    let result = run(&c, &inventory(), &ds, &[]);
    assert!(matches!(
        result.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::Unauthorized
        }
    ));
    ds.get_mut(&request(1)).unwrap().target = BusinessTarget::Ip {
        address: ExecutionAddress("100.64.0.99".parse().unwrap()),
    };
    let result = run(&c, &inventory(), &ds, &[]);
    assert!(matches!(
        result.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::NoPath
        }
    ));
}

#[test]
fn backup_exit_survives_primary_path_budget() {
    let mut c = configuration();
    c.nodes.get_mut(&node(2)).unwrap().exit_enabled = true;
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["1.1.1.1/32".into()],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    c.routing.egress.rules = vec![EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            ip_cidr: vec!["1.1.1.1/32".into()],
            ..Default::default()
        },
        action: RouteAction::Exit {
            nodes: vec![format!("node:{}", node(4)), format!("node:{}", node(2))],
            via: vec![],
        },
    }];
    let mut d = demand(1, 1, 4);
    d.target = BusinessTarget::Ip {
        address: ExecutionAddress("1.1.1.1".parse().unwrap()),
    };
    let result = run(&c, &inventory(), &BTreeMap::from([(d.id, d)]), &[]);
    let paths = &result.nodes[&node(1)].configuration.permits[&request(1)].paths;
    assert!(paths.len() <= 2 * MAX_PATHS);
    assert_eq!(
        paths
            .iter()
            .filter(|p| p.nodes.last() == Some(&node(4)))
            .count(),
        MAX_PATHS
    );
    assert!(paths.iter().any(|p| p.nodes.last() == Some(&node(2))));
    // A local failure of every primary path still leaves an authorized backup.
    assert!(
        paths
            .iter()
            .any(|p| !p.nodes.contains(&node(4)) && p.nodes.last() == Some(&node(2)))
    );
}

#[test]
fn incompatible_tls_pair_does_not_poison_udp() {
    let mut c = configuration();
    let l: Listener = serde_json::from_str(r#"{"type":"vless","listenPort":443,"uuid":"00000000-0000-4000-8000-000000000001","tls":{"serverName":"test.example","minVersion":"1.3","maxVersion":"1.3","certificate":"certificate","privateKey":"private-key"}}"#).unwrap();
    c.nodes
        .get_mut(&node(2))
        .unwrap()
        .listeners
        .insert("vless".into(), PreparedListener::prepare(l, true).unwrap());
    c.defaults.dialers.vless.as_mut().unwrap().tls.max_version = TlsVersion::Tls12;
    let mut inv = inventory();
    for n in inv.values_mut() {
        n.capabilities.insert(
            "transport:vless".into(),
            Capability::Transport {
                roles: vec![TransportRole::Dial, TransportRole::Listen],
                streams: vec![StreamKind::Tcp],
            },
        );
    }
    let ds = BTreeMap::from([(request(1), demand(1, 1, 2))]);
    let result = run(&c, &inv, &ds, &[]);
    assert!(!result.nodes[&node(1)].configuration.permits.is_empty());
    assert!(
        result.nodes[&node(1)]
            .configuration
            .dialers
            .values()
            .all(|candidate| !matches!(candidate.parameters, DialParameters::Vless { .. }))
    );
    for n in inv.values_mut() {
        n.capabilities.remove("transport:native_udp");
    }
    let unavailable = run(&c, &inv, &ds, &[]);
    assert!(matches!(
        unavailable.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::NoPath
        }
    ));
}

#[test]
fn neighbor_probe_projection_is_not_a_business_path_or_grant() {
    let mut c = configuration();
    // A-C-B, with no alternative graph edges. Only A may initiate business.
    c.topology = vec![(1, 3), (3, 2)]
        .into_iter()
        .map(|(a, b)| TopologyRule {
            remark: String::new(),
            action: TopologyAction::Allow,
            a: format!("node:{}", node(a)),
            b: format!("node:{}", node(b)),
        })
        .collect();
    c.defaults.topology = TopologyDefault::Deny;
    c.routing.mesh = vec![MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec![format!("node:{}", node(2))],
        via: vec![format!("node:{}", node(3))],
    }];
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec![format!("node:{}", node(2))],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    // Only C listens: both receiver-side and initiator-side projections matter.
    c.nodes.get_mut(&node(1)).unwrap().listeners.clear();
    c.nodes.get_mut(&node(2)).unwrap().listeners.clear();
    let ds = BTreeMap::from([(request(1), demand(1, 1, 2)), (request(2), demand(2, 3, 2))]);
    let r = run(&c, &inventory(), &ds, &[]);
    let a = &r.nodes[&node(1)].configuration;
    let b = &r.nodes[&node(2)].configuration;
    let relay = &r.nodes[&node(3)].configuration;
    assert_eq!(
        a.neighbor_transports().keys().copied().collect::<Vec<_>>(),
        vec![node(3)]
    );
    assert_eq!(
        b.neighbor_transports().keys().copied().collect::<Vec<_>>(),
        vec![node(3)]
    );
    assert_eq!(
        relay
            .neighbor_transports()
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![node(1), node(2)]
    );
    assert!(
        relay
            .neighbor_transports()
            .values()
            .all(|values| values == &std::collections::BTreeSet::from(["native_udp".into()]))
    );
    assert_eq!(
        a.permits[&request(1)].paths[0].nodes,
        vec![node(1), node(3), node(2)]
    );
    assert!(matches!(
        relay.decisions[&request(2)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::Unauthorized
        }
    ));
    let empty = run(&c, &inventory(), &BTreeMap::new(), &[]);
    assert!(
        empty
            .nodes
            .values()
            .all(|n| n.configuration.neighbor_transports().is_empty())
    );
}

#[test]
fn unavailable_rule_content_cannot_fall_through_to_direct() {
    let mut c = configuration();
    c.rule_sets.insert(
        "remote".into(),
        RuleSet::Remote {
            url: "https://rules.example.test/remote.json".into(),
            format: RuleSetFormat::SingBoxSource,
            update_interval: "24h".parse().unwrap(),
        },
    );
    c.routing.egress.rules = vec![
        EgressRule {
            remark: String::new(),
            matcher: RouteMatch {
                rule_sets: vec!["remote".into()],
                ..Default::default()
            },
            action: RouteAction::Exit {
                nodes: vec![format!("node:{}", node(4))],
                via: vec![],
            },
        },
        EgressRule {
            remark: String::new(),
            matcher: RouteMatch {
                ip_cidr: vec!["1.1.1.1/32".into()],
                ..Default::default()
            },
            action: RouteAction::Direct,
        },
    ];
    let mut d = demand(1, 1, 4);
    d.target = BusinessTarget::Ip {
        address: ExecutionAddress("1.1.1.1".parse().unwrap()),
    };
    let result = run(&c, &inventory(), &BTreeMap::from([(d.id, d)]), &[]);
    assert_eq!(
        result.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::MissingDependency
        }
    );
}

#[test]
fn stable_membership_controls_user_groups_topology_and_routing() {
    let mut c = configuration();
    let email = "user@example.test".to_owned();
    c.nodes.get_mut(&node(1)).unwrap().identity = NodeIdentity::User {
        user: email.clone(),
    };
    c.groups.insert("group:team".into(), vec![email.clone()]);
    let old = UserBinding {
        user_id: "00000000-0000-4005-8000-000000000001".parse().unwrap(),
        membership_id: "00000000-0000-4006-8000-000000000001".parse().unwrap(),
    };
    let current = UserBinding {
        membership_id: "00000000-0000-4006-8000-000000000002".parse().unwrap(),
        ..old.clone()
    };
    let subjects = SubjectContext {
        users: BTreeMap::from([(email.clone(), old)]),
        node_users: BTreeMap::from([(node(1), current.clone())]),
        members: BTreeMap::from([(email.clone(), current.clone())]),
    };
    let ds = BTreeMap::from([(request(1), demand(1, 1, 2))]);
    let grant = |src| AccessRule {
        remark: String::new(),
        src: vec![src],
        dst: vec![format!("node:{}", node(2))],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    };
    for selector in [email.clone(), "group:team".into()] {
        c.access = vec![grant(selector)];
        let result = run_with_subjects(&c, &inventory(), &ds, &[], &subjects);
        assert_eq!(
            result.nodes[&node(1)].configuration.decisions[&request(1)],
            DemandDecision::Unavailable {
                reason: UnavailableReason::Unauthorized
            }
        );
    }
    for selector in [format!("node:{}", node(1)), "*".into()] {
        c.access = vec![grant(selector)];
        assert!(matches!(
            run_with_subjects(&c, &inventory(), &ds, &[], &subjects).nodes[&node(1)]
                .configuration
                .decisions[&request(1)],
            DemandDecision::Permitted { .. }
        ));
    }
    c.routing.mesh = vec![
        MeshRoute {
            remark: String::new(),
            src: vec!["group:team".into()],
            dst: vec!["*".into()],
            via: vec![format!("node:{}", node(3))],
        },
        MeshRoute {
            remark: String::new(),
            src: vec!["*".into()],
            dst: vec!["*".into()],
            via: vec![format!("node:{}", node(4))],
        },
    ];
    let inert = run_with_subjects(&c, &inventory(), &ds, &[], &subjects);
    assert_eq!(
        inert.nodes[&node(1)].configuration.permits[&request(1)].paths[0].nodes,
        vec![node(1), node(4), node(2)]
    );
    let mut active = subjects.clone();
    active.users.insert(email, current);
    let live = run_with_subjects(&c, &inventory(), &ds, &[], &active);
    assert_ne!(inert.dependency_digest, live.dependency_digest);
    assert_eq!(
        live.nodes[&node(1)].configuration.permits[&request(1)].paths[0].nodes,
        vec![node(1), node(3), node(2)]
    );
    c.topology.push(TopologyRule {
        remark: String::new(),
        action: TopologyAction::Block,
        a: "group:team".into(),
        b: "*".into(),
    });
    assert!(matches!(
        run_with_subjects(&c, &inventory(), &ds, &[], &subjects).nodes[&node(1)]
            .configuration
            .decisions[&request(1)],
        DemandDecision::Permitted { .. }
    ));
    assert_eq!(
        run_with_subjects(&c, &inventory(), &ds, &[], &active).nodes[&node(1)]
            .configuration
            .decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::NoPath
        }
    );
    c.topology.clear();
    c.routing.egress.profiles.insert(
        "work".into(),
        serde_json::from_str(r#"{"rules":[],"fallback":{"type":"unmanaged"}}"#).unwrap(),
    );
    c.routing.egress.assignments = serde_json::from_str(
        r#"[{"target":["group:team"],"allowed":["work"],"default":"work","locked":false}]"#,
    )
    .unwrap();
    assert!(!profile_allowed(&c, &subjects, node(1), "work").unwrap());
    assert!(profile_allowed(&c, &subjects, node(1), "mesh").unwrap());
    assert!(profile_allowed(&c, &active, node(1), "work").unwrap());
}
