use super::*;
use meshway_model::TransportProtocol;

fn constrained_route() -> NetworkConfigurationInput {
    let mut c = with_nodes();
    c.groups
        .insert("group:team".into(), vec!["alice@example.com".into()]);
    c.topology = vec![TopologyRule {
        remark: String::new(),
        action: TopologyAction::Allow,
        a: "group:team".into(),
        b: "*".into(),
        transports: Input::Value(vec![TransportProtocol::NativeTcp]),
        keepalive: false,
    }];
    c.routing.mesh = vec![MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{}", id(1))],
        dst: vec![format!("node:{}", id(3))],
        via: vec![format!("node:{}", id(2))],
    }];
    for node in c.nodes.values_mut() {
        node.listeners = serde_json::from_value(
            serde_json::json!({"tcp":{"type":"native_tcp","listenPort":12345}}),
        )
        .unwrap();
    }
    c.defaults.dialers =
        serde_json::from_value(serde_json::json!({"native_tcp":{}, "native_udp":{}})).unwrap();
    c
}

#[test]
fn group_expanded_routing_edge_reports_rule_and_pair_in_preview_and_normalization() {
    let mut c = constrained_route();
    // Only 2--3 is impossible: 1 can still dial 2 and 2 can still dial 1.
    c.nodes.get_mut(&id(3)).unwrap().listeners.clear();
    c.nodes.get_mut(&id(3)).unwrap().dialers = Input::Value(DialerSet::default());
    let check = |error| {
        let ConfigurationError::Invalid(issues) = error else {
            panic!("expected configuration issue");
        };
        let issue = issues
            .iter()
            .find(|issue| issue.code == IssueCode::InvalidTransport)
            .unwrap();
        assert_eq!(issue.path, "/topology/0/transports");
        assert_eq!(issue.params["reason"], "edge_transport_unavailable");
        assert_eq!(issue.params["a"], id(2).to_string());
        assert_eq!(issue.params["b"], id(3).to_string());
        assert_eq!(issue.params["routingRule"], "/routing/mesh/0");
    };
    check(plan_configuration(c.clone(), &context()).err().unwrap());
    check(normalize(c, &context()).unwrap_err());
}

#[test]
fn repeated_transport_in_an_edge_order_is_reported_at_its_position() {
    let mut c = constrained_route();
    c.topology[0].transports = Input::Value(vec![
        TransportProtocol::NativeTcp,
        TransportProtocol::NativeUdp,
        TransportProtocol::NativeTcp,
    ]);
    let ConfigurationError::Invalid(issues) = normalize(c, &context()).unwrap_err() else {
        panic!("expected configuration issue");
    };
    let issue = issues
        .iter()
        .find(|issue| issue.params.get("reason") == Some(&"repeated_transport".into()))
        .unwrap();
    assert_eq!(issue.code, IssueCode::InvalidTransport);
    assert_eq!(issue.path, "/topology/0/transports/2");
}

#[test]
fn retained_transport_loss_is_inert_and_does_not_block_revocation() {
    let c = constrained_route();
    let normalized = normalize_with_dependencies(c.clone(), &context()).unwrap();
    let mut ctx = context();
    ctx.previous = Some(&normalized.configuration);
    ctx.previous_user_bindings = normalized.user_bindings.clone();
    ctx.previous_node_user_bindings = normalized.node_user_bindings.clone();
    let mut c = c;
    c.nodes.get_mut(&id(3)).unwrap().listeners.clear();
    c.nodes.get_mut(&id(3)).unwrap().dialers = Input::Value(DialerSet::default());
    let next = normalize_with_dependencies(c, &ctx).unwrap();
    assert!(
        next.unavailable_dependencies
            .iter()
            .any(|i| i.code == IssueCode::InvalidTransport
                && i.params["reason"] == "edge_transport_unavailable")
    );
}

#[test]
fn declaration_check_accepts_reverse_dial_and_first_allow_is_not_a_union() {
    let mut c = constrained_route();
    c.nodes.get_mut(&id(1)).unwrap().dialers = Input::Value(DialerSet::default());
    assert!(plan_configuration(c.clone(), &context()).is_ok());
    c.topology[0].transports = Input::Value(vec![]);
    let mut later = c.topology[0].clone();
    later.transports = Input::Value(vec![TransportProtocol::NativeTcp]);
    c.topology.push(later);
    assert!(normalize(c.clone(), &context()).is_err());
    c.topology.reverse();
    assert!(normalize(c, &context()).is_ok());
}

#[test]
fn constrained_direct_edge_can_use_a_valid_automatic_bridge_without_skipping_waypoints() {
    let mut c = constrained_route();
    c.routing.mesh[0].via.clear();
    c.topology[0].a = format!("node:{}", id(1));
    c.topology[0].b = format!("node:{}", id(3));
    c.topology[0].transports = Input::Value(vec![]);
    // 1--3 is not an actual edge of the available route 1--2--3.
    assert!(normalize(c.clone(), &context()).is_ok());
    c.nodes.get_mut(&id(2)).unwrap().listeners.clear();
    c.nodes.get_mut(&id(2)).unwrap().dialers = Input::Value(DialerSet::default());
    assert!(normalize(c, &context()).is_err());
}

#[test]
fn route_order_is_retained_and_topology_block_always_wins() {
    let mut c = with_nodes();
    c.routing.mesh = vec![
        MeshRoute {
            remark: "first".into(),
            src: vec!["*".into()],
            dst: vec!["*".into()],
            via: vec![format!("node:{}", id(3))],
        },
        MeshRoute {
            remark: "second".into(),
            src: vec!["*".into()],
            dst: vec!["*".into()],
            via: vec![],
        },
    ];
    c.topology = vec![
        TopologyRule {
            remark: "".into(),
            action: TopologyAction::Block,
            transports: Input::Omitted,
            a: "*".into(),
            b: "*".into(),
            keepalive: false,
        },
        TopologyRule {
            remark: "".into(),
            action: TopologyAction::Allow,
            transports: Input::Omitted,
            a: "*".into(),
            b: "*".into(),
            keepalive: false,
        },
    ];
    let (mut c, _) = normalize(c, &context()).unwrap();
    assert_eq!(c.routing.mesh[0].remark, "first");
    assert_eq!(c.routing.mesh[1].remark, "second");
    assert!(!topology_allows(
        &c,
        &SubjectContext::default(),
        id(1),
        id(2)
    ));
    c.topology.reverse();
    assert!(!topology_allows(
        &c,
        &SubjectContext::default(),
        id(1),
        id(2)
    ));
}

#[test]
fn every_routing_rule_blocked_by_one_edge_is_reported_in_one_validation() {
    let mut c = constrained_route();
    c.topology[0].a = format!("node:{}", id(1));
    c.topology[0].b = format!("node:{}", id(2));
    c.topology[0].transports = Input::Value(vec![]);
    c.topology.push(TopologyRule {
        remark: String::new(),
        action: TopologyAction::Allow,
        a: "*".into(),
        b: "*".into(),
        transports: Input::Omitted,
        keepalive: false,
    });
    let route = |src: u8, dst: u8, via: u8| MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{}", id(src))],
        dst: vec![format!("node:{}", id(dst))],
        via: vec![format!("node:{}", id(via))],
    };
    c.routing.mesh = vec![route(1, 3, 2), route(3, 1, 2)];
    let ConfigurationError::Invalid(issues) = normalize(c, &context()).unwrap_err() else {
        panic!("expected configuration issue");
    };
    let mut rules: Vec<_> = issues
        .iter()
        .filter(|i| i.params.get("reason") == Some(&"edge_transport_unavailable".into()))
        .map(|i| i.params["routingRule"].as_str().to_owned())
        .collect();
    rules.sort();
    rules.dedup();
    assert_eq!(rules, ["/routing/mesh/0", "/routing/mesh/1"], "{issues:?}");
}

#[test]
fn every_blocked_edge_of_one_routed_path_is_reported() {
    let mut c = constrained_route();
    // One rule covers both edges of 1--2--3.
    c.topology[0].a = format!("node:{}", id(2));
    c.topology[0].transports = Input::Value(vec![]);
    let ConfigurationError::Invalid(issues) = normalize(c, &context()).unwrap_err() else {
        panic!("expected configuration issue");
    };
    let mut edges: Vec<_> = issues
        .iter()
        .filter(|i| i.params.get("reason") == Some(&"edge_transport_unavailable".into()))
        .map(|i| {
            (
                i.params["a"].clone(),
                i.params["b"].clone(),
                i.params["routingRule"].clone(),
            )
        })
        .collect();
    edges.sort();
    assert_eq!(
        edges,
        [
            (
                id(1).to_string(),
                id(2).to_string(),
                "/routing/mesh/0".to_owned()
            ),
            (
                id(2).to_string(),
                id(3).to_string(),
                "/routing/mesh/0".to_owned()
            ),
        ],
        "{issues:?}"
    );
}

#[test]
fn block_rule_options_are_accepted_with_a_warning() {
    let mut c = with_nodes();
    let rule = |transports, keepalive| TopologyRule {
        remark: String::new(),
        action: TopologyAction::Block,
        a: format!("node:{}", id(1)),
        b: format!("node:{}", id(2)),
        transports,
        keepalive,
    };
    c.topology = vec![
        rule(Input::Value(vec![TransportProtocol::NativeUdp]), false),
        rule(Input::Value(vec![]), true),
        rule(Input::Omitted, false),
    ];
    let normalized = normalize_with_dependencies(c, &context()).unwrap();
    let warnings: Vec<_> = normalized
        .warnings
        .iter()
        .map(|w| (w.code, w.path.as_str(), w.params["field"].as_str()))
        .collect();
    assert_eq!(
        warnings,
        [
            (
                IssueCode::IneffectiveBlockOption,
                "/topology/0/transports",
                "transports"
            ),
            (
                IssueCode::IneffectiveBlockOption,
                "/topology/1/transports",
                "transports"
            ),
            (
                IssueCode::IneffectiveBlockOption,
                "/topology/1/keepalive",
                "keepalive"
            ),
        ]
    );
}
