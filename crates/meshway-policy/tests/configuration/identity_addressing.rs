use super::*;

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
