use super::*;

pub(super) fn node(i: u8) -> NodeId {
    format!("00000000-0000-4000-8000-{i:012}").parse().unwrap()
}
pub(super) fn request(i: u8) -> RequestId {
    format!("00000000-0000-4001-8000-{i:012}").parse().unwrap()
}
pub(super) fn configuration() -> NetworkConfiguration {
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
                listeners: BTreeMap::from([("udp".into(), PreparedListener::prepare(l).unwrap())]),
                dialers: Input::Omitted,
                exit_enabled: i == 4,
            },
        );
    }
    c
}
pub(super) fn inventory() -> BTreeMap<NodeId, TrustedNode> {
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
pub(super) fn demand(i: u8, source: u8, target: u8) -> Demand {
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

pub(super) fn run(
    c: &NetworkConfiguration,
    inv: &BTreeMap<NodeId, TrustedNode>,
    demands: &BTreeMap<RequestId, Demand>,
    dns: &[TrustedDnsAssociation],
) -> CompiledPublication {
    run_with_subjects(c, inv, demands, dns, &SubjectContext::default())
}
pub(super) fn run_with_subjects(
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
