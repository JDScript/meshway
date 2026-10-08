use super::*;

#[test]
fn egress_first_match_no_fallback_on_revocation_and_dns_is_trusted() {
    let mut c = configuration();
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["domain:example.com".into()],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    c.routing.egress.rules = vec![EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            domain_suffix: vec!["example.com".into()],
            ..Default::default()
        },
        action: RuleAction::Exit {
            nodes: vec![format!("node:{}", node(4))],
            via: vec![format!("node:{}", node(3))],
        },
    }];
    let mut d = demand(1, 1, 4);
    d.target = BusinessTarget::Domain {
        name: "api.example.com".into(),
        address: ExecutionAddress("1.1.1.1".parse().unwrap()),
    };
    let ds = BTreeMap::from([(d.id, d)]);
    let r = run(&c, &inventory(), &ds, &[]);
    assert!(matches!(
        r.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::MissingDns
        }
    ));
    let dns = vec![TrustedDnsAssociation {
        network_id: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
        configuration_revision: Revision::MIN,
        source: node(1),
        name: "api.example.com".into(),
        cname_chain: vec![],
        addresses: vec![ExecutionAddress("1.1.1.1".parse().unwrap())],
        expires_at_ms: 2000,
    }];
    let r = run(&c, &inventory(), &ds, &dns);
    assert_eq!(
        r.nodes[&node(1)].configuration.permits[&request(1)].expires_at_ms,
        2000
    );
    // Renewing the intent cannot extend the trusted DNS association itself.
    let mut renewed = ds.clone();
    renewed.get_mut(&request(1)).unwrap().expires_at_ms = 200_000;
    let renewal = run(&c, &inventory(), &renewed, &dns);
    assert_eq!(
        renewal.nodes[&node(1)].configuration.permits[&request(1)],
        r.nodes[&node(1)].configuration.permits[&request(1)]
    );
    renewed.get_mut(&request(1)).unwrap().expires_at_ms = 1500;
    assert_eq!(
        run(&c, &inventory(), &renewed, &dns).nodes[&node(1)]
            .configuration
            .permits[&request(1)]
            .expires_at_ms,
        1500
    );
    let mut expired_dns = dns.clone();
    expired_dns[0].expires_at_ms = 1000;
    assert_eq!(
        run(&c, &inventory(), &renewed, &expired_dns).nodes[&node(1)]
            .configuration
            .decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::MissingDns,
        }
    );
    // TUN packets contain an IP, not the browser's domain string. The same
    // trusted association must drive routing and bound the grant lifetime.
    let mut ips = ds.clone();
    ips.get_mut(&request(1)).unwrap().target = BusinessTarget::Ip {
        address: ExecutionAddress("1.1.1.1".parse().unwrap()),
    };
    let ip_result = run(&c, &inventory(), &ips, &dns);
    let permit = &ip_result.nodes[&node(1)].configuration.permits[&request(1)];
    assert_eq!(permit.expires_at_ms, 2000);
    assert!(permit.paths.iter().all(|p| p.nodes.contains(&node(3))));
    // Even a non-DNS route must not extend a domain-only access grant.
    c.routing.egress.rules[0].matcher.ip_cidr = vec!["1.1.1.1/32".into()];
    assert_eq!(
        run(&c, &inventory(), &ips, &dns).nodes[&node(1)]
            .configuration
            .permits[&request(1)]
            .expires_at_ms,
        2000
    );
    let mut stale = dns.clone();
    stale[0].expires_at_ms = 1000;
    assert!(matches!(
        run(&c, &inventory(), &ips, &stale).nodes[&node(1)]
            .configuration
            .decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::Unauthorized
        }
    ));
    c.nodes.get_mut(&node(4)).unwrap().exit_enabled = false;
    let r = run(&c, &inventory(), &ds, &dns);
    assert!(matches!(
        r.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::NoPath
        }
    ));
}
#[test]
fn exact_domain_rules_match_one_name_through_trusted_dns() {
    let mut c = configuration();
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["autogroup:internet".into()],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    c.rule_sets.insert(
        "names".into(),
        RuleSet::Inline {
            ip_cidr: vec![],
            domain_suffix: vec![],
            domain: vec!["example.com".into()],
        },
    );
    c.routing.egress.rules = vec![EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            rule_sets: vec!["names".into()],
            ..Default::default()
        },
        action: RuleAction::Exit {
            nodes: vec![format!("node:{}", node(4))],
            via: vec![],
        },
    }];
    let association = |name: &str| TrustedDnsAssociation {
        network_id: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
        configuration_revision: Revision::MIN,
        source: node(1),
        name: name.into(),
        cname_chain: vec![],
        addresses: vec![ExecutionAddress("1.1.1.1".parse().unwrap())],
        expires_at_ms: 2000,
    };
    let mut d = demand(1, 1, 4);
    d.target = BusinessTarget::Domain {
        name: "api.example.com".into(),
        address: ExecutionAddress("1.1.1.1".parse().unwrap()),
    };
    let ds = BTreeMap::from([(d.id, d)]);
    // A child name is not the exact name: no rule fires, the fallback applies.
    let r = run(&c, &inventory(), &ds, &[association("api.example.com")]);
    assert!(matches!(
        r.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unmanaged
    ));
    let mut exact = ds.clone();
    exact.get_mut(&request(1)).unwrap().target = BusinessTarget::Domain {
        name: "Example.com.".into(),
        address: ExecutionAddress("1.1.1.1".parse().unwrap()),
    };
    let r = run(
        &c,
        &inventory(),
        &exact,
        &[association("Example.com."), association("api.example.com")],
    );
    let permit = &r.nodes[&node(1)].configuration.permits[&request(1)];
    assert_eq!(permit.expires_at_ms, 2000);
}

#[test]
fn mesh_ip_demands_never_fall_through_to_unmanaged_egress() {
    let mut c = configuration();
    let mut d = demand(1, 1, 2);
    d.target = BusinessTarget::Ip {
        address: ExecutionAddress("100.64.0.2".parse().unwrap()),
    };
    let mut ds = BTreeMap::from([(d.id, d)]);
    let result = run(&c, &inventory(), &ds, &[]);
    assert!(
        matches!(result.nodes[&node(1)].configuration.permits[&request(1)].target,BusinessTarget::Mesh{node:n} if n==node(2))
    );
    c.access.clear();
    let result = run(&c, &inventory(), &ds, &[]);
    assert!(matches!(
        result.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::Unauthorized
        }
    ));
    ds.get_mut(&request(1)).unwrap().target = BusinessTarget::Ip {
        address: ExecutionAddress("100.64.0.99".parse().unwrap()),
    };
    let result = run(&c, &inventory(), &ds, &[]);
    assert!(matches!(
        result.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::NoPath
        }
    ));
}

#[test]
fn backup_exit_survives_primary_path_budget() {
    let mut c = configuration();
    c.nodes.get_mut(&node(2)).unwrap().exit_enabled = true;
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["1.1.1.1/32".into()],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    c.routing.egress.rules = vec![EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            ip_cidr: vec!["1.1.1.1/32".into()],
            ..Default::default()
        },
        action: RuleAction::Exit {
            nodes: vec![format!("node:{}", node(4)), format!("node:{}", node(2))],
            via: vec![],
        },
    }];
    let mut d = demand(1, 1, 4);
    d.target = BusinessTarget::Ip {
        address: ExecutionAddress("1.1.1.1".parse().unwrap()),
    };
    let result = run(&c, &inventory(), &BTreeMap::from([(d.id, d)]), &[]);
    let paths = &result.nodes[&node(1)].configuration.permits[&request(1)].paths;
    assert!(paths.len() <= 2 * MAX_PATHS);
    assert_eq!(
        paths
            .iter()
            .filter(|p| p.nodes.last() == Some(&node(4)))
            .count(),
        MAX_PATHS
    );
    assert!(paths.iter().any(|p| p.nodes.last() == Some(&node(2))));
    // A local failure of every primary path still leaves an authorized backup.
    assert!(
        paths
            .iter()
            .any(|p| !p.nodes.contains(&node(4)) && p.nodes.last() == Some(&node(2)))
    );
}

#[test]
fn unavailable_rule_content_cannot_fall_through_to_direct() {
    let mut c = configuration();
    c.rule_sets.insert(
        "remote".into(),
        RuleSet::Remote {
            url: "https://rules.example.test/remote.json".into(),
            format: RuleSetFormat::SingBoxSource,
            update_interval: "24h".parse().unwrap(),
        },
    );
    c.routing.egress.rules = vec![
        EgressRule {
            remark: String::new(),
            matcher: RouteMatch {
                rule_sets: vec!["remote".into()],
                ..Default::default()
            },
            action: RuleAction::Exit {
                nodes: vec![format!("node:{}", node(4))],
                via: vec![],
            },
        },
        EgressRule {
            remark: String::new(),
            matcher: RouteMatch {
                ip_cidr: vec!["1.1.1.1/32".into()],
                ..Default::default()
            },
            action: RuleAction::Direct,
        },
    ];
    let mut d = demand(1, 1, 4);
    d.target = BusinessTarget::Ip {
        address: ExecutionAddress("1.1.1.1".parse().unwrap()),
    };
    let result = run(&c, &inventory(), &BTreeMap::from([(d.id, d)]), &[]);
    assert_eq!(
        result.nodes[&node(1)].configuration.decisions[&request(1)],
        DemandDecision::Unavailable {
            reason: UnavailableReason::MissingDependency
        }
    );
}
