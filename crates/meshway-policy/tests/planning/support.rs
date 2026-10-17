use super::*;

pub(super) fn node(i: u8) -> NodeId {
    format!("00000000-0000-4000-8000-{i:012}").parse().unwrap()
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
                exit_families: default_exit_families(),
                region: None,
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
                    capabilities: BTreeMap::from([
                        (
                            "feature:exit-ipv4".into(),
                            Capability::Feature { version: 1 },
                        ),
                        (
                            "transport:native_udp".into(),
                            Capability::Transport {
                                roles: vec![TransportRole::Dial, TransportRole::Listen],
                                streams: vec![StreamKind::Udp],
                            },
                        ),
                    ]),
                },
            )
        })
        .collect()
}

/// One in-memory set, as the Server stores inline and converted content.
pub(super) fn set(
    domain: &[&str],
    suffix: &[&str],
    cidr: &[&str],
) -> std::sync::Arc<meshway_rules::RuleSet> {
    let own = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let (domain, suffix, cidr) = (own(domain), own(suffix), own(cidr));
    std::sync::Arc::new(
        meshway_rules::RuleSet::from_inline(&meshway_rules::inline::Inline {
            domain: &domain,
            domain_suffix: &suffix,
            domain_subdomain: &[],
            ip_cidr: &cidr,
        })
        .unwrap(),
    )
}

/// Loaded sets of the configuration's inline definitions.
pub(super) fn inline_sets(c: &NetworkConfiguration) -> RuleSets {
    c.rule_sets
        .iter()
        .filter_map(|(key, definition)| match definition {
            RuleSet::Inline {
                ip_cidr,
                domain_suffix,
                domain,
                domain_subdomain,
            } => Some((
                key.clone(),
                std::sync::Arc::new(
                    meshway_rules::RuleSet::from_inline(&meshway_rules::inline::Inline {
                        domain,
                        domain_suffix,
                        domain_subdomain,
                        ip_cidr,
                    })
                    .unwrap(),
                ),
            )),
            RuleSet::Remote { .. } => None,
        })
        .collect()
}
