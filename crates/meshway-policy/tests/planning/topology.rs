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
        keepalive: false,
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
