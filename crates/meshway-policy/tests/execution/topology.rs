use super::*;

fn dual_transport() -> (NetworkConfiguration, BTreeMap<NodeId, TrustedNode>) {
    let mut c = configuration();
    let mut inv = inventory();
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["*".into()],
        ip: vec!["*".into()],
        bidirectional: false,
    }];
    for settings in c.nodes.values_mut() {
        let listener =
            serde_json::from_value(serde_json::json!({"type":"native_tcp", "listenPort":12346}))
                .unwrap();
        settings
            .listeners
            .insert("tcp".into(), PreparedListener::prepare(listener).unwrap());
    }
    for trusted in inv.values_mut() {
        trusted.capabilities.insert(
            "transport:native_tcp".into(),
            Capability::Transport {
                roles: vec![TransportRole::Dial, TransportRole::Listen],
                streams: vec![StreamKind::Tcp],
            },
        );
    }
    c.defaults.dialers =
        serde_json::from_value(serde_json::json!({"native_udp":{}, "native_tcp":{}})).unwrap();
    (c, inv)
}

fn edge(a: &str, b: &str, protocols: &[TransportProtocol]) -> TopologyRule {
    TopologyRule {
        remark: String::new(),
        action: TopologyAction::Allow,
        a: a.into(),
        b: b.into(),
        transports: Input::Value(protocols.to_vec()),
    },
    keepalive: false,
}

#[test]
fn edge_intersection_filters_both_dial_directions_and_leaves_other_edges_unrestricted() {
    let (mut c, inv) = dual_transport();
    c.topology = vec![edge(
        &format!("node:{}", node(1)),
        &format!("node:{}", node(2)),
        &[TransportProtocol::NativeTcp],
    )];
    let ds = BTreeMap::from([(request(1), demand(1, 1, 2)), (request(2), demand(2, 1, 3))]);
    let result = run(&c, &inv, &ds, &[]);
    for (a, b) in [(node(1), node(2)), (node(2), node(1))] {
        let candidates: Vec<_> = result.nodes[&a]
            .configuration
            .dialers
            .values()
            .filter(|d| d.peer == b)
            .collect();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].listener_key, "tcp");
        for (protocol, key, permitted) in [
            (TransportProtocol::NativeTcp, "tcp", true),
            (TransportProtocol::NativeUdp, "udp", false),
        ] {
            let carrier = Carrier {
                dialer: a,
                listener: b,
                listener_key: key.into(),
                protocol,
            };
            for (local, peer) in [(a, b), (b, a)] {
                assert_eq!(
                    meshway_policy::admission::admit_adjacency(
                        &result.nodes[&local].configuration,
                        peer,
                        &carrier,
                        1000
                    )
                    .is_ok(),
                    permitted
                );
            }
        }
    }
    assert!(
        result.nodes[&node(1)]
            .configuration
            .dialers
            .values()
            .any(|d| d.peer == node(3) && d.listener_key == "udp")
    );
    assert!(
        result.nodes[&node(1)]
            .configuration
            .dialers
            .values()
            .any(|d| d.peer == node(3) && d.listener_key == "tcp")
    );
}

#[test]
fn edge_order_ranks_candidates_and_unlisted_edges_use_the_platform_order() {
    let (mut c, inv) = dual_transport();
    // 1--2 lists TCP before UDP; 1--3 has no rule, so the platform order applies.
    c.topology = vec![edge(
        &format!("node:{}", node(1)),
        &format!("node:{}", node(2)),
        &[TransportProtocol::NativeTcp, TransportProtocol::NativeUdp],
    )];
    let ds = BTreeMap::from([(request(1), demand(1, 1, 2)), (request(2), demand(2, 1, 3))]);
    let result = run(&c, &inv, &ds, &[]);
    let order = |peer: NodeId| {
        let mut candidates: Vec<_> = result.nodes[&node(1)]
            .configuration
            .dialers
            .values()
            .filter(|d| d.peer == peer)
            .collect();
        candidates.sort_by_key(|d| (d.priority, d.listener_key.clone()));
        candidates
            .iter()
            .map(|d| d.listener_key.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(order(node(2)), ["tcp", "udp"]);
    assert_eq!(order(node(3)), ["udp", "tcp"]);
}

#[test]
fn first_allow_controls_transport_set_and_block_wins_in_either_order() {
    let (mut c, inv) = dual_transport();
    c.topology = vec![
        edge("*", "*", &[TransportProtocol::NativeTcp]),
        edge("*", "*", &[TransportProtocol::NativeUdp]),
    ];
    let ds = BTreeMap::from([(request(1), demand(1, 1, 2))]);
    for expected in ["tcp", "udp"] {
        let result = run(&c, &inv, &ds, &[]);
        assert!(
            result
                .nodes
                .values()
                .flat_map(|n| n.configuration.dialers.values())
                .all(|d| d.listener_key == expected)
        );
        c.topology.reverse();
    }
    let mut block = edge("*", "*", &[]);
    block.action = TopologyAction::Block;
    c.topology.push(block);
    for _ in 0..2 {
        assert!(
            run(&c, &inv, &ds, &[])
                .nodes
                .values()
                .all(|n| n.configuration.permits.is_empty())
        );
        c.topology.reverse();
    }
}

#[test]
fn reverse_dial_satisfies_edge_without_changing_business_direction() {
    let (mut c, inv) = dual_transport();
    c.topology = vec![edge("*", "*", &[TransportProtocol::NativeTcp])];
    c.nodes.get_mut(&node(1)).unwrap().dialers = Input::Value(DialerSet::default());
    c.nodes.get_mut(&node(2)).unwrap().listeners.clear();
    let result = run(
        &c,
        &inv,
        &BTreeMap::from([(request(1), demand(1, 1, 2))]),
        &[],
    );
    let a = &result.nodes[&node(1)].configuration;
    assert!(a.dialers.is_empty());
    assert_eq!(a.permits[&request(1)].paths[0].nodes, [node(1), node(2)]);
    assert!(
        a.permits[&request(1)].paths[0].dial_candidates[&node(2)]
            .contains(&format!("{}/tcp", node(1)))
    );
}

#[test]
fn empty_or_incompatible_edge_does_not_fallback_or_remove_required_hop() {
    let (mut c, inv) = dual_transport();
    c.routing.mesh = vec![MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec![format!("node:{}", node(3))],
        via: vec![format!("node:{}", node(2))],
    }];
    let ds = BTreeMap::from([(request(1), demand(1, 1, 3)), (request(2), demand(2, 3, 4))]);
    for protocols in [vec![], vec![TransportProtocol::Vless]] {
        c.topology = vec![edge(&format!("node:{}", node(2)), "*", &protocols)];
        let result = run(&c, &inv, &ds, &[]);
        assert!(matches!(
            result.nodes[&node(1)].configuration.decisions[&request(1)],
            DemandDecision::Unavailable {
                reason: UnavailableReason::NoPath
            }
        ));
        assert!(
            !result.nodes[&node(1)]
                .configuration
                .permits
                .contains_key(&request(1))
        );
        assert!(
            result.nodes[&node(3)]
                .configuration
                .permits
                .contains_key(&request(2))
        );
        assert!(result.nodes[&node(2)].configuration.dialers.is_empty());
    }
}

#[test]
fn wildcard_and_tag_expansion_intersect_each_pair_with_its_own_listeners() {
    let (mut c, inv) = dual_transport();
    c.topology = vec![edge("tag:server", "*", &[TransportProtocol::NativeTcp])];
    c.nodes.get_mut(&node(2)).unwrap().listeners.remove("tcp");
    c.nodes.get_mut(&node(2)).unwrap().dialers =
        Input::Value(serde_json::from_value(serde_json::json!({"native_udp":{}})).unwrap());
    let ds = BTreeMap::from([(request(1), demand(1, 1, 2)), (request(2), demand(2, 1, 3))]);
    let result = run(&c, &inv, &ds, &[]);
    assert!(matches!(
        result.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::NoPath
        }
    ));
    assert!(
        result.nodes[&node(1)]
            .configuration
            .permits
            .contains_key(&request(2))
    );
}

#[test]
fn vless_and_trojan_constraints_never_fallback_to_available_native_carriers() {
    for protocol in [TransportProtocol::Vless, TransportProtocol::Trojan] {
        let (mut c, mut inv) = dual_transport();
        c.nodes.retain(|id, _| *id == node(1) || *id == node(2));
        c.topology = vec![edge("*", "*", &[protocol])];
        let name = protocol.as_str();
        let mut value = serde_json::json!({"type": name,"listenPort":443,"tls":{"serverName":"test.example","certificate":"certificate","privateKey":"private-key"}});
        if protocol == TransportProtocol::Vless {
            value["uuid"] = serde_json::json!("00000000-0000-4000-8000-000000000001");
        } else {
            value["password"] = serde_json::json!("test-carrier-password");
        }
        c.nodes.get_mut(&node(2)).unwrap().listeners.insert(
            name.into(),
            PreparedListener::prepare(serde_json::from_value(value).unwrap()).unwrap(),
        );
        c.defaults.dialers = DialerSet::network_default();
        c.defaults.dialers.trojan = Some(TrojanDialer::default());
        for n in inv.values_mut() {
            n.capabilities.insert(
                format!("transport:{name}"),
                Capability::Transport {
                    roles: vec![TransportRole::Dial, TransportRole::Listen],
                    streams: vec![StreamKind::Tcp],
                },
            );
        }
        let ds = BTreeMap::from([(request(1), demand(1, 1, 2))]);
        let result = run(&c, &inv, &ds, &[]);
        let a = &result.nodes[&node(1)].configuration;
        assert_eq!(a.dialers.len(), 1);
        assert_eq!(a.dialers.values().next().unwrap().listener_key, name);
        assert_eq!(a.permits[&request(1)].paths[0].nodes, [node(1), node(2)]);
        inv.get_mut(&node(1))
            .unwrap()
            .capabilities
            .remove(&format!("transport:{name}"));
        let result = run(&c, &inv, &ds, &[]);
        assert!(matches!(
            result.nodes[&node(1)].configuration.decisions[&request(1)],
            DemandDecision::Unavailable {
                reason: UnavailableReason::NoPath
            }
        ));
        assert!(result.nodes[&node(1)].configuration.dialers.is_empty());
    }
}

#[test]
fn unavailable_edge_plan_names_deciding_topology_rule_and_pair() {
    let (mut c, inv) = dual_transport();
    c.nodes.retain(|id, _| *id == node(1) || *id == node(2));
    c.topology = vec![edge("*", "*", &[])];
    let error = plan(PlanRequest {
        topology: Topology {
            configuration: &c,
            subjects: &SubjectContext::default(),
            inventory: &inv,
        },
        source: node(1),
        destination: PlanDestination::Node(node(2)),
        profile: None,
        rule_sets: &BTreeMap::new(),
        selected_profiles: &BTreeMap::new(),
    })
    .unwrap_err();
    assert_eq!(
        error,
        CompileError::EdgeTransportUnavailable {
            rule: 0,
            a: node(1),
            b: node(2)
        }
    );
    assert!(error.to_string().contains("/topology/0/transports"));
}
