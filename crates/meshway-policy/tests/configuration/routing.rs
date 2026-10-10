use super::*;

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
            transports: Input::Omitted,
            keepalive: false,
        },
        TopologyRule {
            remark: "".into(),
            action: TopologyAction::Allow,
            a: "*".into(),
            b: "*".into(),
            transports: Input::Omitted,
            keepalive: false,
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
