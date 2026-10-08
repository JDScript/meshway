use super::*;

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
    assert!(config.defaults.dialers.native_udp.as_ref().unwrap().enabled);
    assert!(config.defaults.dialers.native_tcp.as_ref().unwrap().enabled);
    assert!(config.defaults.dialers.vless.as_ref().unwrap().enabled);
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
fn persisted_snapshots_cannot_silently_reapply_defaults() {
    let c = initial();
    let json = canonical_json(&c).unwrap();
    assert_eq!(restore_canonical_json(json.expose()).unwrap(), c);
    let incomplete = json.expose().replace(",\"bidirectional\":false", "");
    assert!(restore_canonical_json(&incomplete).is_err());
    // Versions stored before `domain` existed name neither route-match nor
    // inline rule-set exact names; they must still read back unchanged.
    let mut dated = with_nodes();
    dated.nodes.get_mut(&id(2)).unwrap().exit_enabled = true;
    dated.access = vec![AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["10.20.0.0/16".into()],
        ip: vec!["*".into()],
        bidirectional: false,
    }];
    dated.rule_sets.insert(
        "office".into(),
        RuleSet::Inline {
            ip_cidr: vec!["10.20.0.0/16".into()],
            domain_suffix: vec![],
            domain: vec![],
        },
    );
    dated.routing = serde_json::from_value(serde_json::json!({ "egress": { "rules": [
        {"match":{"ruleSets":["office"]},"action":{"type":"exit","nodes":[format!("node:{}",id(2))],"via":[]}}
    ]}}))
    .unwrap();
    let (dated, _) = normalize(dated, &context()).unwrap();
    let stored = canonical_json(&dated).unwrap();
    assert!(
        !stored.expose().contains("\"domain\":[]"),
        "{}",
        stored.expose()
    );
    assert_eq!(restore_canonical_json(stored.expose()).unwrap(), dated);
    let mut named = dated.clone();
    named.routing.egress.rules[0].matcher.domain = vec!["login.example".into()];
    let stored = canonical_json(&named).unwrap();
    assert!(stored.expose().contains("\"domain\":[\"login.example\"]"));
    assert_eq!(restore_canonical_json(stored.expose()).unwrap(), named);
    let (configuration, _) = normalize(
        parse_jsonc(&incomplete).unwrap(),
        &ValidationContext::default(),
    )
    .unwrap();
    let document = ConfigurationDocument::new(incomplete, configuration);
    assert_eq!(document.configuration, c);
}
