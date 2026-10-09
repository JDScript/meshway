use super::*;
use meshway_policy::admission::{Hop, Traffic, admit_forward, admit_reply};

fn route(source: u8, target: u8, via: &[u8]) -> MeshRoute {
    let selector = |id| {
        if id == 0 {
            "*".into()
        } else {
            format!("node:{}", node(id))
        }
    };
    MeshRoute {
        remark: String::new(),
        src: vec![selector(source)],
        dst: vec![selector(target)],
        via: via.iter().map(|id| selector(*id)).collect(),
    }
}

fn fixture(count: u8) -> (NetworkConfiguration, BTreeMap<NodeId, TrustedNode>) {
    let mut c = configuration();
    let mut inv = inventory();
    for i in 5..=count {
        let mut settings = c.nodes[&node(4)].clone();
        settings.hostname = format!("n{i}");
        settings.addresses.ipv4 = Some(format!("100.64.0.{i}").parse().unwrap());
        settings.exit_enabled = false;
        c.nodes.insert(node(i), settings);
        let mut trusted = inv[&node(4)].clone();
        for (offset, key) in [
            (0, &mut trusted.control_key),
            (10, &mut trusted.dataplane_key),
        ] {
            key.id = format!("00000000-0000-4002-8000-{:012}", i + offset)
                .parse()
                .unwrap();
            key.public_key.bytes = [i + offset; 32].into();
        }
        inv.insert(node(i), trusted);
    }
    (c, inv)
}

fn paths(publication: &CompiledPublication) -> Vec<Vec<NodeId>> {
    publication.nodes[&node(1)].configuration.permits[&request(1)]
        .paths
        .iter()
        .map(|p| p.nodes.clone())
        .collect()
}

#[test]
fn recursive_waypoints_keep_one_business_grant_and_exact_reverse_admission() {
    let (mut c, inv) = fixture(4);
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec![format!("node:{}", node(4))],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    c.routing.mesh = vec![route(1, 4, &[3]), route(1, 3, &[2])];
    let context = ValidationContext {
        nodes: c.nodes.keys().copied().collect(),
        approved_identities: c
            .nodes
            .iter()
            .map(|(id, s)| (*id, s.identity.clone()))
            .collect(),
        ..Default::default()
    };
    let (c, _) = meshway_policy::normalize((&c).into(), &context).unwrap();
    let demands = BTreeMap::from([
        (request(1), demand(1, 1, 4)),
        (request(2), demand(2, 1, 3)),
        (request(3), demand(3, 3, 4)),
        (request(4), demand(4, 4, 1)),
    ]);
    let publication = run(&c, &inv, &demands, &[]);
    let path = vec![node(1), node(2), node(3), node(4)];
    assert_eq!(paths(&publication), vec![path.clone()]);
    for (source, id) in [(1, 2), (3, 3), (4, 4)] {
        assert_eq!(
            publication.nodes[&node(source)].configuration.decisions[&request(id)],
            DemandDecision::Unavailable {
                reason: UnavailableReason::Unauthorized
            }
        );
    }
    let traffic = Traffic {
        source: "100.64.0.1".parse().unwrap(),
        destination: "100.64.0.4".parse().unwrap(),
        source_port: Some(Port::new(50000).unwrap()),
        destination_port: Some(Port::new(443).unwrap()),
        protocol: BusinessProtocol::Tcp,
    };
    let reply = Traffic {
        source: traffic.destination,
        destination: traffic.source,
        source_port: traffic.destination_port,
        destination_port: traffic.source_port,
        ..traffic
    };
    for (position, local) in path.iter().enumerate() {
        let cfg = &publication.nodes[local].configuration;
        assert_eq!(cfg.permits.len(), 1);
        assert_eq!(cfg.permits[&request(1)].source, node(1));
        assert_eq!(
            cfg.permits[&request(1)].target,
            BusinessTarget::Mesh { node: node(4) }
        );
        let previous = position.checked_sub(1).map(|i| path[i]);
        let next = path.get(position + 1).copied();
        assert_eq!(
            admit_forward(cfg, request(1), &path, node(1), previous, &traffic, 1000),
            Ok(next.map_or(Hop::Deliver, Hop::Forward))
        );
        assert_eq!(
            admit_reply(
                cfg,
                request(1),
                &path,
                node(1),
                next,
                &traffic,
                &reply,
                1000
            ),
            Ok(previous.map_or(Hop::Deliver, Hop::Forward))
        );
        assert!(admit_forward(cfg, request(1), &path, node(1), previous, &reply, 1000).is_err());
        assert!(
            admit_forward(cfg, request(1), &path, node(1), previous, &traffic, 100_000).is_err()
        );
    }
    let a = &publication.nodes[&node(1)].configuration;
    assert!(!a.dns.mesh_hosts.contains_key("n2.test.internal"));
    assert!(!a.dns.mesh_hosts.contains_key("n3.test.internal"));
    assert!(
        admit_forward(
            a,
            request(1),
            &[node(1), node(3), node(4)],
            node(1),
            None,
            &traffic,
            1000
        )
        .is_err()
    );
}

#[test]
fn each_segment_matches_its_actual_endpoints_and_first_rule_only() {
    let (mut c, inv) = fixture(5);
    c.routing.mesh = vec![
        route(1, 4, &[3]),
        route(1, 3, &[2]),
        route(1, 3, &[5]),
        route(3, 4, &[5]),
    ];
    let ds = BTreeMap::from([(request(1), demand(1, 1, 4))]);
    assert_eq!(
        paths(&run(&c, &inv, &ds, &[])),
        vec![vec![node(1), node(2), node(3), node(5), node(4)]]
    );
    // Reordering the subsegment rule changes its requirement; the resulting
    // duplicate 5 is a cycle, not permission to select the next matching rule.
    c.routing.mesh.swap(1, 2);
    let publication = run(&c, &inv, &ds, &[]);
    assert_eq!(
        publication.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::RoutingCycle
        }
    );
}

#[test]
fn wildcard_waypoint_at_segment_boundaries_is_already_satisfied() {
    let (mut c, inv) = fixture(4);
    let ds = BTreeMap::from([(request(1), demand(1, 1, 4))]);
    for rule in [
        route(1, 0, &[3]),
        route(0, 4, &[3]),
        route(0, 0, &[3]),
        route(1, 4, &[1, 3, 4]),
    ] {
        c.routing.mesh = vec![rule];
        let result = run(&c, &inv, &ds, &[]);
        assert_eq!(paths(&result)[0], vec![node(1), node(3), node(4)]);
        assert!(paths(&result).iter().all(|path| path.contains(&node(3))));
    }
    c.routing.mesh = vec![route(1, 4, &[3, 1])];
    assert_eq!(
        run(&c, &inv, &ds, &[]).nodes[&node(1)]
            .configuration
            .decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::RoutingCycle
        }
    );
}

#[test]
fn recursive_cycles_are_per_demand_errors_and_never_fall_back() {
    let (mut c, inv) = fixture(4);
    c.routing.mesh = vec![
        route(1, 4, &[3]),
        route(1, 3, &[2]),
        route(1, 2, &[3]),
        route(1, 4, &[]),
    ];
    let ds = BTreeMap::from([(request(1), demand(1, 1, 4)), (request(2), demand(2, 2, 4))]);
    let result = run(&c, &inv, &ds, &[]);
    assert_eq!(
        result.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::RoutingCycle
        }
    );
    assert!(
        result
            .nodes
            .values()
            .all(|n| !n.configuration.permits.contains_key(&request(1)))
    );
    assert!(
        result.nodes[&node(2)]
            .configuration
            .permits
            .contains_key(&request(2))
    );
}

#[test]
fn every_expanded_hop_requires_inventory_topology_and_transport_and_recovers() {
    let (mut c, inv) = fixture(4);
    c.routing.mesh = vec![route(1, 4, &[3]), route(1, 3, &[2]), route(1, 4, &[])];
    let ds = BTreeMap::from([(request(1), demand(1, 1, 4))]);
    let expected = vec![vec![node(1), node(2), node(3), node(4)]];
    assert_eq!(paths(&run(&c, &inv, &ds, &[])), expected);
    for failure in 0..4 {
        let mut c = c.clone();
        let mut inv = inv.clone();
        match failure {
            0 => {
                inv.remove(&node(2));
            }
            1 => {
                c.nodes.get_mut(&node(2)).unwrap().enabled = false;
            }
            2 => c.topology.push(TopologyRule {
                transports: Input::Omitted,
                remark: String::new(),
                action: TopologyAction::Block,
                a: format!("node:{}", node(2)),
                b: format!("node:{}", node(3)),
 keepalive: false,
            }),
            _ => {
                inv.get_mut(&node(2)).unwrap().capabilities.clear();
            }
        }
        let result = run(&c, &inv, &ds, &[]);
        assert_eq!(
            result.nodes[&node(1)].configuration.decisions[&request(1)],
            DemandDecision::Unavailable {
                reason: UnavailableReason::NoPath
            }
        );
        assert!(
            result
                .nodes
                .values()
                .all(|n| !n.configuration.permits.contains_key(&request(1)))
        );
    }
    assert_eq!(paths(&run(&c, &inv, &ds, &[])), expected);
}

#[test]
fn automatic_bridges_obey_segment_rules_and_preserve_waypoint_order() {
    let (mut c, inv) = fixture(5);
    c.defaults.topology = TopologyDefault::Deny;
    c.topology = [(1, 2), (2, 3), (2, 5), (5, 3), (3, 4)]
        .map(|(a, b)| TopologyRule {
            transports: Input::Omitted,
            remark: String::new(),
            action: TopologyAction::Allow,
            a: format!("node:{}", node(a)),
            b: format!("node:{}", node(b)),
 keepalive: false,
        })
        .to_vec();
    c.routing.mesh = vec![route(1, 4, &[3]), route(2, 3, &[5])];
    let ds = BTreeMap::from([(request(1), demand(1, 1, 4))]);
    assert_eq!(
        paths(&run(&c, &inv, &ds, &[])),
        vec![vec![node(1), node(2), node(5), node(3), node(4)]]
    );
}

#[test]
fn path_and_expansion_limits_are_bounded_without_truncating_required_nodes() {
    let (mut c, inv) = fixture(9);
    let ds = BTreeMap::from([(request(1), demand(1, 1, 9))]);
    c.routing.mesh = (3..=9)
        .map(|target| route(1, target, &[target - 1]))
        .collect();
    assert_eq!(
        run(&c, &inv, &ds, &[]).nodes[&node(1)]
            .configuration
            .decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::RoutingLimit
        }
    );
    c.routing.mesh = (3..=8)
        .map(|target| route(1, target, &[target - 1]))
        .collect();
    let ds = BTreeMap::from([(request(1), demand(1, 1, 8))]);
    assert_eq!(
        paths(&run(&c, &inv, &ds, &[])),
        vec![(1..=8).map(node).collect::<Vec<_>>()]
    );
}

#[test]
fn unrelated_overlong_candidate_does_not_poison_a_complete_valid_path() {
    let (mut c, inv) = fixture(9);
    c.routing.mesh = [(4, 3), (3, 5), (5, 6), (6, 7), (7, 8), (8, 9), (9, 1)]
        .map(|(target, via)| route(2, target, &[via]))
        .to_vec();
    let ds = BTreeMap::from([(request(1), demand(1, 1, 4))]);
    assert_eq!(paths(&run(&c, &inv, &ds, &[]))[0], vec![node(1), node(4)]);
}

#[test]
fn diagnostic_requested_path_uses_recursive_compilation_without_a_business_grant() {
    let (mut c, inv) = fixture(4);
    c.routing.mesh = vec![route(1, 4, &[3]), route(1, 3, &[2])];
    let id = ObservationId::new(request(1).into_uuid()).unwrap();
    let path = vec![node(1), node(2), node(3), node(4)];
    for requested in [path.clone(), vec![node(1), node(3), node(4)]] {
        let diagnostic = DiagnosticDemand {
            id,
            source: node(1),
            target: node(4),
            instance_id: InstanceId::new(request(2).into_uuid()).unwrap(),
            service: ObservationServiceScope::IcmpEcho,
            path: Some(requested.clone()),
            expires_at_ms: 10_000,
        };
        let publication = compile_with_diagnostics(
            CompilationInput {
                network: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
                revision: Revision::MIN,
                publication: "00000000-0000-4004-8000-000000000001".parse().unwrap(),
                configuration: &c,
                subjects: &SubjectContext::default(),
                inventory: &inv,
                demands: &BTreeMap::new(),
                dns: &[],
                rule_sets: &BTreeMap::new(),
                selected_profiles: &BTreeMap::new(),
                now_ms: 1000,
            },
            &BTreeMap::from([(id, diagnostic)]),
        )
        .unwrap();
        for node in publication.nodes.values() {
            assert!(node.configuration.permits.is_empty());
            assert!(node.configuration.decisions.is_empty());
            assert_eq!(
                node.configuration.diagnostics.contains_key(&id),
                requested == path
            );
            if requested == path {
                assert_eq!(node.configuration.diagnostics[&id].paths[0].nodes, path);
            }
        }
    }
}

#[test]
fn exit_internal_segments_recurse_without_reauthorizing_relays_as_business_sources() {
    let (mut c, inv) = fixture(4);
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
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
        action: RuleAction::Exit {
            nodes: vec![format!("node:{}", node(4))],
            via: vec![format!("node:{}", node(3))],
        },
    }];
    c.routing.mesh = vec![route(1, 3, &[2])];
    let mut d = demand(1, 1, 4);
    d.target = BusinessTarget::Ip {
        address: ExecutionAddress("1.1.1.1".parse().unwrap()),
    };
    let ds = BTreeMap::from([(d.id, d.clone())]);
    let result = run(&c, &inv, &ds, &[]);
    assert_eq!(
        paths(&result),
        vec![vec![node(1), node(2), node(3), node(4)]]
    );
    for node in result.nodes.values() {
        let permit = &node.configuration.permits[&request(1)];
        assert_eq!(permit.source, d.source);
        assert_eq!(permit.target, d.target);
        assert!(node.configuration.local_egress.is_empty());
    }
    c.routing.mesh.push(route(1, 2, &[3]));
    let result = run(&c, &inv, &ds, &[]);
    assert_eq!(
        result.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::RoutingCycle
        }
    );
    assert!(
        result
            .nodes
            .values()
            .all(|n| n.configuration.local_egress.is_empty() && n.configuration.permits.is_empty())
    );
}
