use meshway_configuration::*;
use meshway_model::NodeId;
use meshway_policy::{
    SubjectContext, domain_suffix_matches, normalize, normalize_with_dependencies,
    plan_configuration, topology_allows,
};
use meshway_transport_spec::{DialerSet, Listener, MaterialRequirement};
use std::collections::BTreeMap;

fn pools() -> AddressPools {
    AddressPools {
        ipv4: vec!["100.80.0.0/16".into()],
        ipv6: vec!["fd42:7a65:8800::/48".into()],
    }
}
fn initial() -> NetworkConfiguration {
    new_network(pools(), "Mesh.Example.".into()).unwrap()
}
fn input() -> NetworkConfigurationInput {
    parse_jsonc(canonical_json(&initial()).unwrap().expose()).unwrap()
}
fn id(n: u8) -> NodeId {
    format!("00000000-0000-0000-0000-{n:012}").parse().unwrap()
}
fn user_identity() -> NodeIdentity {
    NodeIdentity::User {
        user: "alice@example.com".into(),
    }
}
fn context() -> ValidationContext<'static> {
    ValidationContext {
        members: BTreeMap::from([(
            "alice@example.com".into(),
            UserBinding {
                user_id: "00000000-0000-0000-0000-000000000010".parse().unwrap(),
                membership_id: "00000000-0000-0000-0000-000000000020".parse().unwrap(),
            },
        )]),
        nodes: [id(1), id(2), id(3)].into_iter().collect(),
        approved_identities: [id(1), id(2), id(3)]
            .into_iter()
            .map(|id| (id, user_identity()))
            .collect(),
        ..Default::default()
    }
}
fn node(n: u8) -> NodeSettingsInput {
    serde_json::from_str(&format!(r#"{{"hostname":"node-{n}","identity":{{"type":"user","user":"alice@example.com"}},"addresses":{{"ipv4":"100.80.0.{n}","ipv6":null}}}}"#)).unwrap()
}
fn with_nodes() -> NetworkConfigurationInput {
    let mut c = input();
    c.nodes = [(id(1), node(1)), (id(2), node(2)), (id(3), node(3))]
        .into_iter()
        .collect();
    c
}

#[test]
fn dns_name_normalization_and_combined_limit_are_shared_with_configuration() {
    assert_eq!(
        normalize_dns_name("NODE.BÜCHER.Example.").unwrap(),
        "node.xn--bcher-kva.example"
    );
    for invalid in [
        "",
        ".",
        "*.example",
        "a..example",
        "a.example..",
        "127.0.0.1",
        "127.0.0.1.",
        "１２７。０。０。１",
        "::1",
        "https://example.test",
        "a.example:53",
    ] {
        assert!(normalize_dns_name(invalid).is_err(), "{invalid}");
    }
    assert!(normalize_dns_name(&"é".repeat(MAX_DNS_NAME_INPUT_BYTES)).is_err());
    assert!(node_dns_name("127", "0.0.1").is_err());
    let mut numeric = with_nodes();
    numeric.dns.domain = "0.0.1".into();
    numeric.nodes.get_mut(&id(1)).unwrap().hostname = "127".into();
    assert!(
        matches!(normalize(numeric, &context()), Err(ConfigurationError::Invalid(issues))
        if issues.iter().any(|issue| issue.path == format!("/nodes/{}/hostname", id(1))))
    );
    let domain = [
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(54),
    ]
    .join(".");
    let mut c = with_nodes();
    c.dns.domain = domain.clone();
    assert_eq!(node_dns_name("node-1", &domain).unwrap().len(), 253);
    normalize(c.clone(), &context()).unwrap();
    c.dns.domain.push('d');
    assert!(normalize_dns_name(&c.dns.domain).is_ok());
    assert!(node_dns_name("node-1", &c.dns.domain).is_err());
    let ConfigurationError::Invalid(issues) = normalize(c, &context()).unwrap_err() else {
        panic!("expected combined-name validation failure");
    };
    assert!(
        issues
            .iter()
            .any(|issue| issue.path == format!("/nodes/{}/hostname", id(1)))
    );
}

#[test]
fn access_traffic_grammar_rejects_empty_and_unsupported_sets() {
    for token in [
        "tcp:0",
        "tcp:01",
        "tcp:65536",
        "tcp:9-8",
        "udp:",
        "icmp:8",
        "sctp:*",
        "TCP:80",
        "tcp:1-",
        "tcp:+1",
    ] {
        let mut c = with_nodes();
        c.access[0].ip = vec![token.into()];
        assert!(normalize(c, &context()).is_err(), "{token}");
    }
    for token in [
        "*",
        "tcp:65432",
        "tcp:1-65535",
        "udp:41000-41007",
        "udp:*",
        "icmp:*",
    ] {
        let mut c = with_nodes();
        c.access[0].ip = vec![token.into()];
        normalize(c, &context()).unwrap();
    }
}

#[test]
fn declaration_preview_defers_material_only_and_cannot_normalize_as_execution_input() {
    let mut input = with_nodes();
    input.nodes.get_mut(&id(1)).unwrap().listeners.insert(
        "vless".into(),
        serde_json::from_str(
            r#"{"type":"vless","listenPort":443,"tls":{"serverName":"node.example.test"}}"#,
        )
        .unwrap(),
    );
    let plan = plan_configuration(input.clone(), &context()).unwrap();
    assert_eq!(plan.requirements.len(), 4);
    assert!(!plan.view.nodes[&id(1)].listeners["vless"].credential_present);
    assert_eq!(
        plan.input.nodes[&id(1)].listeners["vless"],
        input.nodes[&id(1)].listeners["vless"]
    );
    assert!(matches!(
        normalize_with_dependencies(plan.input, &context()),
        Err(ConfigurationError::PreparationRequired(_))
    ));
    let mut no_identity = context();
    no_identity.approved_identities.clear();
    assert!(matches!(
        plan_configuration(input.clone(), &no_identity),
        Err(ConfigurationError::PreparationRequired(_))
    ));
    input.access.push(AccessRule {
        remark: String::new(),
        src: vec!["group:missing".into()],
        dst: vec!["*".into()],
        ip: vec!["*".into()],
        bidirectional: false,
    });
    assert!(matches!(
        plan_configuration(input, &context()),
        Err(ConfigurationError::Invalid(_))
    ));
}

fn egress_input(grants: &[&str], destination: &str) -> NetworkConfigurationInput {
    let mut c = with_nodes();
    c.nodes.get_mut(&id(2)).unwrap().exit_enabled = true;
    c.access = grants
        .iter()
        .map(|grant| AccessRule {
            remark: String::new(),
            src: vec![format!("node:{}", id(1))],
            dst: vec![(*grant).into()],
            ip: vec!["*".into()],
            bidirectional: false,
        })
        .collect();
    c.routing = serde_json::from_value(serde_json::json!({
        "egress": {"profiles":{"work":{"rules":[{"match":{"ipCIDR":[destination]},
            "action":{"type":"exit","nodes":[format!("node:{}",id(2))],"via":[]}}],
            "fallback":{"type":"unmanaged"}}},
            "assignments":[{"target":[format!("node:{}",id(1))],"allowed":["work"],"default":"work","locked":false}]}
    })).unwrap();
    c
}

#[test]
fn routing_destination_coverage_uses_internet_and_union_of_grants() {
    for grants in [
        vec!["autogroup:internet"],
        vec!["8.8.8.0/25", "8.8.8.128/25"],
    ] {
        normalize(egress_input(&grants, "8.8.8.0/24"), &context()).unwrap();
    }
    assert!(normalize(egress_input(&["8.8.8.0/25"], "8.8.8.0/24"), &context()).is_err());
    for destination in [
        "10.0.0.0/8",
        "100.64.0.0/10",
        "169.254.0.0/16",
        "fd00::/8",
        "fe80::/10",
    ] {
        assert!(
            normalize(
                egress_input(&["autogroup:internet"], destination),
                &context()
            )
            .is_err(),
            "{destination}"
        );
    }
    normalize(
        egress_input(&["autogroup:internet"], "2001:4860::/32"),
        &context(),
    )
    .unwrap();
}

#[test]
fn retained_routes_allow_narrowing_but_ruleset_expansion_requires_current_grants() {
    let mut input = egress_input(&["autogroup:internet"], "8.8.8.0/24");
    input.rule_sets.insert(
        "destinations".into(),
        RuleSet::Inline {
            ip_cidr: vec!["8.8.8.0/24".into()],
            domain_suffix: vec![],
            domain: vec![],
        },
    );
    let rule = &mut input.routing.egress.profiles.get_mut("work").unwrap().rules[0];
    rule.matcher.ip_cidr.clear();
    rule.matcher.rule_sets.push("destinations".into());
    let (old, _) = normalize(input, &context()).unwrap();
    let ctx = ValidationContext {
        previous: Some(&old),
        ..context()
    };
    for (cidr, accepted) in [("8.8.8.0/25", true), ("8.8.0.0/16", false)] {
        let mut changed = NetworkConfigurationInput::from(&old);
        changed.access.clear();
        changed
            .routing
            .egress
            .profiles
            .get_mut("work")
            .unwrap()
            .rules[0]
            .remark = "metadata edit".into();
        changed.rule_sets.insert(
            "destinations".into(),
            RuleSet::Inline {
                ip_cidr: vec![cidr.into()],
                domain_suffix: vec![],
                domain: vec![],
            },
        );
        let result = normalize_with_dependencies(changed, &ctx);
        assert_eq!(result.is_ok(), accepted);
        if let Ok(result) = result {
            assert!(!result.unavailable_dependencies.is_empty());
        }
    }
}

#[test]
fn retained_routing_cannot_prevent_revocation_or_silently_bypass_deleted_hops() {
    let (mut old, bindings) = normalize(
        egress_input(&["autogroup:internet"], "8.8.8.0/24"),
        &context(),
    )
    .unwrap();
    old.routing.mesh = serde_json::from_value(serde_json::json!([
        {"src":[format!("node:{}",id(1))], "dst":[format!("node:{}",id(2))], "via":[format!("node:{}",id(3))]}
    ])).unwrap();
    let ctx = ValidationContext {
        previous: Some(&old),
        expected_user_bindings: bindings.clone(),
        previous_user_bindings: bindings,
        ..context()
    };
    for kind in 0..3 {
        let mut input = NetworkConfigurationInput::from(&old);
        match kind {
            0 => input.access.clear(),
            1 => input.nodes.get_mut(&id(2)).unwrap().exit_enabled = false,
            _ => {
                input.nodes.remove(&id(2));
                input.nodes.remove(&id(3));
            }
        }
        let result = normalize_with_dependencies(input, &ctx).unwrap();
        assert_eq!(result.configuration.routing, old.routing);
        assert!(!result.unavailable_dependencies.is_empty());
        let next_context = ValidationContext {
            previous: Some(&result.configuration),
            ..context()
        };
        let repeated = normalize_with_dependencies(
            NetworkConfigurationInput::from(&result.configuration),
            &next_context,
        )
        .unwrap();
        assert!(!repeated.unavailable_dependencies.is_empty());
        let mut expanded = NetworkConfigurationInput::from(&result.configuration);
        expanded
            .routing
            .egress
            .profiles
            .get_mut("work")
            .unwrap()
            .rules[0]
            .matcher
            .ip_cidr = vec!["1.1.1.0/24".into()];
        assert!(normalize(expanded, &next_context).is_err());
    }
    let mut unknown = NetworkConfigurationInput::from(&old);
    unknown.routing.mesh[0].via = vec![format!("node:{}", id(99))];
    assert!(normalize(unknown, &ctx).is_err());
}
fn invalid(
    value: Result<(NetworkConfiguration, BTreeMap<String, UserBinding>), ConfigurationError>,
    text: &str,
) {
    let ConfigurationError::Invalid(issues) = value.unwrap_err() else {
        panic!("expected semantic error");
    };
    assert!(
        issues.iter().any(|i| i.message.contains(text)),
        "{issues:?}"
    );
}

#[test]
fn initial_network_is_complete_deterministic_and_mesh_only() {
    let config = initial();
    assert_eq!(config.dns.domain, "mesh.example");
    assert!(
        config.nodes.is_empty()
            && config.groups.is_empty()
            && config.tags_owners.is_empty()
            && config.rule_sets.is_empty()
    );
    assert!(
        config.topology.is_empty()
            && config.routing.mesh.is_empty()
            && config.routing.egress.rules.is_empty()
            && config.routing.egress.profiles.is_empty()
    );
    assert_eq!(config.defaults.access, AccessDefault::Deny);
    assert_eq!(config.access[0].dst, vec!["*"]);
    assert!(
        !config
            .access
            .iter()
            .any(|r| r.dst.contains(&"autogroup:internet".into()))
    );
    assert_eq!(
        config
            .defaults
            .dialers
            .native_udp
            .as_ref()
            .unwrap()
            .priority,
        100
    );
    assert_eq!(
        config
            .defaults
            .dialers
            .native_tcp
            .as_ref()
            .unwrap()
            .priority,
        200
    );
    assert_eq!(
        config.defaults.dialers.vless.as_ref().unwrap().priority,
        300
    );
    assert!(config.defaults.dialers.trojan.is_none());
    let json = canonical_json(&config).unwrap();
    let source = format!("// preserve me\n{}\n", json.expose());
    let (configuration, _) =
        normalize(parse_jsonc(&source).unwrap(), &ValidationContext::default()).unwrap();
    let document = ConfigurationDocument::new(source, configuration);
    assert!(
        document
            .source_jsonc
            .expose()
            .starts_with("// preserve me\n")
    );
    assert_eq!(json, canonical_json(&document.configuration).unwrap());
    assert_eq!(
        digest(&config)
            .map_err(|_| ConfigurationError::Serialization)
            .unwrap(),
        digest(&document.configuration)
            .map_err(|_| ConfigurationError::Serialization)
            .unwrap()
    );
    assert_eq!(
        serde_json::from_str::<NetworkConfiguration>(json.expose()).unwrap(),
        config
    );
}

#[test]
fn defaults_do_not_open_listeners_or_exits_and_empty_dialers_disable() {
    let mut c = with_nodes();
    c.nodes.get_mut(&id(2)).unwrap().dialers = Input::Value(DialerSet::default());
    c.nodes.get_mut(&id(3)).unwrap().dialers =
        Input::Value(serde_json::from_str(r#"{"vless":{}}"#).unwrap());
    let (c, _) = normalize(c, &context()).unwrap();
    assert!(c.nodes[&id(1)].dialers.is_omitted());
    assert_eq!(c.nodes[&id(2)].dialers, Input::Value(DialerSet::default()));
    let Input::Value(d) = &c.nodes[&id(3)].dialers else {
        panic!()
    };
    assert!(d.native_udp.is_none() && d.native_tcp.is_none() && d.vless.is_some());
    assert!(
        c.nodes
            .values()
            .all(|n| n.listeners.is_empty() && !n.exit_enabled)
    );
    let text = canonical_json(&c).unwrap();
    assert!(text.expose().contains("\"dialers\":{}"));
}

#[test]
fn omission_null_and_empty_address_families_are_not_interchangeable() {
    let (old, _) = normalize(with_nodes(), &context()).unwrap();
    let mut ctx = context();
    ctx.previous = Some(&old);
    let mut c = with_nodes();
    c.address_pools = AddressPoolsInput::default();
    c.nodes.get_mut(&id(1)).unwrap().addresses = NodeAddressesInput::default();
    let (retained, _) = normalize(c.clone(), &ctx).unwrap();
    assert_eq!(
        retained.nodes[&id(1)].addresses,
        old.nodes[&id(1)].addresses
    );
    c.nodes.get_mut(&id(1)).unwrap().addresses.ipv4 = Input::Value(None);
    assert_eq!(
        normalize(c.clone(), &ctx).unwrap().0.nodes[&id(1)]
            .addresses
            .ipv4,
        None
    );
    c.address_pools.ipv4 = Input::Value(vec![]);
    invalid(normalize(c, &ctx), "outside usable pool");
    let mut first = with_nodes();
    first.nodes.get_mut(&id(1)).unwrap().addresses = NodeAddressesInput::default();
    let ConfigurationError::PreparationRequired(requirements) =
        normalize(first, &context()).unwrap_err()
    else {
        panic!()
    };
    assert!(
        requirements
            .iter()
            .any(|r| r.kind == PreparationKind::AddressV4)
    );
    assert!(serde_json::from_str::<NodeAddresses>(r#"{}"#).is_err());
    assert!(serde_json::from_str::<NodeAddressesInput>(r#"{"ipv4":null}"#).is_ok());
}

#[test]
fn jsonc_strictly_rejects_unknown_duplicate_and_wrong_shapes() {
    let valid = canonical_json(&initial()).unwrap();
    for input in [
        format!("{{\"typo\":true,{}", &valid.expose()[1..]),
        valid
            .expose()
            .replace("\"access\":[", "\"access\":[],\"access\":["),
        valid
            .expose()
            .replace("\"groups\":{}", r#""groups":{"group:x":[],"group:x":[]}"#),
        valid.expose().replace("\"groups\":{}", r#""groups":[]"#),
        valid
            .expose()
            .replace("\"dns\":{", r#""dns":{"domain":"other.example","#),
        valid
            .expose()
            .replace("\"remark\":\"默认允许 Mesh 成员互访\"", r#""remark":null"#),
        valid
            .expose()
            .replace("\"access\":\"deny\"", r#""access":{"deny":null}"#),
        "[]".into(),
        r#"{"addressPools":[[],[]],"dns":{"domain":"mesh.example"}}"#.into(),
    ] {
        assert!(parse_jsonc(&input).is_err(), "accepted: {input}");
    }
    let commented = valid
        .expose()
        .replace("\"groups\":{}", "/* groups */ \"groups\":{},");
    assert!(parse_jsonc(&commented).is_err()); // two consecutive commas
    assert!(
        parse_jsonc(
            "{\"addressPools\":{\"ipv4\":[],\"ipv6\":[],},\"dns\":{\"domain\":\"mesh.example\",},}"
        )
        .is_ok()
    );
}

#[test]
fn inventory_and_stable_user_bindings_cannot_be_invented_by_configuration() {
    invalid(
        normalize(with_nodes(), &ValidationContext::default()),
        "unknown or revoked Node",
    );
    let mut c = with_nodes();
    c.groups
        .insert("group:dev".into(), vec!["group:other".into()]);
    invalid(
        normalize(c, &context()),
        "unknown, inactive, or rebound user",
    );
    let mut c = with_nodes();
    c.nodes.get_mut(&id(1)).unwrap().identity = NodeIdentity::Tag {
        tags: vec!["tag:missing".into()],
    };
    invalid(normalize(c, &context()), "unknown tag");
    let mut ctx = context();
    let mut old = ctx.members["alice@example.com"].clone();
    old.user_id = "00000000-0000-0000-0000-000000000099".parse().unwrap();
    ctx.expected_node_user_bindings.insert(id(1), old);
    invalid(normalize(with_nodes(), &ctx), "active bound membership");
    let mut ctx = context();
    ctx.approved_identities.clear();
    assert!(matches!(
        normalize(with_nodes(), &ctx),
        Err(ConfigurationError::PreparationRequired(_))
    ));
    for identity in [
        r#"{"type":"user","user":"a","tags":[]}"#,
        r#"{"type":"tag","tags":[],"user":"a"}"#,
        r#"["user","a"]"#,
    ] {
        assert!(serde_json::from_str::<NodeIdentity>(identity).is_err());
    }
}

#[test]
fn pools_and_addresses_are_checked_as_one_complete_target() {
    let mut c = with_nodes();
    c.address_pools.ipv4 = Input::Value(vec!["100.80.0.0/16".into(), "100.80.1.0/24".into()]);
    invalid(normalize(c, &context()), "overlapping");
    let mut c = with_nodes();
    c.nodes.get_mut(&id(2)).unwrap().addresses.ipv4 =
        Input::Value(Some("100.80.0.1".parse().unwrap()));
    invalid(normalize(c, &context()), "duplicate address");
    let mut c = with_nodes();
    c.nodes.get_mut(&id(1)).unwrap().addresses.ipv4 =
        Input::Value(Some("10.0.0.1".parse().unwrap()));
    invalid(normalize(c, &context()), "outside usable pool");
    let mut c = with_nodes();
    c.nodes.get_mut(&id(1)).unwrap().addresses.ipv4 =
        Input::Value(Some("100.80.0.2".parse().unwrap()));
    c.nodes.get_mut(&id(2)).unwrap().addresses.ipv4 =
        Input::Value(Some("100.80.0.1".parse().unwrap()));
    assert!(normalize(c, &context()).is_ok()); // atomic exchange, not row-at-a-time
}

#[test]
fn dns_references_and_label_boundaries_are_real() {
    let mut c = input();
    c.dns.override_local = true;
    invalid(normalize(c, &context()), "requires default");
    let mut c = input();
    c.dns
        .routes
        .insert("corp.example".into(), vec!["missing".into()]);
    invalid(normalize(c, &context()), "unknown resolver");
    assert!(domain_suffix_matches("API.Corp.Example.", "corp.example"));
    assert!(!domain_suffix_matches("evilcorp.example", "corp.example"));
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
            a: "*".into(),
            b: "*".into(),
        },
        TopologyRule {
            remark: "".into(),
            action: TopologyAction::Allow,
            a: "*".into(),
            b: "*".into(),
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
fn routing_references_exit_flags_and_assignment_conflicts_are_rejected() {
    let mut c = with_nodes();
    c.routing.egress.profiles.insert(
        "work".into(),
        RoutingProfile {
            rules: vec![],
            fallback: RouteAction::Exit {
                nodes: vec![format!("node:{}", id(2))],
                via: vec![],
            },
        },
    );
    invalid(normalize(c, &context()), "not enabled as an exit");
    let mut c = with_nodes();
    c.routing.egress.assignments = vec![
        ProfileAssignment {
            target: vec!["*".into()],
            allowed: vec!["mesh".into()],
            default: "mesh".into(),
            locked: true,
        },
        ProfileAssignment {
            target: vec!["alice@example.com".into()],
            allowed: vec!["mesh".into()],
            default: "mesh".into(),
            locked: false,
        },
    ];
    invalid(normalize(c, &context()), "conflicting");
    let mut c = with_nodes();
    c.routing.egress.rules.push(EgressRule {
        remark: "".into(),
        matcher: RouteMatch {
            rule_sets: vec!["absent".into()],
            ..Default::default()
        },
        action: RuleAction::Direct,
    });
    invalid(normalize(c, &context()), "unknown rule set");
}

#[test]
fn missing_material_is_a_requirement_and_debug_never_exposes_it() {
    let mut c = with_nodes();
    let listener: Listener = serde_json::from_str(r#"{"type":"vless","listenPort":443,"stream":"websocket","tls":{"serverName":"Edge.Example"}}"#).unwrap();
    c.nodes
        .get_mut(&id(1))
        .unwrap()
        .listeners
        .insert("web".into(), listener);
    let ConfigurationError::PreparationRequired(requirements) =
        normalize(c, &context()).unwrap_err()
    else {
        panic!()
    };
    for kind in [
        MaterialRequirement::Uuid,
        MaterialRequirement::Certificate,
        MaterialRequirement::PrivateKey,
        MaterialRequirement::CertificateValidation,
    ] {
        assert!(
            requirements
                .iter()
                .any(|r| r.kind == PreparationKind::Material(kind))
        );
    }
    let source = "// super-secret\n{\"addressPools\":{\"ipv4\":[],\"ipv6\":[]},\"dns\":{\"domain\":\"mesh.example\"}}".to_owned();
    let (configuration, _) =
        normalize(parse_jsonc(&source).unwrap(), &ValidationContext::default()).unwrap();
    let document = ConfigurationDocument::new(source, configuration);
    assert!(!format!("{document:?}").contains("super-secret"));
}

#[test]
fn persisted_snapshots_cannot_silently_reapply_defaults() {
    let c = initial();
    let json = canonical_json(&c).unwrap();
    assert_eq!(restore_canonical_json(json.expose()).unwrap(), c);
    let incomplete = json.expose().replace(",\"bidirectional\":false", "");
    assert!(restore_canonical_json(&incomplete).is_err());
    let (configuration, _) = normalize(
        parse_jsonc(&incomplete).unwrap(),
        &ValidationContext::default(),
    )
    .unwrap();
    let document = ConfigurationDocument::new(incomplete, configuration);
    assert_eq!(document.configuration, c);
}

#[test]
fn listener_material_retention_requires_the_same_resource_and_exact_attestation() {
    let mut c = with_nodes();
    let mut declaration: Listener = serde_json::from_str(r#"{"type":"vless","listenPort":443,"uuid":"11111111-1111-4111-8111-111111111111","tls":{"serverName":"edge.example","certificate":"certificate fixture","privateKey":"key fixture"}}"#).unwrap();
    declaration.normalize().unwrap();
    c.nodes
        .get_mut(&id(1))
        .unwrap()
        .listeners
        .insert("edge".into(), declaration.clone());
    assert!(matches!(
        normalize(c.clone(), &context()),
        Err(ConfigurationError::PreparationRequired(_))
    ));
    let mut ctx = context();
    ctx.verified_listeners
        .insert((id(1), "edge".into()), declaration.clone());
    let (old, _) = normalize(c, &ctx).unwrap();
    let mut edit = NetworkConfigurationInput::from(&old);
    let Listener::Vless(v) = edit
        .nodes
        .get_mut(&id(1))
        .unwrap()
        .listeners
        .get_mut("edge")
        .unwrap()
    else {
        panic!()
    };
    v.uuid = None;
    v.tls.certificate = None;
    v.tls.private_key = None;
    ctx.previous = Some(&old);
    assert_eq!(
        normalize(edit.clone(), &ctx).unwrap().0.nodes[&id(1)].listeners,
        old.nodes[&id(1)].listeners
    );
    let mut deleted = old.clone();
    deleted.nodes.get_mut(&id(1)).unwrap().listeners.clear();
    ctx.previous = Some(&deleted);
    let ConfigurationError::PreparationRequired(requirements) = normalize(edit, &ctx).unwrap_err()
    else {
        panic!()
    };
    assert!(
        requirements
            .iter()
            .any(|r| r.kind == PreparationKind::Material(MaterialRequirement::Uuid))
    );
}

#[test]
fn assignments_merge_allowed_and_do_not_make_mesh_wildcards_internet_grants() {
    let mut c = with_nodes();
    c.nodes.get_mut(&id(2)).unwrap().exit_enabled = true;
    let global = format!("global:{}", id(2));
    c.routing.egress.assignments = vec![
        ProfileAssignment {
            target: vec!["*".into()],
            allowed: vec!["mesh".into()],
            default: "mesh".into(),
            locked: false,
        },
        ProfileAssignment {
            target: vec!["*".into()],
            allowed: vec![global],
            default: "mesh".into(),
            locked: false,
        },
    ];
    invalid(normalize(c.clone(), &context()), "requires internet access");
    c.access.push(AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["autogroup:internet".into()],
        ip: vec!["*".into()],
        bidirectional: false,
    });
    assert!(normalize(c, &context()).is_ok());
}

#[test]
fn remote_rule_sources_are_typed_https_declarations() {
    for url in [
        "http://example.com/rules",
        "https://user:password@example.com/rules",
        "https://:443/rules",
        "https://example.com:bad/rules",
    ] {
        let mut c = input();
        c.rule_sets.insert(
            "external".into(),
            RuleSet::Remote {
                url: url.into(),
                format: RuleSetFormat::SingBoxSource,
                update_interval: "24h".parse().unwrap(),
            },
        );
        invalid(normalize(c, &context()), "HTTPS");
    }
}

#[test]
fn ipam_node_validation_independently_rejects_excluded_single_addresses() {
    for (ip, prefix) in [
        ("0.0.0.0", 32),
        ("0.255.255.255", 32),
        ("127.0.0.0", 32),
        ("127.255.255.255", 32),
        ("169.254.0.0", 32),
        ("169.254.255.255", 32),
        ("224.0.0.0", 32),
        ("239.255.255.255", 32),
        ("240.0.0.0", 32),
        ("255.255.255.254", 32),
        ("255.255.255.255", 32),
        ("::", 128),
        ("::1", 128),
        ("fe80::", 128),
        ("febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff", 128),
        ("ff00::", 128),
        ("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", 128),
    ] {
        let mut c = input();
        let mut n = node(1);
        let family = if prefix == 32 {
            c.address_pools.ipv4 = Input::Value(vec![format!("{ip}/{prefix}")]);
            c.address_pools.ipv6 = Input::Value(vec![]);
            n.addresses.ipv4 = Input::Value(Some(ip.parse().unwrap()));
            "ipv4"
        } else {
            c.address_pools.ipv4 = Input::Value(vec![]);
            c.address_pools.ipv6 = Input::Value(vec![format!("{ip}/{prefix}")]);
            n.addresses.ipv4 = Input::Value(None);
            n.addresses.ipv6 = Input::Value(Some(ip.parse().unwrap()));
            "ipv6"
        };
        c.nodes.insert(id(1), n);
        let ConfigurationError::Invalid(issues) = normalize(c, &context()).unwrap_err() else {
            panic!("expected invalid {ip}");
        };
        // A rejected pool alone must not hide an allocation-classifier hole.
        assert!(
            issues
                .iter()
                .any(|i| i.path == format!("/nodes/{}/addresses/{family}", id(1))),
            "node classifier accepted {ip}: {issues:?}"
        );
    }
}

#[test]
fn ipam_point_to_point_and_single_address_allocations_keep_their_boundaries() {
    for (pool, ip, valid) in [
        ("100.64.0.0/31", "100.64.0.0", true),
        ("100.64.0.0/31", "100.64.0.1", true),
        ("100.64.0.255/32", "100.64.0.255", true),
        ("223.255.255.255/32", "223.255.255.255", true),
        ("100.64.0.0/30", "100.64.0.0", false),
        ("100.64.0.0/30", "100.64.0.1", true),
        ("100.64.0.0/30", "100.64.0.2", true),
        ("100.64.0.0/30", "100.64.0.3", false),
    ] {
        let mut c = input();
        c.address_pools.ipv4 = Input::Value(vec![pool.into()]);
        c.address_pools.ipv6 = Input::Value(vec![]);
        let mut n = node(1);
        n.addresses.ipv4 = Input::Value(Some(ip.parse().unwrap()));
        c.nodes.insert(id(1), n);
        assert_eq!(normalize(c, &context()).is_ok(), valid, "{ip} in {pool}");
    }
    let mut c = input();
    c.address_pools.ipv4 = Input::Value(vec![]);
    c.address_pools.ipv6 = Input::Value(vec!["fd00::1/128".into()]);
    let mut n = node(1);
    n.addresses.ipv4 = Input::Value(None);
    n.addresses.ipv6 = Input::Value(Some("fd00::1".parse().unwrap()));
    c.nodes.insert(id(1), n);
    assert!(normalize(c, &context()).is_ok());
}

#[test]
fn large_rule_set_validation_preserves_grants_across_unrelated_edits() {
    let entries = 8_500u32;
    let cidrs: Vec<String> = (0..entries)
        .map(|i| format!("1.{}.{}.0/24", (i >> 8) & 0xff, i & 0xff))
        .collect();
    let domains: Vec<String> = (0..entries).map(|i| format!("site{i}.example")).collect();
    let mut input = egress_input(&["autogroup:internet"], "8.8.8.0/24");
    input.access.push(AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", id(1))],
        dst: domains.iter().map(|d| format!("domain:{d}")).collect(),
        ip: vec!["*".into()],
        bidirectional: false,
    });
    input.rule_sets.insert(
        "big".into(),
        RuleSet::Inline {
            ip_cidr: cidrs,
            domain_suffix: domains,
            domain: vec![],
        },
    );
    let rule = &mut input.routing.egress.profiles.get_mut("work").unwrap().rules[0];
    rule.matcher.ip_cidr.clear();
    rule.matcher.rule_sets.push("big".into());
    let (old, bindings) = normalize(input, &context()).unwrap();
    let ctx = ValidationContext {
        previous: Some(&old),
        expected_user_bindings: bindings.clone(),
        previous_user_bindings: bindings,
        ..context()
    };
    let mut edited = NetworkConfigurationInput::from(&old);
    edited
        .groups
        .insert("group:unrelated".into(), vec!["alice@example.com".into()]);
    let result = normalize_with_dependencies(edited, &ctx).unwrap();
    assert_eq!(result.configuration.rule_sets, old.rule_sets);
    assert_eq!(result.configuration.access, old.access);
    assert_eq!(result.configuration.routing, old.routing);
    assert_eq!(
        result.configuration.groups["group:unrelated"],
        vec!["alice@example.com"]
    );
    assert!(result.unavailable_dependencies.is_empty());
}

fn issues(value: Result<NormalizedConfiguration, ConfigurationError>) -> Vec<ConfigurationIssue> {
    match value {
        Err(ConfigurationError::Invalid(issues)) => issues,
        Err(other) => panic!("expected validation issues, got {other:?}"),
        Ok(_) => panic!("expected validation issues, got a normalized configuration"),
    }
}

#[test]
fn diagnostics_carry_closed_codes_parameters_and_precise_pointers() {
    // A node-level exit authorization problem points at the rule and names the node.
    let found = issues(normalize_with_dependencies(
        egress_input(&["8.8.8.0/25"], "8.8.8.0/24"),
        &context(),
    ));
    let outside = found
        .iter()
        .find(|i| i.code == IssueCode::ExitOutsideAccess)
        .unwrap_or_else(|| panic!("{found:?}"));
    assert_eq!(outside.path, "/routing/egress/profiles/work/rules/0");
    assert_eq!(outside.params["node"], id(1).to_string());
    assert_eq!(outside.params["destination"], "8.8.8.0/24");
    assert_eq!(outside.message, IssueCode::ExitOutsideAccess.message());
    // Selector problems point at the element, not the array.
    let mut c = with_nodes();
    c.access.push(AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", id(1)), "group:nobody".into()],
        dst: vec!["mallory@example.com".into()],
        ip: vec!["*".into()],
        bidirectional: false,
    });
    let found = issues(normalize_with_dependencies(c, &context()));
    let group = found
        .iter()
        .find(|i| i.code == IssueCode::UnknownGroup)
        .unwrap();
    assert_eq!(group.path, "/access/1/src/1");
    assert_eq!(group.params["name"], "group:nobody");
    let user = found
        .iter()
        .find(|i| i.code == IssueCode::UnknownUser)
        .unwrap();
    assert_eq!(user.path, "/access/1/dst/0");
    assert_eq!(user.params["user"], "mallory@example.com");
    // Transport declaration problems carry the declaration code.
    let mut c = with_nodes();
    c.nodes.get_mut(&id(1)).unwrap().listeners.insert(
        "vless".into(),
        serde_json::from_str(
            r#"{"type":"vless","listenPort":443,"flow":"xtls","tls":{"serverName":"node.example.test"}}"#,
        )
        .unwrap(),
    );
    let found = issues(normalize_with_dependencies(c, &context()));
    let transport = found
        .iter()
        .find(|i| i.code == IssueCode::InvalidTransport)
        .unwrap();
    assert_eq!(transport.path, format!("/nodes/{}/listeners/vless", id(1)));
    assert_eq!(transport.params["reason"], "vless_flow");
    // Serialized shape: code is snake_case, params always present.
    let json = serde_json::to_value(transport).unwrap();
    assert_eq!(json["code"], "invalid_transport");
    assert!(json["params"].is_object());
}

#[test]
fn source_parse_errors_report_position_without_echoing_text() {
    let syntax = parse_jsonc("{\"dns\": {\"domain\": \"x.test\",}} trailing").unwrap_err();
    let ConfigurationError::Invalid(found) = syntax else {
        panic!("{syntax:?}")
    };
    assert_eq!(found[0].code, IssueCode::InvalidJsonc);
    assert_eq!(found[0].params["line"], "1");
    assert!(found[0].params.contains_key("column"));
    let shape =
        parse_jsonc("{\n  \"dns\": {\"domain\": \"x.test\"},\n  \"groups\": \"s3cret-value\"\n}")
            .unwrap_err();
    let ConfigurationError::Invalid(found) = shape else {
        panic!("{shape:?}")
    };
    assert_eq!(found[0].code, IssueCode::InvalidShape);
    assert_eq!(found[0].params["line"], "3");
    assert!(!format!("{found:?}").contains("s3cret"));
}

#[test]
fn a_rule_fully_covered_by_earlier_rules_is_a_warning_not_an_error() {
    let mut c = with_nodes();
    c.nodes.get_mut(&id(2)).unwrap().exit_enabled = true;
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["autogroup:internet".into(), "domain:example.org".into()],
        ip: vec!["*".into()],
        bidirectional: false,
    }];
    let exit = serde_json::json!({"type":"exit","nodes":[format!("node:{}",id(2))],"via":[]});
    c.routing = serde_json::from_value(serde_json::json!({
        "egress": {"rules":[
            {"match":{"ipCIDR":["8.8.0.0/16"]},"action":exit},
            {"match":{"domainSuffix":["example.org"]},"action":{"type":"direct"}},
            {"match":{"ipCIDR":["8.8.8.0/24"],"domainSuffix":["a.example.org"]},"action":exit},
            {"match":{"ipCIDR":["9.9.9.0/24"]},"action":exit}
        ]}
    }))
    .unwrap();
    let normalized = normalize_with_dependencies(c, &context()).unwrap();
    assert_eq!(normalized.warnings.len(), 1, "{:?}", normalized.warnings);
    let warning = &normalized.warnings[0];
    assert_eq!(warning.code, IssueCode::ShadowedRule);
    assert_eq!(warning.path, "/routing/egress/rules/2");
    assert_eq!(warning.params["by"], "1");
}

fn domain_rules(access: &[&str], rules: serde_json::Value) -> NetworkConfigurationInput {
    let mut c = with_nodes();
    c.nodes.get_mut(&id(2)).unwrap().exit_enabled = true;
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: access.iter().map(|d| (*d).into()).collect(),
        ip: vec!["*".into()],
        bidirectional: false,
    }];
    c.rule_sets.insert(
        "names".into(),
        RuleSet::Inline {
            ip_cidr: vec![],
            domain_suffix: vec![],
            domain: vec!["Telegram.org.".into()],
        },
    );
    c.routing =
        serde_json::from_value(serde_json::json!({ "egress": { "rules": rules } })).unwrap();
    c
}

#[test]
fn domain_destinations_are_covered_by_suffix_grants_or_an_internet_grant() {
    let exit = serde_json::json!({"type":"exit","nodes":[format!("node:{}",id(2))],"via":[]});
    let rules = serde_json::json!([
        {"match":{"domain":["api.example.org"],"ruleSets":["names"]},"action":exit}
    ]);
    // Address-set semantics: public resolutions are inside autogroup:internet.
    let (normalized, _) = normalize(
        domain_rules(&["autogroup:internet"], rules.clone()),
        &context(),
    )
    .unwrap();
    assert_eq!(
        normalized.rule_sets["names"],
        RuleSet::Inline {
            ip_cidr: vec![],
            domain_suffix: vec![],
            domain: vec!["telegram.org".into()],
        }
    );
    normalize(
        domain_rules(
            &["domain:example.org", "domain:telegram.org"],
            rules.clone(),
        ),
        &context(),
    )
    .unwrap();
    // A partial address grant or a sibling suffix does not cover a name.
    for grants in [
        vec!["8.8.8.0/24", "domain:example.org"],
        vec!["domain:example.org", "domain:web.telegram.org"],
    ] {
        let error = normalize(domain_rules(&grants, rules.clone()), &context()).unwrap_err();
        let ConfigurationError::Invalid(issues) = error else {
            panic!("{error:?}")
        };
        // One issue per node the rule applies to (`src: *`), none for example.org.
        assert_eq!(issues.len(), 3, "{issues:?}");
        for issue in &issues {
            assert_eq!(issue.code, IssueCode::ExitOutsideAccess);
            assert_eq!(issue.path, "/routing/egress/rules/0");
            assert_eq!(issue.params["destination"], "domain:telegram.org");
        }
    }
    let error = normalize(
        domain_rules(
            &["autogroup:internet"],
            serde_json::json!([{"match":{"domain":["bad_name", "ok.example"]},"action":exit}]),
        ),
        &context(),
    )
    .unwrap_err();
    let ConfigurationError::Invalid(issues) = error else {
        panic!("{error:?}")
    };
    assert_eq!(issues[0].code, IssueCode::InvalidDomain);
    assert_eq!(issues[0].path, "/routing/egress/rules/0/match/domain/0");
    assert_eq!(issues[0].params["value"], "bad_name");
}

#[test]
fn exact_names_are_shadowed_by_suffixes_but_never_shadow_them() {
    let exit = serde_json::json!({"type":"exit","nodes":[format!("node:{}",id(2))],"via":[]});
    let warnings = |rules: serde_json::Value| {
        normalize_with_dependencies(domain_rules(&["autogroup:internet"], rules), &context())
            .unwrap()
            .warnings
    };
    let shadowed = warnings(serde_json::json!([
        {"match":{"domainSuffix":["example.org"]},"action":{"type":"direct"}},
        {"match":{"domain":["a.example.org"]},"action":exit},
        {"match":{"domain":["example.org"]},"action":exit}
    ]));
    assert_eq!(shadowed.len(), 2, "{shadowed:?}");
    assert!(shadowed.iter().all(|w| w.code == IssueCode::ShadowedRule));
    // An exact name covers the same name (normalized), not its children or
    // the suffix of that name; the suffix then covers a later child.
    let same_name = warnings(serde_json::json!([
        {"match":{"domain":["example.org"]},"action":{"type":"direct"}},
        {"match":{"domain":["EXAMPLE.org."]},"action":exit},
        {"match":{"domain":["a.example.org"]},"action":exit},
        {"match":{"domainSuffix":["example.org"]},"action":exit},
        {"match":{"domain":["b.example.org"]},"action":exit}
    ]));
    assert_eq!(
        same_name
            .iter()
            .map(|w| (w.path.as_str(), w.params["by"].as_str()))
            .collect::<Vec<_>>(),
        [
            ("/routing/egress/rules/1", "0"),
            ("/routing/egress/rules/4", "3")
        ]
    );
}

#[test]
fn unmanaged_is_only_a_fallback_by_type() {
    let rule: Result<EgressRule, _> = serde_json::from_str(
        r#"{"match":{"ipCIDR":["8.8.8.0/24"]},"action":{"type":"unmanaged"}}"#,
    );
    assert!(rule.is_err());
    let profile: RoutingProfile =
        serde_json::from_str(r#"{"rules":[],"fallback":{"type":"unmanaged"}}"#).unwrap();
    assert_eq!(profile.fallback, RouteAction::Unmanaged);
}
