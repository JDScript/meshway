//! An exit set naming its own source: that source's direct egress, decided
//! from the full configuration's exit eligibility, never from what its
//! projection happens to hold.
use super::*;
use std::net::IpAddr;

/// A public rule `1.2.4.0/24 → exit 4 via 3`: node 4 is the exit itself, so
/// it executes the rule as direct (partition, plan, DNS, preview); source 1
/// still leaves through 4 via 3. A backup set naming the source is direct too.
#[test]
fn an_exit_naming_its_source_is_that_sources_direct_egress() {
    let mut s = base(5);
    s.cfg.access = vec![access(
        &[sel(1), sel(4), sel(5)],
        &["autogroup:internet".into()],
        &["tcp:443"],
        false,
    )];
    s.cfg.routing.egress.rules = vec![
        egress_rule("1.2.4.0/24", exit_action(&[4], &[3])),
        egress_rule("1.2.5.0/24", exit_action(&[4, 5], &[])),
    ];
    let compiled = netmaps(&s);
    let compiler =
        |at: u8| LocalCompiler::new(&compiled.nodes[&node(at)].netmap, &Default::default());
    let decide = |at: u8, source: u8, destination: &str| {
        compiler(at)
            .decide(
                node(source),
                destination.parse().unwrap(),
                BusinessProtocol::Tcp,
                port(443),
            )
            .unwrap()
    };
    // The exit itself: direct, on its own projection and on any other.
    for at in [4, 1, 3] {
        assert_eq!(decide(at, 4, "1.2.4.8"), LocalDecision::Direct, "at {at}");
        assert_eq!(decide(at, 4, "1.2.5.8"), LocalDecision::Direct, "at {at}");
    }
    // Ordered: 5 stands after the eligible 4, so 5 exits through 4.
    let LocalDecision::Permitted(backup) = decide(5, 5, "1.2.5.8") else {
        panic!("5 must exit through its primary 4");
    };
    assert_eq!(
        backup.target,
        Target::Exit {
            nodes: vec![node(4)],
            via: vec![]
        }
    );
    // Other sources are unaffected.
    let LocalDecision::Permitted(through) = decide(1, 1, "1.2.4.8") else {
        panic!("source 1 must still exit through 4");
    };
    assert_eq!(
        through.target,
        Target::Exit {
            nodes: vec![node(4)],
            via: vec![node(3)]
        }
    );
    assert!(
        through
            .paths
            .iter()
            .all(|p| p.nodes.first() == Some(&node(1))
                && p.nodes.contains(&node(3))
                && p.nodes.last() == Some(&node(4)))
    );
    let LocalDecision::Permitted(backup) = decide(1, 1, "1.2.5.8") else {
        panic!("source 1 keeps the ordered exit set");
    };
    assert_eq!(
        backup.target,
        Target::Exit {
            nodes: vec![node(4), node(5)],
            via: vec![]
        }
    );
    // The exit captures nothing for its own rules; source 1 captures both.
    let own = compiler(4).capture().unwrap().unwrap();
    assert!(own.managed_ipv4.is_empty() && !own.default_capture);
    let source = compiler(1).capture().unwrap().unwrap();
    let managed = |plan: &EgressCapturePlan, address: &str| {
        let address: std::net::Ipv4Addr = address.parse().unwrap();
        plan.managed_ipv4
            .iter()
            .any(|(start, end)| *start <= address && address <= *end)
    };
    assert!(managed(&source, "1.2.4.8") && managed(&source, "1.2.5.8"));
    // The signed egress plan carries the executed actions.
    let plan4 = compiled.nodes[&node(4)].netmap.egress.as_ref().unwrap();
    assert!(
        plan4
            .rules
            .iter()
            .all(|rule| rule.action == EgressAction::Direct)
    );
    // A relay never carries the exit's own traffic as an exit circuit.
    assert!(
        compiler(3)
            .transit(
                node(4),
                &Target::Exit {
                    nodes: vec![node(4)],
                    via: vec![node(3)]
                }
            )
            .unwrap()
            .is_empty()
    );
    // DNS: pure IP rules never need an exit's answer, however many exits
    // they use: every source resolves locally and nothing is served.
    for source in [1, 4, 5] {
        assert_eq!(
            compiled.nodes[&node(source)].netmap.dns.egress,
            DnsPlan::NoExit,
            "source {source}"
        );
    }
    // With no eligible exit before it, the backup source is direct.
    let mut ineligible_primary = netmaps(&{
        let mut copy = base(5);
        copy.cfg = s.cfg.clone();
        copy.cfg.nodes.get_mut(&node(4)).unwrap().exit_enabled = false;
        copy
    });
    let compiled5 = ineligible_primary.nodes.remove(&node(5)).unwrap();
    assert_eq!(
        LocalCompiler::new(&compiled5.netmap, &Default::default())
            .decide(
                node(5),
                "1.2.5.8".parse().unwrap(),
                BusinessProtocol::Tcp,
                port(443)
            )
            .unwrap(),
        LocalDecision::Direct
    );
    // The management preview equals what the exit executes.
    for (destination, kind) in [
        (
            "1.2.4.8",
            meshway_configuration::evaluation::RouteKind::Direct,
        ),
        (
            "1.2.5.8",
            meshway_configuration::evaluation::RouteKind::Direct,
        ),
    ] {
        let preview = meshway_policy::compile::plan(PlanRequest {
            topology: Topology {
                configuration: &s.cfg,
                subjects: &s.subjects,
                inventory: &s.inventory,
            },
            rule_sets: &BTreeMap::new(),
            selected_profiles: &BTreeMap::new(),
            source: node(4),
            destination: PlanDestination::Address(destination.parse().unwrap()),
            profile: None,
        })
        .unwrap();
        assert_eq!(preview.route.as_ref().map(|route| route.kind), Some(kind));
        assert!(preview.route.unwrap().targets.is_empty());
        assert_eq!(
            preview.outcome,
            meshway_configuration::evaluation::EvaluationOutcome::Direct
        );
        assert!(preview.paths.is_empty());
        assert_eq!(preview.profile.capture, compiler_summary(&s, 4));
    }
    // A name rule through 4 makes 4 the resolver of every source that leaves
    // through it; 4 itself executes the rule as direct and resolves locally.
    s.cfg.routing.egress.rules.truncate(1);
    s.cfg
        .routing
        .egress
        .rules
        .push(domain_rule("example.com", exit_action(&[4], &[3])));
    let compiled = netmaps(&s);
    assert!(matches!(
        &compiled.nodes[&node(1)].netmap.dns.egress,
        DnsPlan::ViaExit { service } if service.exit == node(4)
    ));
    assert!(matches!(
        &compiled.nodes[&node(4)].netmap.dns.egress,
        DnsPlan::ServeExitResolver { services }
            if services.iter().map(|s| s.source).collect::<Vec<_>>() == [node(1), node(5)]
    ));
}

fn compiler_summary(s: &Scenario, at: u8) -> meshway_configuration::evaluation::CaptureSummary {
    let compiled = netmaps(s);
    LocalCompiler::new(&compiled.nodes[&node(at)].netmap, &Default::default())
        .capture_summary(node(at))
        .unwrap()
}

/// `1.2.5.0/24 → exit [4, 5]` for source 5: 4 is an eligible exit of the full
/// configuration, so 5 exits through it or fails closed — never direct — also
/// when topology isolates 4 or 4 holds no keys (not enrolled, or revoked) and
/// 5's projection therefore lacks 4. Only a configuration that makes 4
/// ineligible makes 5 direct. Node, Server preview and signed plan agree.
#[test]
fn a_self_naming_source_never_infers_direct_from_a_missing_exit() {
    let mut s = base(5);
    s.cfg.access = vec![access(
        &[sel(5)],
        &["autogroup:internet".into()],
        &["tcp:443"],
        false,
    )];
    s.cfg.routing.egress.rules = vec![egress_rule("1.2.5.0/24", exit_action(&[4, 5], &[]))];
    let isolated = {
        let mut s = s.clone();
        s.cfg.topology.push(TopologyRule {
            remark: String::new(),
            action: TopologyAction::Block,
            a: sel(4),
            b: "*".into(),
            transports: Input::Omitted,
            keepalive: false,
        });
        s
    };
    let unenrolled = {
        let mut s = s.clone();
        s.inventory.remove(&node(4));
        s
    };
    let ineligible = {
        let mut s = s.clone();
        s.cfg.nodes.get_mut(&node(4)).unwrap().enabled = false;
        s
    };
    let destination: std::net::Ipv4Addr = "1.2.5.8".parse().unwrap();
    for (name, s, direct) in [
        ("reachable", &s, false),
        ("isolated", &isolated, false),
        ("unenrolled", &unenrolled, false),
        ("ineligible", &ineligible, true),
    ] {
        let compiled = netmaps(s);
        let netmap = &compiled.nodes[&node(5)].netmap;
        if name != "reachable" {
            assert!(
                !netmap.projection.configuration.nodes.contains_key(&node(4))
                    || name == "ineligible",
                "{name}: 4 is outside 5's projection"
            );
        }
        assert_eq!(
            netmap.projection.eligible_exits,
            if direct { vec![] } else { vec![node(4)] },
            "{name}"
        );
        let compiler = LocalCompiler::new(netmap, &Default::default());
        let decision = compiler
            .decide(
                node(5),
                IpAddr::V4(destination),
                BusinessProtocol::Tcp,
                port(443),
            )
            .unwrap();
        let plan = netmap.egress.as_ref().unwrap();
        let captured = compiler
            .capture()
            .unwrap()
            .unwrap()
            .managed_ipv4
            .iter()
            .any(|(start, end)| *start <= destination && destination <= *end);
        let preview = meshway_policy::compile::plan(PlanRequest {
            topology: Topology {
                configuration: &s.cfg,
                subjects: &s.subjects,
                inventory: &s.inventory,
            },
            rule_sets: &BTreeMap::new(),
            selected_profiles: &BTreeMap::new(),
            source: node(5),
            destination: PlanDestination::Address(IpAddr::V4(destination)),
            profile: None,
        })
        .unwrap();
        if direct {
            assert_eq!(decision, LocalDecision::Direct, "{name}");
            assert_eq!(plan.rules[0].action, EgressAction::Direct, "{name}");
            assert!(!captured, "{name}");
            assert_eq!(
                preview.outcome,
                meshway_configuration::evaluation::EvaluationOutcome::Direct
            );
            continue;
        }
        assert_eq!(
            plan.rules[0].action,
            EgressAction::Exit {
                nodes: vec![node(4)],
                via: vec![]
            },
            "{name}"
        );
        assert!(captured, "{name}: the exit range stays captured");
        assert_ne!(
            preview.outcome,
            meshway_configuration::evaluation::EvaluationOutcome::Direct,
            "{name}"
        );
        match name {
            "reachable" => assert!(
                matches!(&decision, LocalDecision::Permitted(d) if d.target == Target::Exit { nodes: vec![node(4)], via: vec![] }),
                "{name}: {decision:?}"
            ),
            _ => assert!(
                matches!(decision, LocalDecision::Denied(_)),
                "{name}: fails closed, got {decision:?}"
            ),
        }
    }
}

/// `example.com → exit [6, 4, 5]` with 6 a disabled exit: source 5 executes
/// `[4]` (ineligible exits before it skipped, order kept), so its names have
/// one exit and the configuration is saved, not `dns_exit_plan_required`.
#[test]
fn ineligible_exits_before_a_self_naming_source_leave_its_dns_plan() {
    let mut s = base(6);
    let six = s.cfg.nodes.get_mut(&node(6)).unwrap();
    six.exit_enabled = true;
    six.enabled = false;
    s.cfg.access = vec![access(
        &[sel(5)],
        &["autogroup:internet".into()],
        &["tcp:443", "udp:53"],
        false,
    )];
    s.cfg.routing.egress.rules = vec![domain_rule("example.com", exit_action(&[6, 4, 5], &[]))];
    assert_eq!(
        dns_resolution(
            &s.cfg,
            Exits::Full(&s.cfg),
            &BTreeMap::new(),
            "mesh",
            node(5)
        )
        .unwrap(),
        DnsResolution::Exit {
            exit: node(4),
            via: vec![]
        }
    );
    let compiled = netmaps(&s);
    assert!(matches!(
        &compiled.nodes[&node(5)].netmap.dns.egress,
        DnsPlan::ViaExit { service } if service.exit == node(4)
    ));
    // 6 is ineligible: the projection only carries 4.
    assert_eq!(
        compiled.nodes[&node(5)].netmap.projection.eligible_exits,
        [node(4)]
    );
}
