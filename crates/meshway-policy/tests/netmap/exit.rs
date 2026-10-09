//! Regressions of the per-demand exit contract under the netmap compiler.
//! Readiness is a dynamic fact and never an input here.
use super::*;
use meshway_policy::{
    admission::{
        AdmissionError, Hop, Traffic,
        netmap::{Destination, Leased, admit_forward, classify},
    },
    dns_route::{ResolverRouteError, netmap_dns_service, netmap_resolver},
};
use std::net::IpAddr;

/// Source 1 may reach the Internet and 10.20/16 on tcp:443 through exit 4 via 3.
fn setup() -> Scenario {
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

fn tcp443(destination: &str) -> Traffic {
    Traffic {
        source: "100.64.0.1".parse().unwrap(),
        destination: destination.parse().unwrap(),
        protocol: BusinessProtocol::Tcp,
        source_port: port(1234),
        destination_port: port(443),
    }
}

fn select(compiled: &CompiledNetmaps, destination: &str) -> Result<Destination, AdmissionError> {
    classify(
        &compiled.nodes[&node(1)].netmap,
        destination.parse().unwrap(),
        BusinessProtocol::Tcp,
        port(443),
    )
}

fn exit_keys(compiled: &CompiledNetmaps, destination: &str) -> Vec<GrantKey> {
    match select(compiled, destination) {
        Ok(Destination::Exit(keys)) => keys,
        other => panic!("{destination}: {other:?}"),
    }
}

fn plan(compiled: &CompiledNetmaps) -> &EgressPlan {
    compiled.nodes[&node(1)].netmap.egress.as_ref().unwrap()
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
        let plan = plan(&compiled);
        let nets: Vec<ipnet::IpNet> = plan
            .routes
            .iter()
            .map(|r| r.prefix.parse().unwrap())
            .collect();
        for (i, a) in nets.iter().enumerate() {
            for b in &nets[i + 1..] {
                assert!(!a.contains(b) && !b.contains(a));
            }
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
                let route = plan.route(ip).unwrap();
                assert_eq!(
                    matches!(route.action, EgressAction::Exit { .. }),
                    expected_exit,
                    "{ip} reverse={reverse}"
                );
            }
        }
        assert_eq!(plan.capture.sink_ipv6, ["::/0"]);
        assert!(matches!(
            plan.route("2606:4700::1".parse().unwrap()).unwrap().action,
            EgressAction::Exit { .. }
        ));
    }
}

#[test]
fn exit_grant_enforces_scope_source_and_original_path() {
    let mut s = setup();
    let compiled = netmaps(&s);
    let keys = exit_keys(&compiled, "8.8.8.8");
    assert_eq!(keys.len(), 1);
    let grant = &compiled.nodes[&node(1)].netmap.grants[&keys[0]];
    assert_eq!(
        grant.exit.as_ref().unwrap().destination,
        ExitDestinationScope::Internet
    );
    assert!(grant.paths.iter().all(|p| p.nodes.first() == Some(&node(1))
        && p.nodes.contains(&node(3))
        && p.nodes.last() == Some(&node(4))));
    let path = &grant.paths[0].nodes;
    let exit = &compiled.nodes[&node(4)];
    let lease = lease(exit);
    let leased = Leased {
        netmap: &exit.netmap,
        lease: &lease,
    };
    let previous = Some(path[path.len() - 2]);
    assert_eq!(
        admit_forward(
            leased,
            &keys[0],
            path,
            node(1),
            previous,
            &tcp443("8.8.8.8"),
            1
        ),
        Ok(Hop::Exit)
    );
    let mut forged = tcp443("8.8.8.8");
    forged.source = "100.64.0.3".parse().unwrap();
    assert_eq!(
        admit_forward(leased, &keys[0], path, node(1), previous, &forged, 1),
        Err(AdmissionError::Traffic)
    );
    // The Internet scope never admits private space, even on the same exit.
    assert_eq!(
        admit_forward(
            leased,
            &keys[0],
            path,
            node(1),
            previous,
            &tcp443("10.20.1.1"),
            1
        ),
        Err(AdmissionError::Traffic)
    );
    let cidr = exit_keys(&compiled, "10.20.1.1");
    assert!(matches!(
        compiled.nodes[&node(1)].netmap.grants[&cidr[0]]
            .exit
            .as_ref()
            .unwrap()
            .destination,
        ExitDestinationScope::Cidr { .. }
    ));
    s.cfg.access[0].dst = vec!["autogroup:internet".into()];
    assert_eq!(
        select(&netmaps(&s), "10.20.1.1"),
        Err(AdmissionError::Permit)
    );
    s.cfg.access[0].dst.push("10.20.0.0/16".into());
    s.cfg.nodes.get_mut(&node(4)).unwrap().exit_enabled = false;
    let disabled = netmaps(&s);
    assert_eq!(select(&disabled, "8.8.8.8"), Err(AdmissionError::Permit));
    assert!(disabled.nodes[&node(4)].netmap.exit_role.is_none());
}

#[test]
fn ipv6_and_mesh_pool_destinations_never_leave_through_an_exit() {
    let compiled = netmaps(&setup());
    assert_eq!(
        select(&compiled, "2606:4700:4700::1111"),
        Err(AdmissionError::Traffic)
    );
    assert_eq!(select(&compiled, "100.64.0.200"), Err(AdmissionError::Path));
    assert_eq!(select(&compiled, "127.0.0.1"), Err(AdmissionError::Traffic));
    assert_eq!(plan(&compiled).capture.sink_ipv6, ["::/0"]);
}

#[test]
fn missing_capability_withdraws_role_and_grant_but_keeps_capture() {
    let mut s = setup();
    let ready = netmaps(&s);
    s.inventory
        .get_mut(&node(4))
        .unwrap()
        .capabilities
        .remove("feature:exit-ipv4");
    let incapable = netmaps(&s);
    assert!(incapable.nodes[&node(4)].netmap.exit_role.is_none());
    assert!(incapable.nodes[&node(4)].netmap.grants.is_empty());
    assert_eq!(select(&incapable, "8.8.8.8"), Err(AdmissionError::Permit));
    assert_eq!(plan(&ready), plan(&incapable));
}

#[test]
fn ordered_backup_exits_are_all_granted_and_unreachable_ones_omitted() {
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
    let both = exit_keys(&netmaps(&s), "8.8.8.8");
    let terminals: Vec<NodeId> = both
        .iter()
        .map(|k| match k.target {
            GrantTarget::Exit { node, .. } => node,
            GrantTarget::Mesh { .. } => unreachable!(),
        })
        .collect();
    assert_eq!(terminals, [node(2), node(4)]);
    s.cfg.topology.push(TopologyRule {
        remark: String::new(),
        action: TopologyAction::Block,
        a: sel(2),
        b: "*".into(),
        transports: Input::Omitted,
    });
    let backup = exit_keys(&netmaps(&s), "8.8.8.8");
    assert_eq!(backup.len(), 1);
    assert!(matches!(backup[0].target, GrantTarget::Exit { node: exit, .. } if exit == node(4)));
}

#[test]
fn capture_budget_or_unknown_domain_fails_closed_without_routes() {
    let mut s = setup();
    let mut fragmented = egress_rule("8.8.8.8/32", RuleAction::Direct);
    fragmented.matcher.ip_cidr = (0..1024)
        .map(|i| format!("8.{}.{}.1/32", i / 256, i % 256))
        .collect();
    s.cfg.routing.egress.rules.push(fragmented);
    let limited = netmaps(&s);
    assert_eq!(
        plan(&limited).capture.unavailable,
        Some(UnavailableReason::RoutingLimit)
    );
    assert!(plan(&limited).routes.is_empty());
    assert_eq!(plan(&limited).capture.managed_ipv4, ["0.0.0.0/0"]);
    assert_eq!(select(&limited, "8.8.8.8"), Err(AdmissionError::Traffic));
    assert!(limited.nodes[&node(4)].netmap.grants.is_empty());
    s.cfg.routing.egress.rules.clear();
    let mut unknown = egress_rule("8.8.0.0/16", RuleAction::Direct);
    unknown.matcher.domain.push("example.com".into());
    s.cfg.routing.egress.rules.push(unknown);
    let conflict = netmaps(&s);
    assert_eq!(
        plan(&conflict).capture.unavailable,
        Some(UnavailableReason::EgressConflict)
    );
    assert_eq!(select(&conflict, "8.8.8.8"), Err(AdmissionError::Traffic));
}

#[test]
fn selection_identity_changes_on_switch_and_survives_unrelated_revisions() {
    let mut s = setup();
    let first = netmaps(&s);
    let selection = |c: &CompiledNetmaps| {
        let key = exit_keys(c, "8.8.8.8")[0];
        c.nodes[&node(1)].netmap.grants[&key]
            .exit
            .as_ref()
            .unwrap()
            .selection
    };
    assert_eq!(selection(&first), plan(&first).capture.generation);
    let renewed = compile_with(&s, &previous(&first), Revision::new(7).unwrap());
    assert_eq!(selection(&first), selection(&renewed));
    assert_eq!(first.nodes, renewed.nodes);
    s.selection_versions.insert(node(1), 2);
    assert_ne!(selection(&first), selection(&netmaps(&s)));
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
    // Without an exit profile the default unmanaged route applies.
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
    assert_eq!(service.generation, plan(&compiled).capture.generation);
    assert!(service.path.nodes.contains(&node(3)));
    let resolver = &compiled.nodes[&node(4)];
    let DnsPlan::ServeExitResolver { services } = &resolver.netmap.dns.egress else {
        panic!();
    };
    assert_eq!(services, &[service.as_ref().clone()]);
    assert!(compiled.nodes[&node(3)].netmap.edges.contains_key(&node(4)));
    for compiled_netmap in [source, resolver] {
        let lease = NetmapLease::new(compiled_netmap.reference(), 0, 5000).unwrap();
        let leased = Leased {
            netmap: &compiled_netmap.netmap,
            lease: &lease,
        };
        for transport in [DnsTransport::Udp, DnsTransport::Tcp] {
            let check = |source, exit, path: &[NodeId], now| {
                netmap_dns_service(leased, service, source, exit, path, transport, now)
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
            assert_eq!(
                check(node(1), node(4), &service.path.nodes, 5000),
                Err(ResolverRouteError::Expired)
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
