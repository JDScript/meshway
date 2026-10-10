//! Shared pure configuration generators; no execution issuer or fake installation.
use meshway_configuration::{
    AddressPools, Input, NetworkConfiguration, NodeAddresses, NodeIdentity, NodeSettings,
    SubjectContext,
    execution::TrustedNode,
    netmap::CompiledNetmap,
    transport::{Listener, PreparedListener},
};
use meshway_model::{
    BoundNodeKey, KeyAlgorithm, NetworkId, NodeId, NodePublicKey, Revision, Version,
    state::{Capability, StreamKind, TransportRole},
};
use meshway_policy::compile::netmap::{NetmapInput, PreviousNetmap, compile_netmap};
use meshway_protocol::NodeKeyMetadata;
use std::collections::BTreeMap;

/// The fixed Node ID number `i`.
pub fn node(i: u8) -> NodeId {
    format!("00000000-0000-4000-8000-{i:012}")
        .parse()
        .expect("fixture Node ID")
}

/// The `node:` selector of `id`.
pub fn sel(id: NodeId) -> String {
    format!("node:{id}")
}

/// An empty network on `100.64.0.0/24` with the owner-less `tag:server`.
pub fn network(domain: &str) -> NetworkConfiguration {
    let mut cfg = meshway_configuration::new_network(
        AddressPools {
            ipv4: vec!["100.64.0.0/24".into()],
            ipv6: vec![],
        },
        domain.into(),
    )
    .expect("fixture network");
    cfg.tags_owners.insert("tag:server".into(), vec![]);
    cfg
}

/// An enabled `tag:server` Node at `100.64.0.{index}`.
pub fn tag_node(
    hostname: &str,
    index: u8,
    listeners: BTreeMap<String, PreparedListener>,
    exit_enabled: bool,
) -> NodeSettings {
    NodeSettings {
        hostname: hostname.into(),
        remark: String::new(),
        identity: NodeIdentity::Tag {
            tags: vec!["tag:server".into()],
        },
        enabled: true,
        addresses: NodeAddresses {
            ipv4: Some(format!("100.64.0.{index}").parse().expect("fixture IPv4")),
            ipv6: None,
        },
        listeners,
        dialers: Input::Omitted,
        exit_enabled,
    }
}

/// A prepared listener from its JSON declaration.
pub fn listener(declaration: serde_json::Value) -> PreparedListener {
    let value: Listener = serde_json::from_value(declaration).expect("listener declaration");
    PreparedListener::prepare(value).expect("valid listener")
}

/// Native UDP dial and listen.
pub fn native_udp() -> (String, Capability) {
    (
        "transport:native_udp".into(),
        Capability::Transport {
            roles: vec![TransportRole::Dial, TransportRole::Listen],
            streams: vec![StreamKind::Udp],
        },
    )
}

/// The enrolled keys of a real binding.
pub fn trusted_keys(
    keys: &NodeKeyMetadata,
    capabilities: BTreeMap<String, Capability>,
) -> TrustedNode {
    TrustedNode {
        control_key: keys.control.clone(),
        dataplane_key: keys.dataplane.clone(),
        capabilities,
    }
}

/// Synthetic keys for peer `i` with Native UDP (and IPv4 exit with `exit`).
pub fn trusted(i: u8, exit: bool) -> TrustedNode {
    let key = |offset: u8, algorithm| BoundNodeKey {
        id: format!(
            "00000000-0000-4002-8000-{:012}",
            u16::from(i) + u16::from(offset)
        )
        .parse()
        .expect("fixture key ID"),
        generation: Version::MIN,
        public_key: NodePublicKey {
            algorithm,
            bytes: [i.wrapping_add(offset); 32].into(),
        },
    };
    let mut capabilities = BTreeMap::from([native_udp()]);
    if exit {
        capabilities.insert(
            "feature:exit-ipv4".into(),
            Capability::Feature { version: 1 },
        );
    }
    TrustedNode {
        control_key: key(0, KeyAlgorithm::Ed25519),
        dataplane_key: key(100, KeyAlgorithm::X25519),
        capabilities,
    }
}

/// Every Node's netmap at revision 1; `previous` continues serials.
pub fn compile(
    network: NetworkId,
    cfg: &NetworkConfiguration,
    inventory: &BTreeMap<NodeId, TrustedNode>,
    previous: &BTreeMap<NodeId, &CompiledNetmap>,
) -> BTreeMap<NodeId, CompiledNetmap> {
    let previous = previous
        .iter()
        .map(|(id, netmap)| (*id, PreviousNetmap::from(*netmap)))
        .collect();
    compile_netmap(NetmapInput {
        network,
        revision: Revision::MIN,
        configuration: cfg,
        subjects: &SubjectContext::default(),
        inventory,
        rule_sets: &BTreeMap::new(),
        selected_profiles: &BTreeMap::new(),
        selection_versions: &BTreeMap::new(),
        previous: &previous,
    })
    .expect("fixture netmap compilation")
    .nodes
}

/// `node`'s netmap; `previous` is its own prior netmap.
pub fn compile_one(
    network: NetworkId,
    cfg: &NetworkConfiguration,
    inventory: &BTreeMap<NodeId, TrustedNode>,
    node: NodeId,
    previous: Option<&CompiledNetmap>,
) -> CompiledNetmap {
    let previous = previous
        .map(|old| BTreeMap::from([(node, old)]))
        .unwrap_or_default();
    compile(network, cfg, inventory, &previous)
        .remove(&node)
        .expect("compiled Node")
}

/// One `tag:server` Node with synthetic keys.
pub fn single_node(hostname: &str, previous: Option<&CompiledNetmap>) -> CompiledNetmap {
    let node = node(1);
    let keys = TrustedNode {
        control_key: BoundNodeKey {
            id: "00000000-0000-4002-8000-000000000001".parse().unwrap(),
            generation: Version::MIN,
            public_key: NodePublicKey {
                algorithm: KeyAlgorithm::Ed25519,
                bytes: [4; 32].into(),
            },
        },
        dataplane_key: BoundNodeKey {
            id: "00000000-0000-4002-8000-000000000002".parse().unwrap(),
            generation: Version::MIN,
            public_key: NodePublicKey {
                algorithm: KeyAlgorithm::X25519,
                bytes: [5; 32].into(),
            },
        },
        capabilities: BTreeMap::new(),
    };
    let mut cfg = network("fixture.internal");
    cfg.nodes
        .insert(node, tag_node(hostname, 1, BTreeMap::new(), false));
    compile_one(
        "00000000-0000-4003-8000-000000000001".parse().unwrap(),
        &cfg,
        &BTreeMap::from([(node, keys)]),
        node,
        previous,
    )
}
