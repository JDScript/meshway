//! Rule sets reach Nodes by digest: the same content decides the same way
//! wherever it is loaded, and a missing artifact fails closed.
use super::*;
use std::{net::IpAddr, sync::Arc};

fn set(domain_suffix: &[&str], ip_cidr: &[&str]) -> Arc<meshway_rules::RuleSet> {
    let own = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let (suffix, cidr) = (own(domain_suffix), own(ip_cidr));
    Arc::new(
        meshway_rules::RuleSet::from_inline(&meshway_rules::inline::Inline {
            domain_suffix: &suffix,
            ip_cidr: &cidr,
            ..Default::default()
        })
        .unwrap(),
    )
}

/// Source 1: `corp` addresses direct, everything else to exit 4 via 3.
fn scenario() -> Scenario {
    let mut s = base(4);
    s.cfg.access = vec![access(
        &[sel(1)],
        &["autogroup:internet".into()],
        &["tcp:443"],
        false,
    )];
    s.cfg.routing.egress.profiles.insert(
        "exit".into(),
        RoutingProfile {
            rules: vec![EgressRule {
                remark: String::new(),
                matcher: RouteMatch {
                    rule_sets: vec!["corp".into()],
                    ..Default::default()
                },
                action: RuleAction::Direct,
            }],
            fallback: RouteAction::Exit {
                nodes: vec![sel(4)],
                via: vec![sel(3)],
            },
        },
    );
    s.cfg.routing.egress.assignments.push(ProfileAssignment {
        target: vec![sel(1)],
        allowed: vec!["exit".into()],
        default: "exit".into(),
        locked: false,
    });
    s
}

fn compile(s: &Scenario, sets: &RuleSets) -> CompiledNetmaps {
    compile_netmap(NetmapInput {
        network: network(),
        revision: Revision::MIN,
        configuration: &s.cfg,
        subjects: &s.subjects,
        inventory: &s.inventory,
        rule_sets: sets,
        selected_profiles: &BTreeMap::new(),
        selection_versions: &s.selection_versions,
        previous: &BTreeMap::new(),
    })
    .unwrap()
}

fn decide(compiler: &LocalCompiler, destination: &str) -> LocalDecision {
    compiler
        .decide(
            node(1),
            destination.parse().unwrap(),
            BusinessProtocol::Tcp,
            port(443),
        )
        .unwrap()
}

#[test]
fn artifact_content_decides_like_the_server_and_a_missing_artifact_fails_closed() {
    let s = scenario();
    let corp = set(&[], &["9.9.9.0/24"]);
    let sets: RuleSets = [("corp".to_owned(), corp.clone())].into();
    let compiled = compile(&s, &sets);
    let netmap = &compiled.nodes[&node(1)].netmap;
    let reference = &netmap.projection.rule_sets["corp"];
    assert_eq!(reference.digest, corp.digest());
    assert_eq!(u64::from(reference.bytes), corp.as_bytes().len() as u64);
    // Definitions never reach the projection; only the reference does.
    assert!(netmap.projection.configuration.rule_sets.is_empty());

    // The same bytes, however they were obtained (converted from a remote
    // source here), are the same content and decide identically.
    let converted = meshway_rules::convert::convert(
        b"payload:\n  - 9.9.9.0/24\n",
        meshway_rules::convert::Format::MihomoIpcidr,
    )
    .unwrap()
    .artifact;
    assert_eq!(converted.digest(), corp.digest());
    let artifacts: Artifacts = [(converted.digest(), Arc::new(converted.open().unwrap()))].into();
    let held = LocalCompiler::new(netmap, &artifacts);
    assert!(held.missing().is_empty());
    assert!(Arc::ptr_eq(&held.effective().unwrap(), netmap));
    assert_eq!(Some(&held.egress(node(1)).unwrap()), netmap.egress.as_ref());
    assert_eq!(decide(&held, "9.9.9.9"), LocalDecision::Direct);
    assert!(matches!(
        decide(&held, "8.8.8.8"),
        LocalDecision::Permitted(_)
    ));
    let hit = held
        .egress_rule_for_ip(node(1), "9.9.9.9".parse().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(hit.rule, "/routing/egress/profiles/exit/rules/0");
    assert_eq!(hit.rule_set.unwrap().entry, "9.9.9.0/24");
    assert_eq!(
        held.egress_rule_for_ip(node(1), "8.8.8.8".parse().unwrap())
            .unwrap(),
        None
    );

    // Without the artifact: no rule after it is tried, nothing goes direct
    // and the capture keeps the whole range.
    let missing = LocalCompiler::new(netmap, &Artifacts::new());
    assert_eq!(missing.missing(), vec![corp.digest()]);
    for destination in ["9.9.9.9", "8.8.8.8"] {
        assert_eq!(
            decide(&missing, destination),
            LocalDecision::Denied(UnavailableReason::MissingDependency)
        );
    }
    assert_eq!(
        missing.egress_rule_for_ip(node(1), "9.9.9.9".parse().unwrap()),
        Err(CompileError::UnavailableDependency)
    );
    let effective = missing.effective().unwrap();
    let capture = &effective.egress.as_ref().unwrap().capture;
    assert_eq!(
        capture.unavailable,
        Some(UnavailableReason::MissingDependency)
    );
    assert_eq!(capture.managed_ipv4, ["0.0.0.0/0"]);
    assert_eq!(capture.sink_ipv6, ["::/0"]);
    assert!(capture.direct_ipv4.is_empty() && capture.unmanaged_ipv4.is_empty());
    assert!(effective.egress.as_ref().unwrap().routes.is_empty());
    assert_ne!(effective.egress, netmap.egress);
    assert!(
        effective
            .dns
            .resolvers
            .values()
            .all(|action| *action == ResolverAction::Rejected)
    );
    // An unrelated artifact does not stand in for the referenced one.
    let other: Artifacts = [(
        set(&[], &["9.9.9.0/24", "1.1.1.0/24"]).digest(),
        set(&[], &["9.9.9.0/24", "1.1.1.0/24"]),
    )]
    .into();
    assert_eq!(
        LocalCompiler::new(netmap, &other).missing(),
        vec![corp.digest()]
    );
}

#[test]
fn query_names_are_matched_without_dns_name_validation() {
    let mut s = base(4);
    s.cfg.routing.egress.profiles.insert(
        "local".into(),
        RoutingProfile {
            rules: vec![EgressRule {
                remark: String::new(),
                matcher: RouteMatch {
                    rule_sets: vec!["names".into()],
                    ..Default::default()
                },
                action: RuleAction::Direct,
            }],
            fallback: RouteAction::Unmanaged,
        },
    );
    s.cfg.routing.egress.assignments.push(ProfileAssignment {
        target: vec![sel(1)],
        allowed: vec!["local".into()],
        default: "local".into(),
        locked: false,
    });
    let names = {
        let conversion = meshway_rules::convert::convert(
            b"DOMAIN-SUFFIX,example.org\nDOMAIN-KEYWORD,track\n",
            meshway_rules::convert::Format::Classical,
        )
        .unwrap();
        Arc::new(conversion.artifact.open().unwrap())
    };
    let sets: RuleSets = [("names".to_owned(), names.clone())].into();
    let compiled = compile(&s, &sets);
    let artifacts: Artifacts = [(names.digest(), names)].into();
    let compiler = LocalCompiler::new(&compiled.nodes[&node(1)].netmap, &artifacts);
    let entry = |name: &str| {
        compiler
            .egress_rule_for_name(node(1), name)
            .unwrap()
            .and_then(|hit| hit.rule_set)
            .map(|m| m.entry)
    };
    assert_eq!(entry("_dmarc.Example.org."), Some("example.org".into()));
    assert_eq!(entry("example.org"), Some("example.org".into()));
    assert_eq!(entry("x_y.tracker.net"), Some("track".into()));
    assert_eq!(entry("other.net"), None);
    // An address is judged by its value only: names never match it.
    let address: IpAddr = "9.9.9.9".parse().unwrap();
    assert_eq!(compiler.egress_rule_for_ip(node(1), address).unwrap(), None);
}
