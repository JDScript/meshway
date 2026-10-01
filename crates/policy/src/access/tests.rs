use super::*;
use meshway_protocol::*;

fn configuration() -> NodeConfiguration {
    let mut nodes = (0..9)
        .map(|index| MeshNode {
            id: ((b'a' + index) as char).to_string(),
            name: "node".into(),
            country: Some(if index == 0 { "us" } else { "DE" }.into()),
            mesh_ipv4: format!("100.64.0.{}", index + 1),
            mesh_ipv6: format!("fd42::{}", index + 1),
        })
        .collect::<Vec<_>>();
    NodeConfiguration {
        schema_version: 1,
        revision: 7,
        network: MeshNetwork {
            ipv4_prefix: "100.64.0.0/24".into(),
            ipv6_prefix: "fd42::/64".into(),
        },
        node: nodes.remove(0),
        peers: nodes,
        routing_policy: RoutingPolicy::default(),
        policy_valid: true,
        diagnostics: vec![],
    }
}

fn access(paths: &[&[&str]]) -> MeshAccessPolicy {
    MeshAccessPolicy {
        grants: paths
            .iter()
            .enumerate()
            .map(|(index, path)| MeshAccessGrant {
                id: format!("grant-{index}"),
                path: path.iter().map(|node| (*node).to_owned()).collect(),
            })
            .collect(),
    }
}

fn route(id: &str, source: &str, destination: &str, via: &[&str]) -> RouteRule {
    RouteRule {
        id: id.into(),
        source: SourceSelector {
            node_ids: vec![source.into()],
            countries: vec![],
        },
        destination_scope: DestinationScope::Mesh,
        destination_any: vec![DestinationSelector {
            kind: DestinationSelectorKind::Node,
            value: destination.into(),
        }],
        action: RouteAction {
            kind: RouteActionKind::Route,
            via: via.iter().map(|id| (*id).into()).collect(),
            exit_node_id: None,
        },
    }
}

#[test]
fn explicit_routes_and_grants_intersect_without_reverse_shortcuts_or_subpaths() {
    let mut config = configuration();
    config.routing_policy.rules = vec![
        route("forward", "a", "c", &["b"]),
        route("return", "c", "a", &["b"]),
    ];
    let grants = access(&[&["a", "b", "c"], &["a", "c"], &["a", "b"], &["b", "c"]]);
    assert_eq!(
        compile_mesh_paths(&config, &grants).unwrap(),
        vec![vec!["a", "b", "c"]]
    );
    assert!(
        compile_mesh_paths(&config, &MeshAccessPolicy::default())
            .unwrap()
            .is_empty()
    );
    let both = access(&[&["a", "b", "c"], &["c", "b", "a"]]);
    assert_eq!(compile_mesh_paths(&config, &both).unwrap().len(), 2);
    config.routing_policy.rules.pop();
    assert_eq!(
        compile_mesh_paths(&config, &both).unwrap(),
        vec![vec!["a", "b", "c"]]
    );
    config.routing_policy.default_action = DefaultRouteAction::Direct;
    assert_eq!(
        compile_mesh_paths(&config, &access(&[&["c", "a"]])).unwrap(),
        vec![vec!["c", "a"]]
    );
}

#[test]
fn default_direct_requires_an_exact_directional_grant_and_never_skips_mandatory_hops() {
    let mut config = configuration();
    let forward = access(&[&["a", "c"]]);
    assert!(compile_mesh_paths(&config, &forward).unwrap().is_empty());

    config.routing_policy.default_action = DefaultRouteAction::Direct;
    assert_eq!(
        compile_mesh_paths(&config, &forward).unwrap(),
        vec![vec!["a", "c"]]
    );
    assert!(
        compile_mesh_paths(&config, &MeshAccessPolicy::default())
            .unwrap()
            .is_empty()
    );
    assert!(
        compile_mesh_paths(&config, &access(&[&["a", "b", "c"]]))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        compile_mesh_paths(&config, &access(&[&["c", "a"]])).unwrap(),
        vec![vec!["c", "a"]]
    );
    assert_eq!(
        compile_mesh_paths(&config, &access(&[&["a", "c"], &["c", "a"]])).unwrap(),
        vec![vec!["a", "c"], vec!["c", "a"]]
    );
}

#[test]
fn unknown_domain_before_default_direct_denies_only_the_potentially_affected_direction() {
    let mut config = configuration();
    config.routing_policy.default_action = DefaultRouteAction::Direct;
    let grants = access(&[&["a", "c"], &["c", "a"]]);
    let mut domain = route("domain-via", "a", "c", &["b"]);
    domain.destination_any = vec![DestinationSelector {
        kind: DestinationSelectorKind::DomainSuffix,
        value: "example.com".into(),
    }];
    config.routing_policy.rules = vec![domain.clone()];
    assert_eq!(
        compile_mesh_paths(&config, &grants).unwrap(),
        vec![vec!["c", "a"]]
    );
    domain.action.kind = RouteActionKind::Reject;
    domain.action.via.clear();
    config.routing_policy.rules = vec![domain];
    assert_eq!(
        compile_mesh_paths(&config, &grants).unwrap(),
        vec![vec!["c", "a"]]
    );
    config.routing_policy.rules[0].destination_scope = DestinationScope::Internet;
    assert_eq!(
        compile_mesh_paths(&config, &grants).unwrap(),
        vec![vec!["a", "c"], vec!["c", "a"]]
    );
}

#[test]
fn first_match_source_country_node_cidr_and_destination_country_keep_original_semantics() {
    let grants = access(&[&["a", "b", "c"]]);
    for (kind, value) in [
        (DestinationSelectorKind::Node, "c"),
        (DestinationSelectorKind::Cidr, "100.64.0.3/32"),
        (DestinationSelectorKind::Country, "de"),
    ] {
        let mut config = configuration();
        let mut allow = route("allow", "a", "c", &["b"]);
        allow.source.countries = vec!["US".into()];
        allow.destination_any = vec![DestinationSelector {
            kind,
            value: value.into(),
        }];
        config.routing_policy.rules = vec![allow.clone()];
        assert_eq!(compile_mesh_paths(&config, &grants).unwrap().len(), 1);
        config.node.country = Some("JP".into());
        assert!(compile_mesh_paths(&config, &grants).unwrap().is_empty());
        config.node.country = Some("US".into());
        let mut reject = allow.clone();
        reject.id = "reject-first".into();
        reject.action.kind = RouteActionKind::Reject;
        reject.action.via.clear();
        config.routing_policy.rules.insert(0, reject);
        assert!(compile_mesh_paths(&config, &grants).unwrap().is_empty());
        config.routing_policy.rules.reverse();
        assert_eq!(compile_mesh_paths(&config, &grants).unwrap().len(), 1);
        config.routing_policy.rules[0].action.via = vec!["d".into()];
        assert!(compile_mesh_paths(&config, &grants).unwrap().is_empty());
    }
}

#[test]
fn unknown_domain_cannot_skip_earlier_reject_or_via_but_irrelevant_rules_do_not_block() {
    let grants = access(&[&["a", "b", "c"]]);
    let mut config = configuration();
    let allow = route("allow", "a", "c", &["b"]);
    let mut domain = route("domain", "a", "c", &["d"]);
    domain.destination_any = vec![DestinationSelector {
        kind: DestinationSelectorKind::DomainSuffix,
        value: "example.com".into(),
    }];
    config.routing_policy.rules = vec![domain.clone(), allow.clone()];
    assert!(compile_mesh_paths(&config, &grants).unwrap().is_empty());
    domain.action.kind = RouteActionKind::Reject;
    domain.action.via.clear();
    config.routing_policy.rules[0] = domain.clone();
    assert!(compile_mesh_paths(&config, &grants).unwrap().is_empty());
    config.routing_policy.rules = vec![allow.clone(), domain.clone()];
    assert_eq!(compile_mesh_paths(&config, &grants).unwrap().len(), 1);
    config.routing_policy.rules = vec![domain.clone(), allow.clone()];
    config.routing_policy.rules[0].destination_scope = DestinationScope::Internet;
    assert_eq!(compile_mesh_paths(&config, &grants).unwrap().len(), 1);
    config.routing_policy.rules[0] = domain.clone();
    config.routing_policy.rules[0].source.node_ids = vec!["b".into()];
    assert_eq!(compile_mesh_paths(&config, &grants).unwrap().len(), 1);
    config.routing_policy.rules[0] = domain;
    config.routing_policy.rules[0].source.countries = vec!["JP".into()];
    assert_eq!(compile_mesh_paths(&config, &grants).unwrap().len(), 1);
    // OR with a trusted matching node means this rule's result is already known.
    config.routing_policy.rules[0] = allow;
    config.routing_policy.rules[0].id = "known-or-domain".into();
    config.routing_policy.rules[0]
        .destination_any
        .push(DestinationSelector {
            kind: DestinationSelectorKind::DomainSuffix,
            value: "example.com".into(),
        });
    assert_eq!(compile_mesh_paths(&config, &grants).unwrap().len(), 1);
}

#[test]
fn access_bounds_and_invalid_inventory_or_source_fail_closed() {
    let mut config = configuration();
    config
        .routing_policy
        .rules
        .push(route("allow", "a", "c", &["b"]));
    let good = access(&[&["a", "b", "c"]]);
    for invalid in [
        access(&[&[]]),
        access(&[&["a"]]),
        access(&[&["a", "a"]]),
        access(&[&["a", "missing"]]),
        access(&[&["a", "b"], &["a", "b"]]),
        access(&[&["a", "b", "c", "d", "e", "f", "g", "h", "i"]]),
    ] {
        assert!(compile_mesh_paths(&config, &invalid).is_err());
    }
    let all_nodes: Vec<_> = std::iter::once(config.node.clone())
        .chain(config.peers.clone())
        .collect();
    for id in [
        "".to_owned(),
        "é".into(),
        "a".repeat(65),
        "space id".into(),
        "a\n".into(),
    ] {
        let mut invalid = good.clone();
        invalid.grants[0].id = id;
        let diagnostics = validate_mesh_access(&invalid, &all_nodes);
        assert!(!diagnostics.is_empty());
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule_id.is_none())
        );
    }
    let mut invalid = access(&[&["a", "b"], &["b", "a"]]);
    invalid.grants[1].id = invalid.grants[0].id.clone();
    assert!(!validate_mesh_access(&invalid, &all_nodes).is_empty());
    let invalid = MeshAccessPolicy {
        grants: (0..129)
            .map(|i| MeshAccessGrant {
                id: format!("g{i}"),
                path: vec!["a".into(), "b".into()],
            })
            .collect(),
    };
    assert!(
        validate_mesh_access(&invalid, &all_nodes)
            .iter()
            .any(|d| d.field == "grants")
    );
    let mut invalid = good.clone();
    invalid.grants[0].path[0] = "a".repeat(MAX_MESH_ACCESS_BYTES);
    assert!(
        validate_mesh_access(&invalid, &all_nodes)
            .iter()
            .any(|d| d.field == "policy")
    );
    config.policy_valid = false;
    assert!(compile_mesh_paths(&config, &good).is_err());
    config.policy_valid = true;
    config.peers[0].mesh_ipv4 = config.node.mesh_ipv4.clone();
    assert!(compile_mesh_paths(&config, &good).is_err());
    config.peers[0].mesh_ipv4 = "8.8.8.8".into();
    assert!(compile_mesh_paths(&config, &good).is_err());
}

#[test]
fn invalid_grant_ids_cannot_amplify_diagnostic_bytes_within_the_source_budget() {
    let policy = MeshAccessPolicy {
        grants: vec![MeshAccessGrant {
            id: "x".repeat(120_000),
            path: vec!["missing".into(); 62],
        }],
    };
    assert!(serde_json::to_vec(&policy).unwrap().len() <= MAX_MESH_ACCESS_BYTES);
    let diagnostics = validate_mesh_access(&policy, &[]);
    assert_eq!(diagnostics.len(), crate::MAX_DIAGNOSTICS);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.rule_id.is_none())
    );
    assert!(serde_json::to_vec(&diagnostics).unwrap().len() < 16 * 1024);
}
