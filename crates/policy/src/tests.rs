use super::*;
use meshway_protocol::routing::SourceSelector;

fn network() -> MeshNetwork {
    MeshNetwork {
        ipv4_prefix: "100.64.0.0/24".into(),
        ipv6_prefix: "fd42::/120".into(),
    }
}

fn nodes() -> Vec<MeshNode> {
    [
        ("a", Some("us")),
        ("b", Some("DE")),
        ("c", Some("JP")),
        ("d", None),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (id, country))| MeshNode {
        id: id.into(),
        name: format!("Node {id}"),
        country: country.map(str::to_owned),
        mesh_ipv4: format!("100.64.0.{}", index + 1),
        mesh_ipv6: format!("fd42::{}", index + 1),
    })
    .collect()
}

fn flow() -> FlowQuery {
    FlowQuery {
        source_node_id: "a".into(),
        destination_node_id: None,
        destination_ip: None,
        domain: Some("example.com".into()),
        destination_country: None,
    }
}

fn selector(kind: DestinationSelectorKind, value: &str) -> DestinationSelector {
    DestinationSelector {
        kind,
        value: value.into(),
    }
}

fn rule() -> RouteRule {
    RouteRule {
        id: "rule-1".into(),
        source: SourceSelector::default(),
        destination_scope: DestinationScope::Any,
        destination_any: vec![],
        action: RouteAction {
            kind: RouteActionKind::Route,
            via: vec![],
            exit_node_id: None,
        },
    }
}

fn policy(rule: RouteRule) -> RoutingPolicy {
    RoutingPolicy {
        default_action: DefaultRouteAction::Reject,
        rules: vec![rule],
    }
}

fn check(policy: &RoutingPolicy, query: &FlowQuery) -> PreviewResult {
    preview(policy, &network(), &nodes(), query)
}

fn assert_invalid(result: &PreviewResult, field: &str) {
    assert!(!result.valid, "{result:?}");
    assert_eq!(result.decision.action, RouteDecisionAction::Invalid);
    assert!(result.decision.path.is_empty());
    assert!(result.decision.exit_node_id.is_none());
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.field == field),
        "{result:?}"
    );
}

#[test]
fn suffixes_normalize_idna_case_terminal_dot_and_respect_label_boundaries() {
    for (suffix, query, expected) in [
        ("EXAMPLE.COM.", "example.com", true),
        ("example.com", "Api.Example.Com.", true),
        ("example.com", "badexample.com", false),
        ("example.com", "example.com.evil", false),
        ("example.com", "example.net", false),
        ("BÜCHER.example.", "shop.xn--bcher-kva.example", true),
        ("xn--bcher-kva.example", "BÜCHER.example。", true),
        ("bücher.example", "badbücher.example", false),
    ] {
        let mut rule = rule();
        rule.destination_any = vec![selector(DestinationSelectorKind::DomainSuffix, suffix)];
        let mut query_flow = flow();
        query_flow.domain = Some(query.into());
        let result = check(&policy(rule), &query_flow);
        assert!(result.valid, "{suffix} / {query}: {result:?}");
        assert_eq!(
            result.decision.action == RouteDecisionAction::Route,
            expected,
            "{suffix} / {query}"
        );
    }
}

#[test]
fn invalid_domain_names_are_diagnostics_in_rules_and_flows() {
    for value in [
        "",
        ".",
        "example..com",
        "example.com..",
        "*.example.com",
        ".example.com",
        "https://example.com",
        "has space.example",
        "-bad.example",
        "bad_.example",
        "127.0.0.1",
        "[::1]",
    ] {
        let mut rule = rule();
        rule.destination_any = vec![selector(DestinationSelectorKind::DomainSuffix, value)];
        let report = compile(&policy(rule), &network(), &nodes());
        assert!(!report.is_valid(), "accepted domain {value:?}");
        let mut query = flow();
        query.domain = Some(value.into());
        assert_invalid(&check(&RoutingPolicy::default(), &query), "flow.domain");
    }
    let mut query = flow();
    query.domain = Some(format!("{}.example", "x".repeat(64)));
    assert_invalid(&check(&RoutingPolicy::default(), &query), "flow.domain");
}

#[test]
fn normalization_is_deterministic_idempotent_and_does_not_change_inputs_or_order() {
    let mut first = rule();
    first.source.countries = vec!["us".into(), "jP".into()];
    first.destination_scope = DestinationScope::Internet;
    first.destination_any = vec![
        selector(DestinationSelectorKind::DomainSuffix, "BÜCHER.Example."),
        selector(DestinationSelectorKind::Cidr, "192.0.2.99/24"),
        selector(DestinationSelectorKind::Cidr, "2001:DB8:1234::abcd/48"),
        selector(DestinationSelectorKind::Country, "de"),
    ];
    first.action.via = vec!["c".into(), "b".into()];
    first.action.exit_node_id = Some("d".into());
    let mut second = rule();
    second.id = "rule-2".into();
    let original = RoutingPolicy {
        rules: vec![first, second],
        ..RoutingPolicy::default()
    };
    let saved = original.clone();
    let inventory = nodes();
    let saved_inventory = inventory.clone();
    let report = compile(&original, &network(), &inventory);
    assert!(report.is_valid(), "{report:?}");
    assert_eq!(original, saved);
    assert_eq!(inventory, saved_inventory);
    assert_eq!(report, compile(&original, &network(), &inventory));
    assert_eq!(
        report,
        compile(&report.normalized_policy, &network(), &inventory)
    );
    let first = &report.normalized_policy.rules[0];
    assert_eq!(first.id, "rule-1");
    assert_eq!(report.normalized_policy.rules[1].id, "rule-2");
    assert_eq!(first.source.countries, ["US", "JP"]);
    assert_eq!(first.action.via, ["c", "b"]);
    assert_eq!(
        first
            .destination_any
            .iter()
            .map(|s| s.value.as_str())
            .collect::<Vec<_>>(),
        [
            "xn--bcher-kva.example",
            "192.0.2.0/24",
            "2001:db8:1234::/48",
            "DE"
        ]
    );
}

#[test]
fn sources_or_within_dimensions_and_across_dimensions_use_registered_country() {
    let mut rule = rule();
    rule.source.node_ids = vec!["a".into(), "b".into()];
    rule.source.countries = vec!["US".into(), "jp".into()];
    let policy = policy(rule);
    for (source, expected) in [("a", true), ("b", false), ("c", false), ("d", false)] {
        let mut query = flow();
        query.source_node_id = source.into();
        // An Internet destination hint must never substitute for source metadata.
        query.destination_country = Some("US".into());
        let result = check(&policy, &query);
        assert!(result.valid);
        assert_eq!(
            result.decision.action == RouteDecisionAction::Route,
            expected,
            "{source}"
        );
    }
}

#[test]
fn empty_source_dimensions_are_wildcards_including_unknown_countries() {
    let mut rule = rule();
    rule.source.node_ids = vec!["d".into()];
    let mut query = flow();
    query.source_node_id = "d".into();
    assert_eq!(
        check(&policy(rule.clone()), &query).decision.action,
        RouteDecisionAction::Route
    );
    rule.source.node_ids.clear();
    rule.source.countries = vec!["JP".into(), "DE".into()];
    assert_eq!(
        check(&policy(rule.clone()), &query).decision.action,
        RouteDecisionAction::Reject
    );
    for source in ["b", "c"] {
        query.source_node_id = source.into();
        assert_eq!(
            check(&policy(rule.clone()), &query).decision.action,
            RouteDecisionAction::Route
        );
    }
}

#[test]
fn destination_selectors_are_or_but_scope_is_a_separate_restriction() {
    let mut rule = rule();
    rule.destination_scope = DestinationScope::Internet;
    rule.destination_any = vec![
        selector(DestinationSelectorKind::DomainSuffix, "example.com"),
        selector(DestinationSelectorKind::Cidr, "192.0.2.0/24"),
        selector(DestinationSelectorKind::Country, "JP"),
    ];
    for (domain, ip, country, expected) in [
        (Some("example.com"), None, None, true),
        (None, Some("192.0.2.3"), None, true),
        (Some("other.example"), None, Some("jp"), true),
        (Some("other.example"), None, Some("DE"), false),
        (Some("example.com"), Some("100.64.0.2"), None, false),
    ] {
        let query = FlowQuery {
            domain: domain.map(str::to_owned),
            destination_ip: ip.map(str::to_owned),
            destination_country: country.map(str::to_owned),
            ..flow()
        };
        let result = check(&policy(rule.clone()), &query);
        assert!(result.valid, "{result:?}");
        assert_eq!(
            result.decision.action == RouteDecisionAction::Route,
            expected
        );
    }
}

#[test]
fn ipv4_and_ipv6_cidrs_match_with_canonical_host_bits_and_without_dns() {
    let mut rule = rule();
    rule.destination_any = vec![
        selector(DestinationSelectorKind::Cidr, "192.0.2.101/24"),
        selector(DestinationSelectorKind::Cidr, "2001:db8::feed/32"),
    ];
    for (ip, expected) in [
        (Some("192.0.2.1"), true),
        (Some("192.0.3.1"), false),
        (Some("2001:db8:abcd::1"), true),
        (Some("2001:db9::1"), false),
        (None, false),
    ] {
        let query = FlowQuery {
            destination_ip: ip.map(str::to_owned),
            ..flow()
        };
        assert_eq!(
            check(&policy(rule.clone()), &query).decision.action == RouteDecisionAction::Route,
            expected
        );
    }
}

#[test]
fn first_match_is_authoritative_for_reject_and_ordered_route() {
    let mut route = rule();
    route.destination_scope = DestinationScope::Internet;
    route.action.via = vec!["c".into(), "b".into()];
    route.action.exit_node_id = Some("d".into());
    let mut reject = rule();
    reject.id = "reject".into();
    reject.action.kind = RouteActionKind::Reject;
    let mut policy = RoutingPolicy {
        rules: vec![reject, route],
        ..RoutingPolicy::default()
    };
    let denied = check(&policy, &flow());
    assert!(denied.valid);
    assert_eq!(denied.decision.action, RouteDecisionAction::Reject);
    assert_eq!(denied.decision.rule_id.as_deref(), Some("reject"));
    assert!(denied.decision.path.is_empty());
    policy.rules.swap(0, 1);
    let routed = check(&policy, &flow());
    assert!(routed.valid);
    assert_eq!(routed.decision.rule_id.as_deref(), Some("rule-1"));
    assert_eq!(routed.decision.path, ["a", "c", "b", "d"]);
    assert_eq!(routed.decision.exit_node_id.as_deref(), Some("d"));
}

#[test]
fn empty_policy_rejects_and_configured_direct_handles_mesh_and_internet() {
    let mut policy = RoutingPolicy::default();
    let denied = check(&policy, &flow());
    assert!(denied.valid);
    assert_eq!(denied.decision.action, RouteDecisionAction::Reject);
    assert!(denied.decision.path.is_empty());
    policy.default_action = DefaultRouteAction::Direct;
    let direct = check(&policy, &flow());
    assert!(direct.valid);
    assert_eq!(direct.decision.path, ["a"]);
    assert_eq!(direct.decision.exit_node_id.as_deref(), Some("a"));
    assert!(direct.decision.rule_id.is_none());
    let query = FlowQuery {
        destination_node_id: Some("b".into()),
        domain: None,
        ..flow()
    };
    let direct = check(&policy, &query);
    assert!(direct.valid);
    assert_eq!(direct.decision.path, ["a", "b"]);
    assert!(direct.decision.exit_node_id.is_none());
}

#[test]
fn mesh_routes_preserve_mandatory_vias_and_have_no_exit() {
    let mut rule = rule();
    rule.destination_scope = DestinationScope::Mesh;
    rule.destination_any = vec![selector(DestinationSelectorKind::Node, "b")];
    rule.action.via = vec!["d".into(), "c".into()];
    let query = FlowQuery {
        destination_node_id: Some("b".into()),
        domain: None,
        ..flow()
    };
    let result = check(&policy(rule), &query);
    assert!(result.valid, "{result:?}");
    assert_eq!(result.decision.path, ["a", "d", "c", "b"]);
    assert!(result.decision.exit_node_id.is_none());
}

#[test]
fn explicit_local_exit_is_valid_only_without_vias() {
    let mut rule = rule();
    rule.destination_scope = DestinationScope::Internet;
    rule.action.exit_node_id = Some("a".into());
    let result = check(&policy(rule.clone()), &flow());
    assert!(result.valid);
    assert_eq!(result.decision.path, ["a"]);
    assert_eq!(result.decision.exit_node_id.as_deref(), Some("a"));
    rule.action.via.push("b".into());
    assert!(compile(&policy(rule.clone()), &network(), &nodes()).is_valid());
    assert_invalid(&check(&policy(rule), &flow()), "flow.path");
}

#[test]
fn literal_mesh_addresses_resolve_owners_countries_and_node_selectors() {
    let mut rule = rule();
    rule.destination_scope = DestinationScope::Mesh;
    rule.destination_any = vec![selector(DestinationSelectorKind::Node, "b")];
    for ip in ["100.64.0.2", "fd42::2", "FD42:0:0:0:0:0:0:2"] {
        let query = FlowQuery {
            destination_ip: Some(ip.into()),
            domain: None,
            ..flow()
        };
        let result = check(&policy(rule.clone()), &query);
        assert!(result.valid, "{result:?}");
        assert_eq!(result.decision.path, ["a", "b"]);
        assert!(result.decision.exit_node_id.is_none());
        rule.destination_any = vec![selector(DestinationSelectorKind::Country, "de")];
        let result = check(&policy(rule.clone()), &query);
        assert_eq!(result.decision.path, ["a", "b"]);
        rule.destination_any = vec![selector(DestinationSelectorKind::Node, "b")];
    }
}

#[test]
fn explicit_node_derives_ipv4_but_accepts_its_assigned_ipv6() {
    let mut rule = rule();
    rule.destination_any = vec![selector(DestinationSelectorKind::Cidr, "100.64.0.2/32")];
    let mut query = FlowQuery {
        destination_node_id: Some("b".into()),
        domain: None,
        ..flow()
    };
    assert_eq!(
        check(&policy(rule.clone()), &query).decision.path,
        ["a", "b"]
    );
    query.destination_ip = Some("fd42::2".into());
    assert_eq!(
        check(&policy(rule.clone()), &query).decision.action,
        RouteDecisionAction::Reject
    );
    rule.destination_any = vec![selector(DestinationSelectorKind::Cidr, "fd42::2/128")];
    assert_eq!(check(&policy(rule), &query).decision.path, ["a", "b"]);
}

#[test]
fn inconsistent_mesh_context_and_inactive_nodes_fail_closed() {
    let policy = RoutingPolicy {
        default_action: DefaultRouteAction::Direct,
        rules: vec![],
    };
    for ip in ["192.0.2.1", "100.64.0.3", "fd42::3"] {
        let query = FlowQuery {
            destination_node_id: Some("b".into()),
            destination_ip: Some(ip.into()),
            ..flow()
        };
        assert_invalid(&check(&policy, &query), "flow.destination_ip");
    }
    for (node_id, country, expected) in [
        (Some("b"), "US", false),
        (None, "JP", false),
        (Some("b"), "de", true),
        (Some("d"), "DE", false),
    ] {
        let query = FlowQuery {
            destination_node_id: node_id.map(str::to_owned),
            destination_ip: if node_id.is_none() {
                Some("fd42::2".into())
            } else {
                None
            },
            destination_country: Some(country.into()),
            ..flow()
        };
        let result = check(&policy, &query);
        if expected {
            assert!(result.valid, "{result:?}");
        } else {
            assert_invalid(&result, "flow.destination_country");
        }
    }
    let missing_source = FlowQuery {
        source_node_id: "revoked".into(),
        ..flow()
    };
    assert_invalid(&check(&policy, &missing_source), "flow.source_node_id");
    let missing_target = FlowQuery {
        destination_node_id: Some("revoked".into()),
        ..flow()
    };
    assert_invalid(&check(&policy, &missing_target), "flow.destination_node_id");
}

#[test]
fn mapped_ipv6_literals_cannot_bypass_mesh_rejection_or_fall_through_to_direct() {
    let mut reject_mesh = rule();
    reject_mesh.destination_scope = DestinationScope::Mesh;
    reject_mesh.action.kind = RouteActionKind::Reject;
    let policy = RoutingPolicy {
        default_action: DefaultRouteAction::Direct,
        rules: vec![reject_mesh],
    };
    for (plain, expected, mapped_forms) in [
        (
            "100.64.0.2",
            RouteDecisionAction::Reject,
            [
                "::ffff:100.64.0.2",
                "::ffff:6440:2",
                "0:0:0:0:0:FFFF:6440:0002",
            ],
        ),
        (
            "100.64.0.99",
            RouteDecisionAction::Invalid,
            [
                "::ffff:100.64.0.99",
                "::ffff:6440:63",
                "0:0:0:0:0:FFFF:6440:0063",
            ],
        ),
        (
            "192.0.2.1",
            RouteDecisionAction::Route,
            [
                "::ffff:192.0.2.1",
                "::ffff:c000:201",
                "0:0:0:0:0:FFFF:C000:0201",
            ],
        ),
    ] {
        let mut query = FlowQuery {
            destination_ip: Some(plain.into()),
            domain: None,
            ..flow()
        };
        let native = check(&policy, &query);
        assert_eq!(native.decision.action, expected, "{plain}: {native:?}");
        assert_eq!(native.valid, expected != RouteDecisionAction::Invalid);
        if expected == RouteDecisionAction::Route {
            assert_eq!(native.decision.path, ["a"]);
            assert_eq!(native.decision.exit_node_id.as_deref(), Some("a"));
        }
        for mapped in mapped_forms {
            query.destination_ip = Some(mapped.into());
            let result = check(&policy, &query);
            assert_invalid(&result, "flow.destination_ip");
            assert!(result.decision.rule_id.is_none(), "{mapped}: {result:?}");
            assert!(result.decision.reason.contains("IPv4-mapped IPv6"));
            assert!(result.decision.reason.contains("unsupported"));
            assert_eq!(result.diagnostics.len(), 1);
        }
    }
}

#[test]
fn mapped_ipv6_with_explicit_node_has_the_same_unsupported_error() {
    let policy = RoutingPolicy {
        default_action: DefaultRouteAction::Direct,
        rules: vec![],
    };
    for mapped in [
        "::ffff:100.64.0.2",
        "::ffff:6440:2",
        "0:0:0:0:0:FFFF:6440:0002",
    ] {
        let mut query = FlowQuery {
            destination_ip: Some(mapped.into()),
            domain: None,
            ..flow()
        };
        let implicit = check(&policy, &query);
        query.destination_node_id = Some("b".into());
        let explicit = check(&policy, &query);
        assert_invalid(&explicit, "flow.destination_ip");
        assert!(explicit.decision.reason.contains("IPv4-mapped IPv6"));
        assert!(explicit.decision.reason.contains("unsupported"));
        assert_eq!(explicit, implicit);
    }
}

#[test]
fn unassigned_or_revoked_mesh_addresses_never_fall_back_to_internet() {
    let policy = RoutingPolicy {
        default_action: DefaultRouteAction::Direct,
        rules: vec![],
    };
    for ip in ["100.64.0.99", "fd42::99"] {
        let query = FlowQuery {
            destination_ip: Some(ip.into()),
            ..flow()
        };
        assert_invalid(&check(&policy, &query), "flow.destination_ip");
    }
    let active = nodes()
        .into_iter()
        .filter(|node| node.id != "b")
        .collect::<Vec<_>>();
    for ip in ["100.64.0.2", "fd42::2"] {
        let query = FlowQuery {
            destination_ip: Some(ip.into()),
            ..flow()
        };
        assert_invalid(
            &preview(&policy, &network(), &active, &query),
            "flow.destination_ip",
        );
    }
    for ip in ["100.64.1.99", "fd42::199", "192.0.2.1", "2001:db8::1"] {
        let query = FlowQuery {
            destination_ip: Some(ip.into()),
            ..flow()
        };
        let result = check(&policy, &query);
        assert!(result.valid, "{result:?}");
        assert_eq!(result.decision.path, ["a"]);
        assert_eq!(result.decision.exit_node_id.as_deref(), Some("a"));
    }
}

#[test]
fn flow_requires_a_real_target_and_well_formed_explicit_hints() {
    let policy = RoutingPolicy {
        default_action: DefaultRouteAction::Direct,
        rules: vec![],
    };
    for country in [None, Some("US")] {
        let query = FlowQuery {
            domain: None,
            destination_country: country.map(str::to_owned),
            ..flow()
        };
        assert_invalid(&check(&policy, &query), "flow.destination");
    }
    for ip in ["example.com", "192.0.2.1/24", "", "not-an-ip"] {
        let query = FlowQuery {
            destination_ip: Some(ip.into()),
            ..flow()
        };
        assert_invalid(&check(&policy, &query), "flow.destination_ip");
    }
    for country in ["USA", "1A", "éa", "", " US"] {
        let query = FlowQuery {
            destination_country: Some(country.into()),
            ..flow()
        };
        assert_invalid(&check(&policy, &query), "flow.destination_country");
    }
    let query = FlowQuery {
        destination_country: Some("jp".into()),
        ..flow()
    };
    assert!(check(&policy, &query).valid);
}

#[test]
fn resolved_cycles_fail_closed_without_skipping_hops_or_later_rules() {
    let query = FlowQuery {
        destination_node_id: Some("b".into()),
        ..flow()
    };
    for via in [vec!["a"], vec!["b"], vec!["c", "a"], vec!["c", "b"]] {
        let mut first = rule();
        first.destination_scope = DestinationScope::Mesh;
        first.action.via = via.into_iter().map(str::to_owned).collect();
        let mut later = rule();
        later.id = "later-direct".into();
        let policy = RoutingPolicy {
            default_action: DefaultRouteAction::Direct,
            rules: vec![first, later],
        };
        assert!(compile(&policy, &network(), &nodes()).is_valid());
        let result = check(&policy, &query);
        assert_invalid(&result, "flow.path");
        assert_eq!(result.decision.rule_id.as_deref(), Some("rule-1"));
    }
    let direct = RoutingPolicy {
        default_action: DefaultRouteAction::Direct,
        rules: vec![],
    };
    let self_query = FlowQuery {
        destination_node_id: Some("a".into()),
        ..flow()
    };
    assert_invalid(&check(&direct, &self_query), "flow.path");
}

#[test]
fn every_node_reference_is_checked_against_active_inventory() {
    for field in ["source", "destination", "via", "exit"] {
        let mut rule = rule();
        match field {
            "source" => rule.source.node_ids = vec!["revoked".into()],
            "destination" => {
                rule.destination_any = vec![selector(DestinationSelectorKind::Node, "revoked")]
            }
            "via" => {
                rule.destination_scope = DestinationScope::Mesh;
                rule.action.via = vec!["revoked".into()];
            }
            "exit" => {
                rule.destination_scope = DestinationScope::Internet;
                rule.action.exit_node_id = Some("revoked".into());
            }
            _ => unreachable!(),
        }
        let report = compile(&policy(rule), &network(), &nodes());
        assert!(!report.is_valid(), "{field}");
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("active inventory"))
        );
    }
}

#[test]
fn structural_action_conflicts_and_scope_ambiguity_are_invalid_policies() {
    for (scope, kind, via, exit, destination_node) in [
        (
            DestinationScope::Mesh,
            RouteActionKind::Route,
            vec!["c", "c"],
            None,
            None,
        ),
        (
            DestinationScope::Internet,
            RouteActionKind::Route,
            vec!["b"],
            Some("b"),
            None,
        ),
        (
            DestinationScope::Internet,
            RouteActionKind::Reject,
            vec!["b"],
            None,
            None,
        ),
        (
            DestinationScope::Internet,
            RouteActionKind::Reject,
            vec![],
            Some("b"),
            None,
        ),
        (
            DestinationScope::Any,
            RouteActionKind::Route,
            vec![],
            Some("b"),
            None,
        ),
        (
            DestinationScope::Mesh,
            RouteActionKind::Route,
            vec![],
            Some("b"),
            None,
        ),
        (
            DestinationScope::Internet,
            RouteActionKind::Route,
            vec!["b"],
            None,
            None,
        ),
        (
            DestinationScope::Any,
            RouteActionKind::Route,
            vec!["b"],
            None,
            None,
        ),
        (
            DestinationScope::Internet,
            RouteActionKind::Route,
            vec![],
            None,
            Some("b"),
        ),
    ] {
        let mut rule = rule();
        rule.destination_scope = scope;
        rule.action = RouteAction {
            kind,
            via: via.into_iter().map(str::to_owned).collect(),
            exit_node_id: exit.map(str::to_owned),
        };
        if let Some(id) = destination_node {
            rule.destination_any
                .push(selector(DestinationSelectorKind::Node, id));
        }
        let result = check(&policy(rule), &flow());
        assert!(!result.valid, "{result:?}");
        assert_eq!(result.decision.action, RouteDecisionAction::Invalid);
        assert!(result.decision.path.is_empty());
    }
}

#[test]
fn malformed_countries_prefixes_and_rule_ids_have_explicit_diagnostics() {
    for (kind, value) in [
        (DestinationSelectorKind::Country, "USA"),
        (DestinationSelectorKind::Country, "éa"),
        (DestinationSelectorKind::Cidr, "192.0.2.1"),
        (DestinationSelectorKind::Cidr, "192.0.2.0/33"),
        (DestinationSelectorKind::Cidr, "2001:db8::/129"),
        (DestinationSelectorKind::Cidr, "example.com/24"),
    ] {
        let mut rule = rule();
        rule.destination_any.push(selector(kind, value));
        let report = compile(&policy(rule), &network(), &nodes());
        assert!(!report.is_valid(), "{kind:?}: {value}");
        assert_eq!(report.diagnostics[0].field, "destination_any[0].value");
    }
    let mut invalid = rule();
    invalid.source.countries.push("us ".into());
    invalid.id = " ".into();
    let report = compile(&policy(invalid), &network(), &nodes());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.field == "id")
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.field == "source.countries[0]")
    );
    let duplicate = RoutingPolicy {
        rules: vec![rule(), rule()],
        ..RoutingPolicy::default()
    };
    let report = compile(&duplicate, &network(), &nodes());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unique"))
    );
}

#[test]
fn inventory_rejects_duplicate_ids_ips_wrong_families_pools_and_bad_countries() {
    for mutation in 0..8 {
        let mut inventory = nodes();
        match mutation {
            0 => inventory[1].id = inventory[0].id.clone(),
            1 => inventory[1].mesh_ipv4 = inventory[0].mesh_ipv4.clone(),
            2 => inventory[1].mesh_ipv6 = "FD42:0:0:0:0:0:0:1".into(),
            3 => inventory[1].mesh_ipv4 = "fd42::2".into(),
            4 => inventory[1].mesh_ipv6 = "100.64.0.2".into(),
            5 => inventory[1].mesh_ipv4 = "100.64.1.2".into(),
            6 => inventory[1].mesh_ipv6 = "fd43::2".into(),
            7 => inventory[1].country = Some("not-a-country".into()),
            _ => unreachable!(),
        }
        let report = compile(&RoutingPolicy::default(), &network(), &inventory);
        assert!(!report.is_valid(), "mutation {mutation}");
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.field.starts_with("nodes["))
        );
        assert_eq!(
            report,
            compile(&RoutingPolicy::default(), &network(), &inventory)
        );
    }
    for (v4, v6) in [
        ("bad", "fd42::/120"),
        ("fd42::/120", "fd42::/120"),
        ("100.64.0.0/24", "bad"),
        ("100.64.0.0/24", "100.64.0.0/24"),
    ] {
        let config = MeshNetwork {
            ipv4_prefix: v4.into(),
            ipv6_prefix: v6.into(),
        };
        assert!(!compile(&RoutingPolicy::default(), &config, &nodes()).is_valid());
    }
}

#[test]
fn rule_selector_hop_and_diagnostic_limits_are_enforced_at_boundaries() {
    let mut many_rules = RoutingPolicy {
        rules: (0..MAX_RULES)
            .map(|index| RouteRule {
                id: format!("rule-{index}"),
                ..rule()
            })
            .collect(),
        ..RoutingPolicy::default()
    };
    assert!(compile(&many_rules, &network(), &nodes()).is_valid());
    many_rules.rules.push(RouteRule {
        id: "extra".into(),
        ..rule()
    });
    assert!(!compile(&many_rules, &network(), &nodes()).is_valid());

    let mut selection = rule();
    selection.source.node_ids = vec!["a".into(); 10];
    selection.source.countries = vec!["US".into(); 10];
    selection.destination_any = vec![selector(DestinationSelectorKind::Country, "DE"); 12];
    assert!(compile(&policy(selection.clone()), &network(), &nodes()).is_valid());
    selection
        .destination_any
        .push(selector(DestinationSelectorKind::Country, "JP"));
    assert!(!compile(&policy(selection), &network(), &nodes()).is_valid());

    let inventory: Vec<_> = (1..=11)
        .map(|index| MeshNode {
            id: format!("n{index}"),
            name: format!("Node {index}"),
            country: None,
            mesh_ipv4: format!("100.64.0.{index}"),
            mesh_ipv6: format!("fd42::{index:x}"),
        })
        .collect();
    let mut route = rule();
    route.destination_scope = DestinationScope::Mesh;
    route.action.via = (2..=9).map(|index| format!("n{index}")).collect();
    let query = FlowQuery {
        source_node_id: "n1".into(),
        destination_node_id: Some("n11".into()),
        ..flow()
    };
    let result = preview(&policy(route.clone()), &network(), &inventory, &query);
    assert!(result.valid, "{result:?}");
    assert_eq!(result.decision.path.len(), MAX_VIA + 2);
    route.action.via.push("n10".into());
    assert!(!compile(&policy(route), &network(), &inventory).is_valid());

    for rule in &mut many_rules.rules {
        rule.source.node_ids = vec!["missing".into()];
    }
    let report = compile(&many_rules, &network(), &nodes());
    assert_eq!(report.diagnostics.len(), MAX_DIAGNOSTICS);
}

#[test]
fn serialized_policy_size_is_bounded_in_bytes() {
    let mut policy = policy(rule());
    let base_size = serde_json::to_vec(&policy).unwrap().len();
    let extra = MAX_POLICY_BYTES - base_size;
    policy.rules[0].id.push_str(&"x".repeat(extra));
    assert_eq!(serde_json::to_vec(&policy).unwrap().len(), MAX_POLICY_BYTES);
    assert!(compile(&policy, &network(), &nodes()).is_valid());
    policy.rules[0].id.push('x');
    let report = compile(&policy, &network(), &nodes());
    assert!(!report.is_valid());
    assert_eq!(report.diagnostics[0].field, "policy");
    // Escaping and UTF-8 byte width, rather than character count, affect the cap.
    policy.rules[0].id = "界\"".repeat(MAX_POLICY_BYTES / 3);
    assert!(!compile(&policy, &network(), &nodes()).is_valid());
}

#[test]
fn wire_contract_requires_nullable_fields_and_rejects_unsupported_domain_kinds() {
    let full = r#"{"source_node_id":"a","destination_node_id":null,"destination_ip":null,"domain":null,"destination_country":null}"#;
    assert!(serde_json::from_str::<FlowQuery>(full).is_ok());
    for field in [
        "destination_node_id",
        "destination_ip",
        "domain",
        "destination_country",
    ] {
        let missing = full.replace(&format!(",\"{field}\":null"), "");
        assert!(
            serde_json::from_str::<FlowQuery>(&missing).is_err(),
            "{field}"
        );
    }
    assert!(
        serde_json::from_str::<MeshNode>(
            r#"{"id":"a","name":"A","mesh_ipv4":"100.64.0.1","mesh_ipv6":"fd42::1"}"#
        )
        .is_err()
    );
    assert!(serde_json::from_str::<RouteAction>(r#"{"kind":"route","via":[]}"#).is_err());
    assert!(serde_json::from_str::<PolicyDiagnostic>(r#"{"field":"id","message":"bad"}"#).is_err());
    assert!(
        serde_json::from_str::<RouteDecision>(
            r#"{"action":"reject","rule_id":null,"path":[],"reason":"default"}"#
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<RouteDecision>(
            r#"{"action":"reject","exit_node_id":null,"path":[],"reason":"default"}"#
        )
        .is_err()
    );
    for kind in ["domain_prefix", "domain_exact", "regex"] {
        assert!(
            serde_json::from_str::<DestinationSelector>(&format!(
                r#"{{"kind":"{kind}","value":"example.com"}}"#
            ))
            .is_err()
        );
    }
    assert_eq!(
        serde_json::to_string(&DestinationSelectorKind::DomainSuffix).unwrap(),
        "\"domain_suffix\""
    );
    assert_eq!(
        serde_json::to_string(&RoutingPolicy::default()).unwrap(),
        r#"{"default_action":"reject","rules":[]}"#
    );
}
