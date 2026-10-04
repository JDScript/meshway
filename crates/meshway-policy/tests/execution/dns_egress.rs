use super::*;

fn requires_plan() -> DnsEgress {
    DnsEgress::Unavailable {
        reason: DnsEgressUnavailable::ExitPlanRequired,
    }
}

fn assignment(config: &mut NetworkConfiguration, default: &str, allowed: &[&str]) {
    config.routing.egress.assignments = vec![ProfileAssignment {
        target: vec!["*".into()],
        allowed: allowed.iter().map(|value| (*value).into()).collect(),
        default: default.into(),
        locked: false,
    }];
}

fn exit_rule() -> EgressRule {
    EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            domain_suffix: vec!["unsampled.example".into()],
            ..Default::default()
        },
        action: RuleAction::Exit {
            nodes: vec![format!("node:{}", node(4))],
            via: vec![],
        },
    }
}

fn classify(config: &NetworkConfiguration) -> DnsEgress {
    run(config, &inventory(), &BTreeMap::new(), &[]).nodes[&node(1)]
        .configuration
        .dns
        .egress
        .clone()
}

#[test]
fn default_network_proves_no_exit_without_demands_even_for_disabled_nodes() {
    let mut c = configuration();
    c.nodes.get_mut(&node(2)).unwrap().enabled = false;
    let result = run(&c, &inventory(), &BTreeMap::new(), &[]);
    for node in result.nodes.values() {
        assert_eq!(node.configuration.dns.profile, "mesh");
        assert_eq!(node.configuration.dns.egress, DnsEgress::NoExit);
        assert!(node.configuration.local_egress.is_empty());
        assert!(node.configuration.permits.is_empty());
    }
}

#[test]
fn mesh_profile_public_exit_requires_plan_even_shadowed_offline_or_disabled() {
    let mut c = configuration();
    c.routing.egress.rules = vec![
        EgressRule {
            matcher: RouteMatch {
                ip_cidr: vec!["0.0.0.0/0".into(), "::/0".into()],
                ..Default::default()
            },
            action: RuleAction::Direct,
            ..exit_rule()
        },
        exit_rule(),
    ];
    c.nodes.get_mut(&node(1)).unwrap().enabled = false;
    c.nodes.get_mut(&node(4)).unwrap().enabled = false;
    // No DNS samples or exit keys/capabilities can establish absence of an
    // explicitly configured exit; only the source has trusted inventory here.
    let mut inv = inventory();
    inv.retain(|id, _| *id == node(1));
    let result = run(&c, &inv, &BTreeMap::new(), &[]);
    assert_eq!(
        result.nodes[&node(1)].configuration.dns.egress,
        requires_plan()
    );
}

#[test]
fn global_and_custom_profiles_classify_all_rules_and_fallbacks() {
    let mut c = configuration();
    let global = format!("global:{}", node(4));
    assignment(&mut c, &global, &[&global]);
    assert_eq!(classify(&c), requires_plan());
    assignment(&mut c, "work", &["work"]);
    c.routing.egress.profiles.insert(
        "work".into(),
        RoutingProfile {
            rules: vec![],
            fallback: RouteAction::Exit {
                nodes: vec![format!("node:{}", node(4))],
                via: vec![],
            },
        },
    );
    assert_eq!(classify(&c), requires_plan());
    for fallback in [RouteAction::Direct, RouteAction::Unmanaged] {
        c.routing.egress.profiles.insert(
            "work".into(),
            RoutingProfile {
                rules: vec![EgressRule {
                    action: RuleAction::Direct,
                    ..exit_rule()
                }],
                fallback,
            },
        );
        assert_eq!(classify(&c), DnsEgress::NoExit);
        c.routing
            .egress
            .profiles
            .get_mut("work")
            .unwrap()
            .rules
            .push(exit_rule());
        assert_eq!(classify(&c), requires_plan());
    }
    // Profiles not selected or reached through auto are not source-effective.
    assignment(&mut c, "mesh", &["mesh", "work"]);
    assert_eq!(classify(&c), DnsEgress::NoExit);
}

#[test]
fn auto_checks_every_configured_choice_not_just_the_current_runtime_fallback() {
    let mut c = configuration();
    let global = format!("global:{}", node(4));
    assignment(&mut c, "auto", &["auto", "mesh", "work", &global]);
    c.routing.egress.profiles.insert(
        "work".into(),
        RoutingProfile {
            rules: vec![],
            fallback: RouteAction::Direct,
        },
    );
    c.routing.egress.auto = Input::Value(AutoProfile {
        rules: vec![CountryProfile {
            countries: vec!["US".into()],
            profile: global.clone(),
        }],
        fallback: "mesh".into(),
    });
    assert_eq!(classify(&c), requires_plan());
    let d = Demand {
        profile: "auto".into(),
        target: BusinessTarget::Ip {
            address: ExecutionAddress("1.1.1.1".parse().unwrap()),
        },
        ..demand(1, 1, 4)
    };
    let result = run(&c, &inventory(), &BTreeMap::from([(d.id, d.clone())]), &[]);
    assert_eq!(
        result.nodes[&d.source].configuration.decisions[&d.id],
        DemandDecision::Unmanaged
    );
    assert_eq!(
        result.nodes[&d.source].configuration.dns.egress,
        requires_plan()
    );
    c.routing.egress.auto = Input::Value(AutoProfile {
        rules: vec![CountryProfile {
            countries: vec!["US".into()],
            profile: "work".into(),
        }],
        fallback: "mesh".into(),
    });
    assert_eq!(classify(&c), DnsEgress::NoExit);
    c.routing
        .egress
        .profiles
        .get_mut("work")
        .unwrap()
        .rules
        .push(exit_rule());
    assert_eq!(classify(&c), requires_plan());
    c.routing.egress.auto = Input::Value(AutoProfile {
        rules: vec![],
        fallback: global,
    });
    assert_eq!(classify(&c), requires_plan());
    // Malformed/unknown dependencies cannot fabricate a proof or recurse.
    for fallback in ["auto", "missing"] {
        c.routing.egress.auto = Input::Value(AutoProfile {
            rules: vec![],
            fallback: fallback.into(),
        });
        assert_eq!(classify(&c), requires_plan());
    }
}

#[test]
fn dns_classification_uses_the_same_effective_selected_profile() {
    let mut c = configuration();
    let global = format!("global:{}", node(4));
    assignment(&mut c, "mesh", &["mesh", &global]);
    let inv = inventory();
    let profiles = BTreeMap::from([(node(1), global.clone())]);
    let result = compile(CompilationInput {
        network: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
        revision: Revision::MIN,
        publication: "00000000-0000-4004-8000-000000000001".parse().unwrap(),
        configuration: &c,
        subjects: &SubjectContext::default(),
        inventory: &inv,
        demands: &BTreeMap::new(),
        dns: &[],
        rule_sets: &BTreeMap::new(),
        selected_profiles: &profiles,
        now_ms: 1000,
    })
    .unwrap();
    assert_eq!(result.nodes[&node(1)].configuration.dns.profile, global);
    assert_eq!(
        result.nodes[&node(1)].configuration.dns.egress,
        requires_plan()
    );
    assert_eq!(
        result.nodes[&node(2)].configuration.dns.egress,
        DnsEgress::NoExit
    );
}

#[test]
fn dns_egress_wire_is_required_and_delta_replaces_the_proof() {
    assert_eq!(
        serde_json::to_value(DnsEgress::NoExit).unwrap(),
        serde_json::json!({"type":"noExit"})
    );
    assert_eq!(
        serde_json::to_value(requires_plan()).unwrap(),
        serde_json::json!({"type":"unavailable","reason":"exitPlanRequired"})
    );
    let mut c = configuration();
    let old = run(&c, &inventory(), &BTreeMap::new(), &[])
        .nodes
        .remove(&node(1))
        .unwrap();
    c.routing.egress.rules.push(exit_rule());
    let new = run(&c, &inventory(), &BTreeMap::new(), &[])
        .nodes
        .remove(&node(1))
        .unwrap();
    let delta = configuration_delta(&old, &new).unwrap();
    assert_eq!(delta.dns.egress, requires_plan());
    assert_eq!(apply_configuration_delta(&old, delta.clone()).unwrap(), new);
    let mut forged = delta;
    forged.dns.egress = DnsEgress::NoExit;
    assert!(apply_configuration_delta(&old, forged).is_err());
    let mut wire = serde_json::to_value(&old.configuration.dns).unwrap();
    assert!(wire.as_object_mut().unwrap().remove("egress").is_some());
    assert!(serde_json::from_value::<DnsExecution>(wire).is_err());
}
