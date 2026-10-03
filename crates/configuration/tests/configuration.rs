use meshway_configuration::*;
use meshway_model::NodeId;
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
    let document = parse_document(
        format!("// preserve me\n{}\n", json.expose()),
        &ValidationContext::default(),
    )
    .unwrap();
    assert!(
        document
            .source_jsonc
            .expose()
            .starts_with("// preserve me\n")
    );
    assert_eq!(json, canonical_json(&document.configuration).unwrap());
    assert_eq!(
        configuration_digest(&config).unwrap(),
        configuration_digest(&document.configuration).unwrap()
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
    ctx.expected_user_bindings
        .insert("alice@example.com".into(), old);
    invalid(normalize(with_nodes(), &ctx), "rebound user");
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
    assert!(!topology_allows(&c, id(1), id(2)));
    c.topology.reverse();
    assert!(!topology_allows(&c, id(1), id(2)));
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
        action: RouteAction::Direct,
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
    let document = parse_document(source, &ValidationContext::default()).unwrap();
    assert!(!format!("{document:?}").contains("super-secret"));
}

#[test]
fn persisted_snapshots_cannot_silently_reapply_defaults() {
    let c = initial();
    let json = canonical_json(&c).unwrap();
    assert_eq!(restore_canonical_json(json.expose()).unwrap(), c);
    let incomplete = json.expose().replace(",\"bidirectional\":false", "");
    assert!(restore_canonical_json(&incomplete).is_err());
    assert!(parse_document(incomplete, &ValidationContext::default()).is_ok());
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
fn ipam_pools_reject_intersections_with_excluded_ranges_and_invalid_families() {
    for cidr in [
        "0.0.0.0/0",
        "128.0.0.0/1",
        "126.0.0.0/7",
        "169.252.0.0/14",
        "0.0.0.0/32",
        "0.255.255.255/32",
        "127.0.0.0/32",
        "127.255.255.255/32",
        "169.254.0.0/32",
        "169.254.255.255/32",
        "224.0.0.0/32",
        "239.255.255.255/32",
        "240.0.0.0/32",
        "255.255.255.254/32",
        "255.255.255.255/32",
        "256.0.0.0/8",
        "100.64.0.0/33",
        "fd00::/8",
    ] {
        let pools = AddressPools {
            ipv4: vec![cidr.into()],
            ipv6: vec![],
        };
        assert!(
            matches!(
                new_network(pools, "mesh.example".into()),
                Err(ConfigurationError::Invalid(_))
            ),
            "accepted IPv4 pool {cidr}"
        );
    }
    for cidr in [
        "::/0",
        "::/127",
        "fe00::/8",
        "fe00::/7",
        "fe80::/9",
        "::/128",
        "::1/128",
        "fe80::/128",
        "febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128",
        "ff00::/128",
        "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128",
        "fd00::/129",
        "100.64.0.0/10",
    ] {
        let pools = AddressPools {
            ipv4: vec![],
            ipv6: vec![cidr.into()],
        };
        assert!(
            matches!(
                new_network(pools, "mesh.example".into()),
                Err(ConfigurationError::Invalid(_))
            ),
            "accepted IPv6 pool {cidr}"
        );
    }
    for json in [
        r#"{"ipv4":"256.0.0.1","ipv6":null}"#,
        r#"{"ipv4":"fd00::1","ipv6":null}"#,
        r#"{"ipv4":null,"ipv6":"100.64.0.1"}"#,
    ] {
        assert!(
            serde_json::from_str::<NodeAddressesInput>(json).is_err(),
            "accepted {json}"
        );
    }
}

#[test]
fn ipam_pools_keep_private_unicast_and_adjacent_allowed_boundaries() {
    // These are IPAM acceptance cases, not internet grant classifications.
    for cidr in [
        "10.0.0.0/8",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "100.64.0.0/10",
        "1.0.0.0/32",
        "126.255.255.255/32",
        "128.0.0.0/32",
        "169.253.255.255/32",
        "169.255.0.0/32",
        "223.255.255.255/32",
        "100.64.0.0/31",
        "100.64.0.255/32",
    ] {
        let pools = AddressPools {
            ipv4: vec![cidr.into()],
            ipv6: vec![],
        };
        assert!(
            new_network(pools, "mesh.example".into()).is_ok(),
            "rejected IPv4 pool {cidr}"
        );
    }
    for cidr in [
        "fc00::/7",
        "fd00::/8",
        "fd42:7a65:8800::/48",
        "fd00::1/128",
        "::2/128",
        "fe00::/9",
        "fe7f:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128",
        "fec0::/128",
        "feff:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128",
    ] {
        let pools = AddressPools {
            ipv4: vec![],
            ipv6: vec![cidr.into()],
        };
        assert!(
            new_network(pools, "mesh.example".into()).is_ok(),
            "rejected IPv6 pool {cidr}"
        );
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
