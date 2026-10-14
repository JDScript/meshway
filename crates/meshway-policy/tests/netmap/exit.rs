//! Regressions of the per-demand exit contract under projection compilation.
//! Readiness is a dynamic fact and never an input here.
use super::*;
use meshway_policy::{
    admission::{AdmissionError, Hop, Traffic, netmap::admit_forward},
    dns_route::{ResolverRouteError, netmap_dns_service, netmap_resolver},
};
use std::net::IpAddr;

/// Source 1 may reach the Internet and 10.20/16 on tcp:443 through exit 4 via 3.
pub(super) fn setup() -> Scenario {
    let mut s = base(4);
    s.cfg.access = vec![access(
        &[sel(1)],
        &["autogroup:internet".into(), "10.20.0.0/16".into()],
        &["tcp:443"],
        false,
    )];
    s.cfg.routing.egress.profiles.insert(
        "exit".into(),
        RoutingProfile {
            rules: vec![],
            fallback: RouteAction::Exit {
                nodes: vec![sel(4)],
                via: vec![sel(3)],
            },
        },
    );
    s.cfg.routing.egress.assignments.push(ProfileAssignment {
        target: vec![sel(1)],
        allowed: vec!["exit".into(), "mesh".into()],
        default: "exit".into(),
        locked: false,
    });
    s
}

#[test]
fn owning_compilers_reconstruct_the_same_dns_service_at_source_relay_and_exit() {
    let compiled = netmaps(&setup());
    let DnsPlan::ViaExit { service } = &compiled.nodes[&node(1)].netmap.dns.egress else {
        panic!("source must use its exit resolver")
    };
    let service = *service.clone();
    let compilers: Vec<_> = service
        .path
        .nodes
        .iter()
        .map(|node| {
            meshway_policy::compile::netmap::LocalCompiler::new(
                &compiled.nodes[node].netmap,
                &Default::default(),
            )
        })
        .collect();
    drop(compiled);
    for compiler in compilers {
        assert_eq!(
            compiler.dns_service(node(1)).unwrap(),
            Some(service.clone())
        );
        assert_eq!(compiler.dns_service(node(2)).unwrap(), None);
    }
}

fn tcp443(destination: &str) -> Traffic {
    Traffic {
        source: "100.64.0.1".parse().unwrap(),
        destination: destination.parse().unwrap(),
        protocol: BusinessProtocol::Tcp,
        source_port: port(1234),
        destination_port: port(443),
    }
}

/// Decision for source 1, computed on `at`'s own projection.
fn decide_at(compiled: &CompiledNetmaps, at: u8, destination: &str) -> LocalDecision {
    compile_local(
        &compiled.nodes[&node(at)].netmap,
        &Default::default(),
        node(1),
        destination.parse().unwrap(),
        (BusinessProtocol::Tcp, port(443)),
    )
    .unwrap()
}

fn select(compiled: &CompiledNetmaps, destination: &str) -> LocalDecision {
    decide_at(compiled, 1, destination)
}

fn permitted(compiled: &CompiledNetmaps, destination: &str) -> Decision {
    match select(compiled, destination) {
        LocalDecision::Permitted(decision) => decision,
        other => panic!("{destination}: {other:?}"),
    }
}

fn terminals(decision: &Decision) -> Vec<NodeId> {
    decision
        .paths
        .iter()
        .map(|p| *p.nodes.last().unwrap())
        .collect()
}

fn plan(compiled: &CompiledNetmaps) -> &EgressPlan {
    compiled.nodes[&node(1)].netmap.egress.as_ref().unwrap()
}

/// The capture boundary source 1 installs from its own netmap.
fn capture(compiled: &CompiledNetmaps) -> EgressCapturePlan {
    meshway_policy::compile::netmap::LocalCompiler::new(
        &compiled.nodes[&node(1)].netmap,
        &Default::default(),
    )
    .capture()
    .unwrap()
    .unwrap()
}

fn sink_all() -> Vec<(std::net::Ipv6Addr, std::net::Ipv6Addr)> {
    vec![(
        std::net::Ipv6Addr::UNSPECIFIED,
        std::net::Ipv6Addr::from(u128::MAX),
    )]
}

#[test]
fn routes_partition_is_disjoint_and_follows_rule_order() {
    let mut s = setup();
    for reverse in [false, true] {
        let mut rules = vec![
            egress_rule("8.0.0.0/8", exit_action(&[4], &[])),
            egress_rule("8.8.0.0/16", RuleAction::Direct),
            egress_rule("10.0.0.0/8", RuleAction::Direct),
            egress_rule("10.20.0.0/16", exit_action(&[4], &[])),
        ];
        if reverse {
            rules.reverse();
        }
        s.cfg.routing.egress.rules = rules.clone();
        let compiled = netmaps(&s);
        let compiler = meshway_policy::compile::netmap::LocalCompiler::new(
            &compiled.nodes[&node(1)].netmap,
            &Default::default(),
        );
        let partition = compiler.partition(node(1)).unwrap().unwrap();
        let ranges: Vec<_> = partition.ipv4().collect();
        assert_eq!(
            ranges.first().unwrap().0.start(),
            &std::net::Ipv4Addr::UNSPECIFIED
        );
        assert_eq!(
            ranges.last().unwrap().0.end(),
            &std::net::Ipv4Addr::BROADCAST
        );
        for pair in ranges.windows(2) {
            assert_eq!(
                u32::from(*pair[0].0.end()) + 1,
                u32::from(*pair[1].0.start())
            );
            assert_ne!(pair[0].1, pair[1].1, "adjacent ranges share an action");
        }
        for a in [0u8, 8, 10, 11, 127] {
            for b in [0u8, 8, 20, 21, 255] {
                let ip: IpAddr = std::net::Ipv4Addr::new(a, b, 1, 1).into();
                let expected_exit = rules
                    .iter()
                    .find(|rule| {
                        rule.matcher.ip_cidr[0]
                            .parse::<ipnet::IpNet>()
                            .unwrap()
                            .contains(&ip)
                    })
                    .is_none_or(|rule| matches!(rule.action, RuleAction::Exit { .. }));
                assert_eq!(
                    matches!(partition.action(ip), EgressAction::Exit { .. }),
                    expected_exit,
                    "{ip} reverse={reverse}"
                );
            }
        }
        assert_eq!(capture(&compiled).sink_ipv6, sink_all());
        assert!(plan(&compiled).unavailable.is_none());
    }
}

#[test]
fn exit_decision_enforces_scope_source_and_original_path() {
    let mut s = setup();
    let compiled = netmaps(&s);
    let decision = permitted(&compiled, "8.8.8.8");
    assert_eq!(
        decision.target,
        Target::Exit {
            nodes: vec![node(4)],
            via: vec![node(3)]
        }
    );
    assert!(
        decision
            .paths
            .iter()
            .all(|p| p.nodes.first() == Some(&node(1))
                && p.nodes.contains(&node(3))
                && p.nodes.last() == Some(&node(4)))
    );
    let LocalDecision::Permitted(at_exit) = decide_at(&compiled, 4, "8.8.8.8") else {
        panic!()
    };
    assert_eq!(at_exit, decision);
    let path = &decision.paths[0].nodes;
    let exit = &compiled.nodes[&node(4)];
    let leased = &exit.netmap;
    let previous = Some(path[path.len() - 2]);
    assert_eq!(
        admit_forward(
            leased,
            &at_exit,
            path,
            node(1),
            previous,
            &tcp443("8.8.8.8"),
        ),
        Ok(Hop::Exit)
    );
    let mut forged = tcp443("8.8.8.8");
    forged.source = "100.64.0.3".parse().unwrap();
    assert_eq!(
        admit_forward(leased, &at_exit, path, node(1), previous, &forged),
        Err(AdmissionError::Traffic)
    );
    // A decision covers exactly its destination; another one decides anew.
    assert_eq!(
        admit_forward(
            leased,
            &at_exit,
            path,
            node(1),
            previous,
            &tcp443("1.1.1.1"),
        ),
        Err(AdmissionError::Traffic)
    );
    assert!(matches!(
        select(&compiled, "10.20.1.1"),
        LocalDecision::Permitted(_)
    ));
    s.cfg.access[0].dst = vec!["autogroup:internet".into()];
    assert_eq!(
        select(&netmaps(&s), "10.20.1.1"),
        LocalDecision::Denied(UnavailableReason::Unauthorized)
    );
    s.cfg.access[0].dst.push("10.20.0.0/16".into());
    s.cfg.nodes.get_mut(&node(4)).unwrap().exit_enabled = false;
    let disabled = netmaps(&s);
    assert_eq!(
        select(&disabled, "8.8.8.8"),
        LocalDecision::Denied(UnavailableReason::NoPath)
    );
    assert!(disabled.nodes[&node(4)].netmap.exit_role.is_none());
}

#[test]
fn ipv6_and_mesh_pool_destinations_never_leave_through_an_exit() {
    let compiled = netmaps(&setup());
    assert_eq!(
        select(&compiled, "2606:4700:4700::1111"),
        LocalDecision::Denied(UnavailableReason::UnsupportedFamily)
    );
    assert_eq!(
        select(&compiled, "100.64.0.200"),
        LocalDecision::Denied(UnavailableReason::NoPath)
    );
    assert_eq!(
        select(&compiled, "127.0.0.1"),
        LocalDecision::Denied(UnavailableReason::Unauthorized)
    );
    assert_eq!(capture(&compiled).sink_ipv6, sink_all());
}

#[test]
fn missing_capability_withdraws_role_and_decision_but_keeps_capture() {
    let mut s = setup();
    let ready = netmaps(&s);
    s.inventory
        .get_mut(&node(4))
        .unwrap()
        .capabilities
        .remove("feature:exit-ipv4");
    let incapable = netmaps(&s);
    assert!(incapable.nodes[&node(4)].netmap.exit_role.is_none());
    assert_eq!(
        select(&incapable, "8.8.8.8"),
        LocalDecision::Denied(UnavailableReason::ExitUnavailable)
    );
    assert_eq!(plan(&ready), plan(&incapable));
}

#[test]
fn ordered_backup_exits_and_unreachable_ones() {
    let mut s = setup();
    s.cfg.nodes.get_mut(&node(2)).unwrap().exit_enabled = true;
    s.inventory.insert(node(2), trusted(2, true, true));
    s.cfg
        .routing
        .egress
        .profiles
        .get_mut("exit")
        .unwrap()
        .fallback = RouteAction::Exit {
        nodes: vec![sel(2), sel(4)],
        via: vec![sel(3)],
    };
    let both = permitted(&netmaps(&s), "8.8.8.8");
    let mut ends = terminals(&both);
    ends.dedup();
    assert_eq!(ends, [node(2), node(4)]);
    s.cfg.topology.push(TopologyRule {
        remark: String::new(),
        action: TopologyAction::Block,
        a: sel(2),
        b: "*".into(),
        transports: Input::Omitted,
        keepalive: false,
    });
    let backup = permitted(&netmaps(&s), "8.8.8.8");
    assert!(terminals(&backup).iter().all(|t| *t == node(4)));
}

#[test]
fn partition_limit_or_unknown_domain_fails_closed_without_routes() {
    let mut s = setup();
    // 20,000 separate /32 direct exceptions inside the exit fallback: more
    // managed ranges than a Node installs.
    let mut fragmented = egress_rule("8.8.8.8/32", RuleAction::Direct);
    fragmented.matcher.ip_cidr = (0..20_000u32)
        .map(|i| format!("{}/32", std::net::Ipv4Addr::from(0x0b00_0000 + i * 2)))
        .collect();
    s.cfg.routing.egress.rules.push(fragmented);
    let limited = netmaps(&s);
    assert_eq!(
        plan(&limited).unavailable,
        Some(UnavailableReason::RoutingLimit)
    );
    let boundary = capture(&limited);
    assert_eq!(boundary.unavailable, Some(UnavailableReason::RoutingLimit));
    assert_eq!(
        boundary.managed_ipv4,
        [(
            std::net::Ipv4Addr::UNSPECIFIED,
            std::net::Ipv4Addr::BROADCAST
        )]
    );
    assert_eq!(boundary.sink_ipv6, sink_all());
    // The Node's diagnostic reports the counts that exceed its limits.
    let summary = meshway_policy::compile::netmap::LocalCompiler::new(
        &limited.nodes[&node(1)].netmap,
        &Default::default(),
    )
    .capture_summary(node(1))
    .unwrap();
    assert_eq!(summary.unavailable, Some(UnavailableReason::RoutingLimit));
    assert_eq!(summary.managed_ipv4, 20_001);
    assert!(summary.routes > 20_000);
    assert_eq!(
        select(&limited, "8.8.8.8"),
        LocalDecision::Denied(UnavailableReason::RoutingLimit)
    );
    // The same number of exceptions stays within the limit as ranges once
    // they are contiguous.
    s.cfg
        .routing
        .egress
        .rules
        .last_mut()
        .unwrap()
        .matcher
        .ip_cidr = (0..20_000u32)
        .map(|i| format!("{}/32", std::net::Ipv4Addr::from(0x0b00_0000 + i)))
        .collect();
    let merged = netmaps(&s);
    assert!(plan(&merged).unavailable.is_none());
    assert_eq!(capture(&merged).managed_ipv4.len(), 2);
    s.cfg.routing.egress.rules.clear();
    // A rule's names own no address: only its IP part partitions, and the
    // names decide nothing without a trusted association.
    let mut mixed = egress_rule("8.8.0.0/16", RuleAction::Direct);
    mixed.matcher.domain.push("example.com".into());
    s.cfg.routing.egress.rules.push(mixed);
    let partitioned = netmaps(&s);
    assert_eq!(plan(&partitioned).unavailable, None);
    assert_eq!(select(&partitioned, "8.8.8.8"), LocalDecision::Direct);
    assert!(matches!(
        select(&partitioned, "9.9.9.9"),
        LocalDecision::Permitted(_)
    ));
}

#[test]
fn selection_identity_changes_on_switch_and_survives_unrelated_revisions() {
    let mut s = setup();
    let first = netmaps(&s);
    let selection = |c: &CompiledNetmaps| permitted(c, "8.8.8.8").selection.unwrap();
    assert_eq!(selection(&first), plan(&first).generation);
    let renewed = compile_with(&s, &previous(&first), Revision::new(7).unwrap());
    assert_eq!(selection(&first), selection(&renewed));
    assert_eq!(first.nodes, renewed.nodes);
    s.selection_versions
        .insert(node(1), meshway_model::Version::new(2).unwrap());
    let switched = netmaps(&s);
    assert_ne!(selection(&first), selection(&switched));
    // The exit recomputes the source's selection from its own projection.
    let LocalDecision::Permitted(at_exit) = decide_at(&switched, 4, "8.8.8.8") else {
        panic!()
    };
    assert_eq!(at_exit.selection, Some(selection(&switched)));
}

#[test]
fn resolver_decisions_follow_the_egress_partition() {
    let mut s = setup();
    let server = |address: &str| DnsServer {
        transport: DnsTransport::Udp,
        server: address.parse().unwrap(),
        server_port: 53.try_into().unwrap(),
        use_with_exit: false,
    };
    s.cfg.routing.egress.rules = vec![egress_rule("9.9.9.0/24", RuleAction::Direct)];
    s.cfg.dns.servers = BTreeMap::from([
        ("direct".into(), server("9.9.9.9")),
        ("exit".into(), server("1.1.1.1")),
        ("mesh".into(), server("100.64.0.2")),
    ]);
    let compiled = netmaps(&s);
    let source = &compiled.nodes[&node(1)].netmap;
    assert_eq!(
        netmap_resolver(source, "direct"),
        Ok(ResolverAction::Direct)
    );
    assert_eq!(netmap_resolver(source, "exit"), Ok(ResolverAction::Exit));
    assert_eq!(
        netmap_resolver(source, "mesh"),
        Ok(ResolverAction::Rejected)
    );
    assert_eq!(
        netmap_resolver(source, "absent"),
        Err(ResolverRouteError::Missing)
    );
    assert_eq!(
        netmap_resolver(&compiled.nodes[&node(2)].netmap, "exit"),
        Ok(ResolverAction::Unmanaged)
    );
}

#[test]
fn single_exit_dns_service_is_shared_by_source_and_resolver() {
    let mut s = setup();
    let compiled = netmaps(&s);
    let source = &compiled.nodes[&node(1)];
    let DnsPlan::ViaExit { service } = &source.netmap.dns.egress else {
        panic!("{:?}", source.netmap.dns.egress);
    };
    assert_eq!(service.generation, plan(&compiled).generation);
    assert!(service.path.nodes.contains(&node(3)));
    let resolver = &compiled.nodes[&node(4)];
    let DnsPlan::ServeExitResolver { services } = &resolver.netmap.dns.egress else {
        panic!();
    };
    assert_eq!(services, &[service.as_ref().clone()]);
    for compiled_netmap in [source, resolver] {
        let netmap = &compiled_netmap.netmap;
        for transport in [DnsTransport::Udp, DnsTransport::Tcp] {
            let check = |source, exit, path: &[NodeId], _: u64| {
                netmap_dns_service(netmap, service, source, exit, path, transport)
            };
            assert_eq!(check(node(1), node(4), &service.path.nodes, 1), Ok(()));
            assert_eq!(
                check(node(2), node(4), &service.path.nodes, 1),
                Err(ResolverRouteError::Binding)
            );
            assert_eq!(
                check(node(1), node(3), &service.path.nodes, 1),
                Err(ResolverRouteError::Binding)
            );
            assert_eq!(
                check(node(1), node(4), &[node(1), node(4)], 1),
                Err(ResolverRouteError::Binding)
            );
        }
    }
    let unavailable = |s: &Scenario| netmaps(s).nodes[&node(1)].netmap.dns.egress.clone();
    s.cfg.nodes.get_mut(&node(4)).unwrap().exit_enabled = false;
    assert_eq!(
        unavailable(&s),
        DnsPlan::Unavailable {
            reason: DnsEgressUnavailable::ExitUnavailable
        }
    );
    s.cfg.nodes.get_mut(&node(4)).unwrap().exit_enabled = true;
    s.cfg.access.clear();
    assert_eq!(
        unavailable(&s),
        DnsPlan::Unavailable {
            reason: DnsEgressUnavailable::Unauthorized
        }
    );
    s.cfg.access = setup().cfg.access;
    s.cfg.nodes.get_mut(&node(3)).unwrap().enabled = false;
    assert_eq!(
        unavailable(&s),
        DnsPlan::Unavailable {
            reason: DnsEgressUnavailable::NoPath
        }
    );
}
