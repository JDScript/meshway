//! Planning reports the same decisions compilation would make for a demand,
//! naming the configuration element behind each one.
use super::*;
use meshway_configuration::evaluation::*;

fn exit(nodes: &[NodeId], via: &[NodeId]) -> RuleAction {
    RuleAction::Exit {
        nodes: nodes.iter().map(|n| format!("node:{n}")).collect(),
        via: via.iter().map(|n| format!("node:{n}")).collect(),
    }
}

fn evaluate(
    c: &NetworkConfiguration,
    inv: &BTreeMap<NodeId, TrustedNode>,
    rule_sets: &BTreeMap<String, RuleSetContent>,
    destination: PlanDestination,
    profile: Option<&str>,
) -> Evaluation {
    let subjects = SubjectContext::default();
    plan(PlanRequest {
        topology: Topology {
            configuration: c,
            subjects: &subjects,
            inventory: inv,
        },
        rule_sets,
        selected_profiles: &BTreeMap::new(),
        source: node(1),
        destination,
        profile,
    })
    .unwrap()
}

fn routed() -> NetworkConfiguration {
    let mut c = configuration();
    c.access = vec![
        AccessRule {
            remark: String::new(),
            src: vec![format!("node:{}", node(1))],
            dst: vec![format!("node:{}", node(2))],
            ip: vec!["tcp:443".into()],
            bidirectional: false,
        },
        AccessRule {
            remark: String::new(),
            src: vec![format!("node:{}", node(2))],
            dst: vec![format!("node:{}", node(1))],
            ip: vec!["udp:*".into()],
            bidirectional: true,
        },
        AccessRule {
            remark: String::new(),
            src: vec!["*".into()],
            dst: vec!["autogroup:internet".into(), "domain:example.org".into()],
            ip: vec!["*".into()],
            bidirectional: false,
        },
    ];
    c.topology = vec![TopologyRule {
        remark: String::new(),
        a: format!("node:{}", node(1)),
        b: format!("node:{}", node(3)),
        action: TopologyAction::Allow,
    }];
    c.defaults.topology = TopologyDefault::Allow;
    c.routing.mesh = vec![MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec![format!("node:{}", node(2))],
        via: vec![format!("node:{}", node(3))],
    }];
    c.rule_sets.insert(
        "names".into(),
        RuleSet::Inline {
            ip_cidr: vec!["9.9.9.0/24".into()],
            domain_suffix: vec!["example.org".into()],
            domain: vec![],
        },
    );
    c.rule_sets.insert(
        "remote".into(),
        serde_json::from_value(serde_json::json!({
            "type": "remote", "url": "https://rules.example/list", "format": "mihomo-domain"
        }))
        .unwrap(),
    );
    c.routing.egress.rules = vec![EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            ip_cidr: vec!["8.8.8.0/24".into()],
            ..Default::default()
        },
        action: RuleAction::Direct,
    }];
    c.routing.egress.profiles.insert(
        "work".into(),
        RoutingProfile {
            rules: vec![
                EgressRule {
                    remark: String::new(),
                    matcher: RouteMatch {
                        rule_sets: vec!["remote".into()],
                        ..Default::default()
                    },
                    action: exit(&[node(4)], &[]),
                },
                EgressRule {
                    remark: String::new(),
                    matcher: RouteMatch {
                        rule_sets: vec!["names".into()],
                        ..Default::default()
                    },
                    action: exit(&[node(4)], &[node(3)]),
                },
            ],
            fallback: RouteAction::Unmanaged,
        },
    );
    c.routing.egress.assignments = vec![ProfileAssignment {
        target: vec!["*".into()],
        allowed: vec!["mesh".into(), "work".into()],
        default: "work".into(),
        locked: false,
    }];
    c
}

fn remote_content() -> BTreeMap<String, RuleSetContent> {
    BTreeMap::from([(
        "remote".to_owned(),
        RuleSetContent {
            network_id: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
            artifact: "00000000-0000-4005-8000-000000000001".parse().unwrap(),
            ip_cidr: vec![],
            domain_suffix: vec!["telegram.org".into()],
            domain: vec!["t.me".into()],
        },
    )])
}

#[test]
fn mesh_destination_names_access_route_and_topology_elements() {
    let c = routed();
    let e = evaluate(
        &c,
        &inventory(),
        &BTreeMap::new(),
        PlanDestination::Address("100.64.0.2".parse().unwrap()),
        None,
    );
    assert_eq!(
        e.destination,
        EvaluatedDestination::Node { node_id: node(2) }
    );
    assert_eq!(e.profile.name, "work");
    assert_eq!(
        e.profile.assignment.as_deref(),
        Some("/routing/egress/assignments/0")
    );
    assert!(!e.profile.requested);
    assert!(e.access.allowed);
    assert_eq!(e.access.grants.len(), 2);
    assert_eq!(e.access.grants[0].rule, "/access/0");
    assert!(!e.access.grants[0].reverse);
    assert_eq!(e.access.grants[1].rule, "/access/1");
    assert!(e.access.grants[1].reverse);
    let route = e.route.unwrap();
    assert_eq!(route.kind, RouteKind::Mesh);
    assert_eq!(route.rule.as_deref(), Some("/routing/mesh/0"));
    assert_eq!(route.via, vec![node(3)]);
    assert_eq!(route.targets, vec![node(2)]);
    assert_eq!(e.outcome, EvaluationOutcome::Permitted);
    // The primary path first; alternatives still pass the mandatory hop.
    assert_eq!(e.paths[0].nodes, vec![node(1), node(3), node(2)]);
    assert!(e.paths.iter().all(|p| p.nodes.contains(&node(3))));
    assert_eq!(e.paths[0].edges[0].topology.as_deref(), Some("/topology/0"));
    assert_eq!(e.paths[0].edges[1].topology, None);
    assert_eq!(e.targets[0].problem, None);
    assert_eq!(usize::try_from(e.targets[0].paths).unwrap(), e.paths.len());

    // Unassigned Mesh addresses are captured and unroutable; nothing is planned.
    let e = evaluate(
        &c,
        &inventory(),
        &BTreeMap::new(),
        PlanDestination::Address("100.64.0.200".parse().unwrap()),
        None,
    );
    assert_eq!(
        e.destination,
        EvaluatedDestination::Address {
            address: "100.64.0.200".into(),
            class: AddressClass::Mesh
        }
    );
    assert!(e.route.is_none());
    assert!(e.access.grants.is_empty());
    assert_eq!(
        e.outcome,
        EvaluationOutcome::Unavailable {
            reason: UnavailableReason::NoPath
        }
    );
}

#[test]
fn external_destinations_report_first_match_rule_sets_and_resolution_dependence() {
    let c = routed();
    let remote = remote_content();
    // A public address: the public rule decides before any profile.
    let e = evaluate(
        &c,
        &inventory(),
        &remote,
        PlanDestination::Address("8.8.8.8".parse().unwrap()),
        None,
    );
    let route = e.route.unwrap();
    assert_eq!(route.kind, RouteKind::Direct);
    assert_eq!(route.rule.as_deref(), Some("/routing/egress/rules/0"));
    assert_eq!(route.profile, None);
    assert_eq!(e.outcome, EvaluationOutcome::Direct);
    assert!(e.access.allowed);
    assert_eq!(e.access.grants.len(), 1);
    assert_eq!(e.access.grants[0].rule, "/access/2");
    assert!(!e.access.grants[0].resolution_dependent);

    // A name in the inline set via the profile; the remote name rule before it
    // did not match, and the public address rule was never a candidate.
    let e = evaluate(
        &c,
        &inventory(),
        &remote,
        PlanDestination::Domain("Www.Example.org.".into()),
        None,
    );
    assert_eq!(
        e.destination,
        EvaluatedDestination::Domain {
            name: "www.example.org".into()
        }
    );
    let route = e.route.unwrap();
    assert_eq!(route.kind, RouteKind::Exit);
    assert_eq!(
        route.rule.as_deref(),
        Some("/routing/egress/profiles/work/rules/1")
    );
    assert_eq!(route.profile.as_deref(), Some("work"));
    assert_eq!(
        route.rule_set,
        Some(RuleSetMatch {
            key: "names".into(),
            entry: "example.org".into()
        })
    );
    assert_eq!(route.targets, vec![node(4)]);
    assert_eq!(route.via, vec![node(3)]);
    assert_eq!(route.resolution_dependent, vec!["/routing/egress/rules/0"]);
    // Both grants cover the name: the suffix definitely, the Internet grant by
    // resolution; the rule is one grant, definite.
    assert!(e.access.allowed);
    assert_eq!(e.access.grants.len(), 1);
    assert!(!e.access.grants[0].resolution_dependent);
    assert_eq!(e.outcome, EvaluationOutcome::Permitted);
    assert_eq!(e.paths[0].nodes, vec![node(1), node(3), node(4)]);

    // A remote exact name; the public address rule is again resolution dependent.
    let e = evaluate(
        &c,
        &inventory(),
        &remote,
        PlanDestination::Domain("t.me".into()),
        None,
    );
    let route = e.route.unwrap();
    assert_eq!(
        route.rule.as_deref(),
        Some("/routing/egress/profiles/work/rules/0")
    );
    assert_eq!(
        route.rule_set,
        Some(RuleSetMatch {
            key: "remote".into(),
            entry: "t.me".into()
        })
    );
    // The Internet grant covers a name by address-set semantics, as validation
    // accepts it when declaring the exit rule.
    assert!(e.access.allowed);
    assert_eq!(e.outcome, EvaluationOutcome::Permitted);
    // With only a CIDR and an unrelated suffix, authorization depends on what
    // the name resolves to; paths are still planned so the person sees what
    // authorization would allow.
    let mut narrowed = c.clone();
    narrowed.access[2].dst = vec!["8.8.8.0/24".into(), "domain:example.org".into()];
    let e = evaluate(
        &narrowed,
        &inventory(),
        &remote,
        PlanDestination::Domain("t.me".into()),
        None,
    );
    assert_eq!(e.access.grants.len(), 1);
    assert!(e.access.grants[0].resolution_dependent);
    assert!(!e.access.allowed);
    assert_eq!(
        e.outcome,
        EvaluationOutcome::Unavailable {
            reason: UnavailableReason::Unauthorized
        }
    );
    assert!(!e.paths.is_empty());

    // A private address no rule names: profile fallback, which leaves it alone.
    let e = evaluate(
        &c,
        &inventory(),
        &remote,
        PlanDestination::Address("10.1.2.3".parse().unwrap()),
        None,
    );
    let route = e.route.unwrap();
    assert_eq!(route.kind, RouteKind::Unmanaged);
    assert_eq!(
        route.rule.as_deref(),
        Some("/routing/egress/profiles/work/fallback")
    );
    assert_eq!(
        route.resolution_dependent,
        vec![
            "/routing/egress/profiles/work/rules/0",
            "/routing/egress/profiles/work/rules/1"
        ]
    );
    // Only the domain grant could reach a private address, by resolution.
    assert_eq!(e.access.grants.len(), 1);
    assert!(e.access.grants[0].resolution_dependent);
    assert!(!e.access.allowed);
    assert_eq!(e.outcome, EvaluationOutcome::Unmanaged);

    // The public Internet beyond every rule, under the explicitly named mesh profile.
    let e = evaluate(
        &c,
        &inventory(),
        &remote,
        PlanDestination::Internet,
        Some("mesh"),
    );
    assert!(e.profile.requested);
    let route = e.route.unwrap();
    assert_eq!(route.kind, RouteKind::Unmanaged);
    assert_eq!(route.rule, None);
    assert_eq!(route.profile.as_deref(), Some("mesh"));
    assert!(e.access.allowed);

    // Without the remote content, the plan cannot judge and says so.
    let e = evaluate(
        &c,
        &inventory(),
        &BTreeMap::new(),
        PlanDestination::Domain("t.me".into()),
        None,
    );
    assert!(e.route.is_none());
    assert_eq!(
        e.outcome,
        EvaluationOutcome::Unavailable {
            reason: UnavailableReason::MissingDependency
        }
    );
}

#[test]
fn targets_explain_why_no_path_exists_and_inputs_are_checked() {
    let mut c = routed();
    let remote = remote_content();
    c.nodes.get_mut(&node(4)).unwrap().exit_enabled = false;
    let e = evaluate(
        &c,
        &inventory(),
        &remote,
        PlanDestination::Domain("a.example.org".into()),
        None,
    );
    assert_eq!(e.targets[0].problem, Some(TargetProblem::ExitDisabled));
    assert!(e.paths.is_empty());
    assert_eq!(
        e.outcome,
        EvaluationOutcome::Unavailable {
            reason: UnavailableReason::NoPath
        }
    );
    c.nodes.get_mut(&node(4)).unwrap().exit_enabled = true;
    let mut inv = inventory();
    inv.remove(&node(4));
    let e = evaluate(
        &c,
        &inv,
        &remote,
        PlanDestination::Domain("a.example.org".into()),
        None,
    );
    assert_eq!(e.targets[0].problem, Some(TargetProblem::NotEnrolled));
    // A topology block towards the exit leaves no path, whatever the relay.
    c.topology.push(TopologyRule {
        remark: String::new(),
        a: "*".into(),
        b: format!("node:{}", node(4)),
        action: TopologyAction::Block,
    });
    let e = evaluate(
        &c,
        &inventory(),
        &remote,
        PlanDestination::Domain("a.example.org".into()),
        None,
    );
    assert_eq!(e.targets[0].problem, Some(TargetProblem::NoPath));
    // A disabled source is reported as such even with a decided route.
    c.nodes.get_mut(&node(1)).unwrap().enabled = false;
    let e = evaluate(
        &c,
        &inventory(),
        &remote,
        PlanDestination::Address("8.8.8.8".parse().unwrap()),
        None,
    );
    assert_eq!(
        e.outcome,
        EvaluationOutcome::Unavailable {
            reason: UnavailableReason::InactiveNode
        }
    );
    let subjects = SubjectContext::default();
    let none = BTreeMap::new();
    let request = |destination, profile| PlanRequest {
        topology: Topology {
            configuration: &c,
            subjects: &subjects,
            inventory: &inv,
        },
        rule_sets: &remote,
        selected_profiles: &none,
        source: node(1),
        destination,
        profile,
    };
    assert_eq!(
        plan(request(PlanDestination::Node(node(9)), None)).unwrap_err(),
        CompileError::InvalidInput
    );
    assert_eq!(
        plan(request(PlanDestination::Domain("bad name".into()), None)).unwrap_err(),
        CompileError::InvalidInput
    );
    assert_eq!(
        plan(request(PlanDestination::Internet, Some("global:nope"))).unwrap_err(),
        CompileError::InvalidInput
    );
}
