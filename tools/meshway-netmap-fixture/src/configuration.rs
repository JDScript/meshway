//! Shared pure configuration fixture; no execution issuer or fake installation.
use meshway_configuration::{
    AddressPools, Input, NodeAddresses, NodeIdentity, NodeSettings, SubjectContext,
    execution::TrustedNode, netmap::CompiledNetmap,
};
use meshway_model::{BoundNodeKey, KeyAlgorithm, NodeId, NodePublicKey, Revision, Version};
use meshway_policy::compile::netmap::{NetmapInput, PreviousNetmap, compile_netmap};
use std::collections::BTreeMap;

pub fn single_node(hostname: &str, previous: Option<&CompiledNetmap>) -> CompiledNetmap {
    let node: NodeId = "00000000-0000-4000-8000-000000000001".parse().unwrap();
    let key = |id: &str, algorithm, bytes: u8| BoundNodeKey {
        id: id.parse().unwrap(),
        generation: Version::MIN,
        public_key: NodePublicKey {
            algorithm,
            bytes: [bytes; 32].into(),
        },
    };
    let keys = TrustedNode {
        control_key: key(
            "00000000-0000-4002-8000-000000000001",
            KeyAlgorithm::Ed25519,
            4,
        ),
        dataplane_key: key(
            "00000000-0000-4002-8000-000000000002",
            KeyAlgorithm::X25519,
            5,
        ),
        capabilities: BTreeMap::new(),
    };
    let mut cfg = meshway_configuration::new_network(
        AddressPools {
            ipv4: vec!["100.64.0.0/24".into()],
            ipv6: vec![],
        },
        "fixture.internal".into(),
    )
    .unwrap();
    cfg.tags_owners.insert("tag:server".into(), vec![]);
    cfg.nodes.insert(
        node,
        NodeSettings {
            hostname: hostname.into(),
            remark: String::new(),
            identity: NodeIdentity::Tag {
                tags: vec!["tag:server".into()],
            },
            enabled: true,
            addresses: NodeAddresses {
                ipv4: Some("100.64.0.1".parse().unwrap()),
                ipv6: None,
            },
            listeners: BTreeMap::new(),
            dialers: Input::Omitted,
            exit_enabled: false,
        },
    );
    compile_netmap(NetmapInput {
        network: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
        revision: Revision::MIN,
        configuration: &cfg,
        subjects: &SubjectContext::default(),
        inventory: &BTreeMap::from([(node, keys)]),
        rule_sets: &BTreeMap::new(),
        selected_profiles: &BTreeMap::new(),
        selection_versions: &BTreeMap::new(),
        previous: &previous
            .map(|old| BTreeMap::from([(node, PreviousNetmap::from(old))]))
            .unwrap_or_default(),
    })
    .unwrap()
    .nodes
    .remove(&node)
    .unwrap()
}
