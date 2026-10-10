use super::*;
use meshway_policy::{
    PermitTransition,
    admission::{AdmissionError, Hop, Traffic, admit_forward},
    is_internet_address, permit_transition,
};
use std::net::IpAddr;
mod dns;

fn setup() -> (NetworkConfiguration, Demand, ExitInputs) {
    let mut cfg = configuration();
    cfg.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec!["autogroup:internet".into(), "10.20.0.0/16".into()],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    cfg.routing.egress.profiles.insert(
        "exit".into(),
        RoutingProfile {
            rules: vec![],
            fallback: RouteAction::Exit {
                nodes: vec![format!("node:{}", node(4))],
                via: vec![format!("node:{}", node(3))],
            },
        },
    );
    cfg.routing.egress.assignments.push(ProfileAssignment {
        target: vec![format!("node:{}", node(1))],
        allowed: vec!["exit".into(), "mesh".into()],
        default: "exit".into(),
        locked: false,
    });
    let mut d = demand(1, 1, 4);
    d.profile = "exit".into();
    d.target = BusinessTarget::Ip {
        address: ExecutionAddress("8.8.8.8".parse().unwrap()),
    };
    let runtime = ExitInputs {
        instances: BTreeMap::from([
            (node(1), node(1).to_string().parse().unwrap()),
            (node(4), node(4).to_string().parse().unwrap()),
        ]),
        instance_expires_at_ms: BTreeMap::from([(node(1), 120_000), (node(4), 120_000)]),
        ready_until_ms: BTreeMap::from([(node(4), 100_000)]),
        ..Default::default()
    };
    (cfg, d, runtime)
}

fn compiled(cfg: &NetworkConfiguration, d: &Demand, runtime: &ExitInputs) -> CompiledPublication {
    compile_with_exit(
        CompilationInput {
            network: node(1).to_string().parse().unwrap(),
            revision: Revision::MIN,
            publication: node(2).to_string().parse().unwrap(),
            configuration: cfg,
            subjects: &SubjectContext::default(),
            inventory: &inventory(),
            demands: &BTreeMap::from([(d.id, d.clone())]),
            dns: &[],
            rule_sets: &BTreeMap::new(),
            selected_profiles: &BTreeMap::new(),
            now_ms: 1000,
        },
        &BTreeMap::new(),
        runtime,
    )
    .unwrap()
}

fn decision(publication: &CompiledPublication) -> &DemandDecision {
    &publication.nodes[&node(1)].configuration.decisions[&request(1)]
}
fn permit(publication: &CompiledPublication) -> &PathPermit {
    let DemandDecision::Permitted { permit } = decision(publication) else {
        panic!("{:?}", decision(publication));
    };
    permit
}
fn rule(prefix: &str, exit: bool) -> EgressRule {
    EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            ip_cidr: vec![prefix.into()],
            domain: vec![],
            domain_suffix: vec![],
            rule_sets: vec![],
        },
        action: if exit {
            RuleAction::Exit {
                nodes: vec![format!("node:{}", node(4))],
                via: vec![],
            }
        } else {
            RuleAction::Direct
        },
    }
}
fn contains(nets: &[String], ip: IpAddr) -> bool {
    nets.iter()
        .any(|net| net.parse::<ipnet::IpNet>().unwrap().contains(&ip))
}

#[test]
fn first_match_cidr_partition_is_disjoint_and_equivalent_to_rule_order() {
    let (mut cfg, d, runtime) = setup();
    for reverse in [false, true] {
        let mut rules = vec![
            rule("8.0.0.0/8", true),
            rule("8.8.0.0/16", false),
            rule("10.0.0.0/8", false),
            rule("10.20.0.0/16", true),
        ];
        if reverse {
            rules.reverse();
        }
        cfg.routing.egress.rules = rules.clone();
        let result = compiled(&cfg, &d, &runtime);
        let plan = &result.nodes[&node(1)].configuration.egress_capture;
        assert!(plan.default_capture);
        let all: Vec<ipnet::IpNet> = plan
            .managed_ipv4
            .iter()
            .chain(&plan.direct_ipv4)
            .chain(&plan.unmanaged_ipv4)
            .map(|net| net.parse().unwrap())
            .collect();
        assert!(all.len() <= MAX_CAPTURE_PREFIXES);
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert!(!a.contains(b) && !b.contains(a));
            }
        }
        for a in [0, 7, 8, 9, 10, 11, 127, 255] {
            for b in 0..=255 {
                let ip: IpAddr = std::net::Ipv4Addr::new(a, b, 1, 1).into();
                let expected_exit = rules
                    .iter()
                    .find(|rule| contains(&rule.matcher.ip_cidr, ip))
                    .is_none_or(|rule| matches!(rule.action, RuleAction::Exit { .. }));
                assert_eq!(
                    contains(&plan.managed_ipv4, ip),
                    expected_exit,
                    "{ip}, reverse={reverse}"
                );
                assert_eq!(contains(&plan.direct_ipv4, ip), !expected_exit);
            }
        }
        assert_eq!(plan.sink_ipv6, ["::/0"]);
    }
}

#[test]
fn public_boundary_excludes_special_use_and_mapped_ipv6() {
    for ip in [
        "1.1.1.1",
        "8.8.8.8",
        "100.63.255.255",
        "100.128.0.0",
        "2001:4860:4860::8888",
        "2606:4700:4700::1111",
    ] {
        assert!(is_internet_address(ip.parse().unwrap()), "{ip}");
    }
    for ip in [
        "0.0.0.0",
        "0.1.2.3",
        "10.0.0.1",
        "100.64.0.0",
        "100.127.255.255",
        "127.0.0.1",
        "169.254.0.1",
        "172.16.0.1",
        "192.168.1.1",
        "192.0.2.1",
        "198.18.0.1",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "239.255.255.255",
        "240.0.0.1",
        "255.255.255.255",
        "::",
        "::1",
        "fc00::1",
        "fd00::1",
        "fe80::1",
        "ff02::1",
        "2001:db8::1",
        "2002::1",
        "::ffff:8.8.8.8",
    ] {
        assert!(!is_internet_address(ip.parse().unwrap()), "{ip}");
    }
}

#[test]
fn exit_joint_permission_and_original_source_path_are_enforced() {
    let (mut cfg, mut d, runtime) = setup();
    let result = compiled(&cfg, &d, &runtime);
    let p = permit(&result);
    assert!(p.paths.iter().all(|path| path.nodes.starts_with(&[node(1)])
        && path.nodes.contains(&node(3))
        && path.nodes.last() == Some(&node(4))));
    assert_eq!(
        p.exit.as_ref().unwrap().destination,
        ExitDestinationScope::Internet
    );
    let traffic = Traffic {
        source: "100.64.0.1".parse().unwrap(),
        destination: "8.8.8.8".parse().unwrap(),
        protocol: BusinessProtocol::Tcp,
        source_port: Some(1234.try_into().unwrap()),
        destination_port: d.port,
    };
    let path = &p.paths[0].nodes;
    let target = &result.nodes[&node(4)].configuration;
    let previous = path[path.len() - 2];
    assert_eq!(
        admit_forward(target, d.id, path, node(1), Some(previous), &traffic, 1001),
        Ok(Hop::Exit)
    );
    let mut forged = traffic;
    forged.source = "100.64.0.3".parse().unwrap();
    assert_eq!(
        admit_forward(target, d.id, path, node(1), Some(previous), &forged, 1001),
        Err(AdmissionError::Traffic)
    );
    d.target = BusinessTarget::Ip {
        address: ExecutionAddress("10.20.1.1".parse().unwrap()),
    };
    assert!(matches!(
        permit(&compiled(&cfg, &d, &runtime))
            .exit
            .as_ref()
            .unwrap()
            .destination,
        ExitDestinationScope::Cidr { .. }
    ));
    cfg.access[0].dst = vec!["autogroup:internet".into()];
    assert_eq!(
        decision(&compiled(&cfg, &d, &runtime)),
        &DemandDecision::Unavailable {
            reason: UnavailableReason::Unauthorized
        }
    );
    cfg.access[0].dst.push("10.20.0.0/16".into());
    cfg.nodes.get_mut(&node(4)).unwrap().exit_enabled = false;
    assert_eq!(
        decision(&compiled(&cfg, &d, &runtime)),
        &DemandDecision::Unavailable {
            reason: UnavailableReason::NoPath
        }
    );
}

#[test]
fn ipv6_exit_is_rejected_and_captured_without_source_ipv6() {
    let (cfg, mut d, runtime) = setup();
    d.target = BusinessTarget::Ip {
        address: ExecutionAddress("2606:4700:4700::1111".parse().unwrap()),
    };
    let result = compiled(&cfg, &d, &runtime);
    assert_eq!(
        decision(&result),
        &DemandDecision::Unavailable {
            reason: UnavailableReason::UnsupportedFamily
        }
    );
    assert_eq!(
        result.nodes[&node(1)]
            .configuration
            .egress_capture
            .sink_ipv6,
        ["::/0"]
    );
}

#[test]
fn missing_capability_withdraws_permit_but_keeps_role_and_capture() {
    let (cfg, d, mut runtime) = setup();
    let old = compiled(&cfg, &d, &runtime);
    assert!(!old.nodes[&node(4)].configuration.permits.is_empty());
    runtime.ready_until_ms.clear();
    let next = compiled(&cfg, &d, &runtime);
    assert_eq!(
        decision(&next),
        &DemandDecision::Unavailable {
            reason: UnavailableReason::ExitUnavailable
        }
    );
    assert!(next.nodes[&node(4)].configuration.permits.is_empty());
    assert!(next.nodes[&node(4)].configuration.exit_role.is_some());
    assert_eq!(
        old.nodes[&node(1)].configuration.egress_capture,
        next.nodes[&node(1)].configuration.egress_capture
    );
    let delta = configuration_delta(&old.nodes[&node(4)], &next.nodes[&node(4)]).unwrap();
    assert_eq!(
        apply_configuration_delta(&old.nodes[&node(4)], delta).unwrap(),
        next.nodes[&node(4)]
    );
}

#[test]
fn unreachable_primary_capability_deadline_cannot_shorten_a_valid_backup_permit() {
    let (mut cfg, d, mut runtime) = setup();
    cfg.nodes.get_mut(&node(2)).unwrap().exit_enabled = true;
    cfg.routing
        .egress
        .profiles
        .get_mut("exit")
        .unwrap()
        .fallback = RouteAction::Exit {
        nodes: vec![format!("node:{}", node(2)), format!("node:{}", node(4))],
        via: vec![format!("node:{}", node(3))],
    };
    cfg.topology.push(TopologyRule {
        remark: String::new(),
        action: TopologyAction::Block,
        a: format!("node:{}", node(2)),
        b: "*".into(),
        transports: Input::Omitted,
        keepalive: false,
    });
    runtime
        .instances
        .insert(node(2), node(2).to_string().parse().unwrap());
    runtime.ready_until_ms.insert(node(2), 1001);
    let result = compiled(&cfg, &d, &runtime);
    let permit = permit(&result);
    assert_eq!(permit.expires_at_ms, 100_000);
    assert!(!permit.exit.as_ref().unwrap().roles.contains_key(&node(2)));
    assert!(permit.exit.as_ref().unwrap().roles.contains_key(&node(4)));
}

#[test]
fn path_evaluation_projects_fresh_exit_failure_without_direct_fallback() {
    use meshway_configuration::evaluation::{EvaluationOutcome, TargetProblem};
    let (cfg, _, mut runtime) = setup();
    runtime.ready_until_ms.clear();
    runtime.unavailable_reasons.insert(
        node(4),
        meshway_model::exit::ExitUnavailableReason::IpForwardDisabled,
    );
    let result = plan_with_exit(
        PlanRequest {
            topology: Topology {
                configuration: &cfg,
                subjects: &SubjectContext::default(),
                inventory: &inventory(),
            },
            rule_sets: &BTreeMap::new(),
            selected_profiles: &BTreeMap::new(),
            source: node(1),
            destination: PlanDestination::Address("8.8.8.8".parse().unwrap()),
            profile: None,
        },
        &runtime,
        1000,
    )
    .unwrap();
    assert_eq!(
        result.outcome,
        EvaluationOutcome::Unavailable {
            reason: UnavailableReason::ExitUnavailable
        }
    );
    assert_eq!(
        result.targets[0].problem,
        Some(TargetProblem::ExitUnavailable)
    );
    assert_eq!(
        result.targets[0].exit_unavailable_reason,
        Some(meshway_model::exit::ExitUnavailableReason::IpForwardDisabled)
    );
    assert!(result.paths.is_empty());
}

#[test]
fn capture_partition_budget_fails_closed_and_does_not_publish_partial_exceptions() {
    let (mut cfg, d, runtime) = setup();
    let mut fragmented = rule("8.8.8.8/32", false);
    fragmented.matcher.ip_cidr = (0..1024)
        .map(|i| format!("8.{}.{}.1/32", i / 256, i % 256))
        .collect();
    cfg.routing.egress.rules.push(fragmented);
    let result = compiled(&cfg, &d, &runtime);
    let capture = &result.nodes[&node(1)].configuration.egress_capture;
    assert_eq!(capture.unavailable, Some(UnavailableReason::RoutingLimit));
    assert_eq!(capture.managed_ipv4, ["0.0.0.0/0"]);
    assert!(capture.direct_ipv4.is_empty());
    assert_eq!(capture.sink_ipv6, ["::/0"]);
}

#[test]
fn renewal_preserves_authority_but_selection_generation_and_expiry_end_flow() {
    let (cfg, mut d, mut runtime) = setup();
    let first = compiled(&cfg, &d, &runtime);
    let old = permit(&first);
    runtime.ready_until_ms.insert(node(4), 200_000);
    d.expires_at_ms = 200_000;
    let renewal = compiled(&cfg, &d, &runtime);
    assert_eq!(
        permit_transition(old, permit(&renewal), 1001).unwrap(),
        PermitTransition::Renewal
    );
    assert_eq!(
        permit_transition(old, permit(&renewal), 100_000).unwrap(),
        PermitTransition::Replacement
    );
    runtime.selection_generations.insert(node(1), 2);
    let switched_back = compiled(&cfg, &d, &runtime);
    assert_eq!(
        permit_transition(old, permit(&switched_back), 1001).unwrap(),
        PermitTransition::Replacement
    );
}

#[test]
fn unknown_domain_capture_is_fail_closed_not_an_implicit_direct_route() {
    let (mut cfg, d, runtime) = setup();
    let mut unknown = rule("8.8.0.0/16", false);
    unknown.matcher.domain.push("example.com".into());
    cfg.routing.egress.rules.push(unknown);
    let result = compiled(&cfg, &d, &runtime);
    let capture = &result.nodes[&node(1)].configuration.egress_capture;
    assert_eq!(capture.unavailable, Some(UnavailableReason::EgressConflict));
    assert_eq!(capture.managed_ipv4, ["0.0.0.0/0"]);
    assert!(capture.direct_ipv4.is_empty());
    assert!(matches!(
        decision(&result),
        DemandDecision::Unavailable { .. }
    ));
}
