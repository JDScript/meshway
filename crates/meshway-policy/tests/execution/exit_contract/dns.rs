use super::*;
use meshway_policy::dns_route::{ResolverRouteError, dns_service_permit};

#[test]
fn unrelated_instance_renewal_does_not_replace_no_exit_targets() {
    let (mut cfg, mut d, mut runtime) = setup();
    cfg.routing.egress.assignments.clear();
    d.profile = "mesh".into();
    let first = compiled(&cfg, &d, &runtime);
    runtime.instance_expires_at_ms.insert(node(1), 180_000);
    assert_eq!(first, compiled(&cfg, &d, &runtime));
}

#[test]
fn single_exit_dns_joint_permission_and_deadline() {
    let (mut cfg, d, mut runtime) = setup();
    let result = compiled(&cfg, &d, &runtime);
    let source = &result.nodes[&node(1)].configuration;
    let DnsEgress::ViaExit {
        exit,
        service,
        generation,
    } = &source.dns.egress
    else {
        panic!("{:?}", source.dns.egress);
    };
    assert_eq!(*exit, node(4));
    assert_eq!(*generation, source.egress_capture.generation);
    assert_eq!(service.expires_at_ms, runtime.ready_until_ms[exit]);
    assert!(service.path.nodes.contains(&node(3)));
    let target = &result.nodes[exit].configuration;
    let DnsEgress::ServeExitResolver { permit } = &target.dns.egress else {
        panic!();
    };
    assert_eq!(permit, &[service.as_ref().clone()]);
    for configuration in [source, target] {
        for transport in [DnsTransport::Udp, DnsTransport::Tcp] {
            assert_eq!(
                dns_service_permit(
                    configuration,
                    service,
                    node(1),
                    node(4),
                    &service.path.nodes,
                    transport,
                    1001
                ),
                Ok(())
            );
            assert_eq!(
                dns_service_permit(
                    configuration,
                    service,
                    node(2),
                    node(4),
                    &service.path.nodes,
                    transport,
                    1001
                ),
                Err(ResolverRouteError::Binding)
            );
            assert_eq!(
                dns_service_permit(
                    configuration,
                    service,
                    node(1),
                    node(3),
                    &service.path.nodes,
                    transport,
                    1001
                ),
                Err(ResolverRouteError::Binding)
            );
            assert_eq!(
                dns_service_permit(
                    configuration,
                    service,
                    node(1),
                    node(4),
                    &[node(1), node(4)],
                    transport,
                    1001
                ),
                Err(ResolverRouteError::Binding)
            );
            assert_eq!(
                dns_service_permit(
                    configuration,
                    service,
                    node(1),
                    node(4),
                    &service.path.nodes,
                    transport,
                    service.expires_at_ms
                ),
                Err(ResolverRouteError::Expired)
            );
        }
    }
    runtime.ready_until_ms.clear();
    assert_unavailable(&cfg, &d, &runtime, DnsEgressUnavailable::ExitUnavailable);
    runtime.ready_until_ms.insert(node(4), 1000);
    assert_unavailable(&cfg, &d, &runtime, DnsEgressUnavailable::ExitUnavailable);
    runtime.ready_until_ms.insert(node(4), 100_000);
    runtime.instance_expires_at_ms.insert(node(1), 1500);
    let bounded = compiled(&cfg, &d, &runtime);
    let DnsEgress::ViaExit { service, .. } = &bounded.nodes[&node(1)].configuration.dns.egress
    else {
        panic!();
    };
    assert_eq!(service.expires_at_ms, 1500);
    runtime.instance_expires_at_ms.insert(node(1), 1000);
    assert_unavailable(&cfg, &d, &runtime, DnsEgressUnavailable::Unauthorized);
    runtime.instance_expires_at_ms.insert(node(1), 120_000);
    cfg.nodes.get_mut(&node(4)).unwrap().exit_enabled = false;
    assert_unavailable(&cfg, &d, &runtime, DnsEgressUnavailable::ExitUnavailable);
    cfg.nodes.get_mut(&node(4)).unwrap().exit_enabled = true;
    cfg.access.clear();
    assert_unavailable(&cfg, &d, &runtime, DnsEgressUnavailable::Unauthorized);
}

fn assert_unavailable(
    cfg: &NetworkConfiguration,
    d: &Demand,
    runtime: &ExitInputs,
    reason: DnsEgressUnavailable,
) {
    assert_eq!(
        compiled(cfg, d, runtime).nodes[&node(1)]
            .configuration
            .dns
            .egress,
        DnsEgress::Unavailable { reason }
    );
}

#[test]
fn resolver_loop_multi_exit_and_missing_path_stay_unavailable() {
    let (mut cfg, d, runtime) = setup();
    cfg.routing.egress.assignments.push(ProfileAssignment {
        target: vec![format!("node:{}", node(4))],
        allowed: vec!["exit".into()],
        default: "exit".into(),
        locked: false,
    });
    assert_unavailable(&cfg, &d, &runtime, DnsEgressUnavailable::ExitPlanRequired);
    cfg.routing.egress.assignments.pop();
    let RouteAction::Exit { nodes, .. } = &mut cfg
        .routing
        .egress
        .profiles
        .get_mut("exit")
        .unwrap()
        .fallback
    else {
        panic!();
    };
    nodes.push(format!("node:{}", node(2)));
    assert_unavailable(&cfg, &d, &runtime, DnsEgressUnavailable::ExitPlanRequired);
    let RouteAction::Exit { nodes, .. } = &mut cfg
        .routing
        .egress
        .profiles
        .get_mut("exit")
        .unwrap()
        .fallback
    else {
        panic!();
    };
    nodes.pop();
    cfg.nodes.get_mut(&node(3)).unwrap().enabled = false;
    assert_unavailable(&cfg, &d, &runtime, DnsEgressUnavailable::NoPath);
}

#[test]
fn ip_rule_single_exit_is_selected_before_name_resolution() {
    let (mut cfg, d, runtime) = setup();
    let profile = cfg.routing.egress.profiles.get_mut("exit").unwrap();
    profile.rules = vec![EgressRule {
        matcher: RouteMatch {
            ip_cidr: vec!["10.20.0.0/16".into()],
            ..Default::default()
        },
        action: RuleAction::Exit {
            nodes: vec![format!("node:{}", node(4))],
            via: vec![format!("node:{}", node(3))],
        },
        remark: String::new(),
    }];
    profile.fallback = RouteAction::Unmanaged;
    let result = compiled(&cfg, &d, &runtime);
    assert!(matches!(
        result.nodes[&node(1)].configuration.dns.egress,
        DnsEgress::ViaExit { .. }
    ));
    assert!(
        result.nodes[&node(1)]
            .configuration
            .dns
            .associations
            .is_empty()
    );
}
