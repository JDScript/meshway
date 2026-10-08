use super::*;

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
            transports: Input::Omitted,
        },
        TopologyRule {
            remark: String::new(),
            action: TopologyAction::Allow,
            a: "*".into(),
            b: "*".into(),
            transports: Input::Omitted,
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
            transports: Input::Omitted,
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
    // The group rule chooses 3 for 1→2; the separate 3→2 segment then
    // matches the wildcard rule and must also traverse 4.
    assert_eq!(
        live.nodes[&node(1)].configuration.permits[&request(1)].paths[0].nodes,
        vec![node(1), node(3), node(4), node(2)]
    );
    c.topology.push(TopologyRule {
        remark: String::new(),
        action: TopologyAction::Block,
        a: "group:team".into(),
        b: "*".into(),
        transports: Input::Omitted,
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
