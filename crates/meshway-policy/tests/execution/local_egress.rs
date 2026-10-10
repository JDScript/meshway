use super::*;

fn external() -> Demand {
    Demand {
        target: BusinessTarget::Ip {
            address: ExecutionAddress("1.1.1.1".parse().unwrap()),
        },
        protocol: BusinessProtocol::Udp,
        port: Some(53.try_into().unwrap()),
        ..demand(1, 1, 4)
    }
}

fn direct_rule() -> EgressRule {
    EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            ip_cidr: vec!["0.0.0.0/0".into()],
            ..Default::default()
        },
        action: RuleAction::Direct,
    }
}

#[test]
fn locked_auto_resolves_allowed_fallbacks_without_allowing_user_profile_switches() {
    let mut c = configuration();
    c.routing.egress.assignments = vec![ProfileAssignment {
        target: vec!["*".into()],
        allowed: vec!["auto".into(), "mesh".into(), "work".into()],
        default: "auto".into(),
        locked: true,
    }];
    c.routing.egress.profiles.insert(
        "work".into(),
        RoutingProfile {
            rules: vec![],
            fallback: RouteAction::Direct,
        },
    );
    let context = ValidationContext {
        nodes: c.nodes.keys().copied().collect(),
        approved_identities: c
            .nodes
            .iter()
            .map(|(id, node)| (*id, node.identity.clone()))
            .collect(),
        ..Default::default()
    };
    let d = Demand {
        profile: "auto".into(),
        ..external()
    };
    for (fallback, action, decision) in [
        (
            "mesh",
            LocalEgressAction::Unmanaged,
            DemandDecision::Unmanaged,
        ),
        ("work", LocalEgressAction::Direct, DemandDecision::Direct),
    ] {
        c.routing.egress.auto = Input::Value(AutoProfile {
            rules: vec![],
            fallback: fallback.into(),
        });
        let (normalized, _) = meshway_policy::normalize((&c).into(), &context).unwrap();
        let publication = run(
            &normalized,
            &inventory(),
            &BTreeMap::from([(d.id, d.clone())]),
            &[],
        );
        let node = &publication.nodes[&d.source].configuration;
        assert_eq!(node.decisions[&d.id], decision);
        assert_eq!(
            node.local_egress[&d.id],
            LocalEgressPermit {
                demand_id: d.id,
                source: d.source,
                target: d.target.clone(),
                protocol: d.protocol,
                port: d.port,
                profile: "auto".into(),
                action,
                expires_at_ms: d.expires_at_ms,
            }
        );
        let switched = Demand {
            profile: fallback.into(),
            ..d.clone()
        };
        assert!(
            !profile_allowed(&normalized, &SubjectContext::default(), d.source, fallback).unwrap()
        );
        let denied = run(
            &normalized,
            &inventory(),
            &BTreeMap::from([(switched.id, switched)]),
            &[],
        );
        assert_eq!(
            denied.nodes[&d.source].configuration.decisions[&d.id],
            DemandDecision::Unavailable {
                reason: UnavailableReason::InvalidProfile,
            }
        );
        assert!(
            denied.nodes[&d.source]
                .configuration
                .local_egress
                .is_empty()
        );
    }
    c.routing.egress.assignments[0]
        .allowed
        .retain(|profile| profile != "work");
    assert!(meshway_policy::normalize((&c).into(), &context).is_err());
    // Pure compilation also refuses an out-of-allowed internal choice, even
    // when presented directly with an unnormalized trusted-input fixture.
    assert_eq!(
        compile(CompilationInput {
            network: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
            revision: Revision::MIN,
            publication: "00000000-0000-4004-8000-000000000001".parse().unwrap(),
            configuration: &c,
            subjects: &SubjectContext::default(),
            inventory: &inventory(),
            demands: &BTreeMap::from([(d.id, d)]),
            dns: &[],
            rule_sets: &BTreeMap::new(),
            selected_profiles: &BTreeMap::new(),
            now_ms: 1000,
        })
        .unwrap_err(),
        CompileError::InvalidInput
    );
}

#[test]
fn local_binding_is_exact_and_published_only_to_the_authenticated_source_slice() {
    let mut c = configuration();
    c.routing.egress.rules = vec![direct_rule()];
    let d = external();
    let result = run(&c, &inventory(), &BTreeMap::from([(d.id, d.clone())]), &[]);
    let slice = &result.nodes[&d.source];
    assert_eq!(
        slice.configuration.local_egress[&d.id],
        LocalEgressPermit {
            demand_id: d.id,
            source: d.source,
            target: d.target.clone(),
            protocol: d.protocol,
            port: d.port,
            profile: d.profile.clone(),
            action: LocalEgressAction::Direct,
            expires_at_ms: d.expires_at_ms,
        }
    );
    for (id, node) in &result.nodes {
        assert!(node.configuration.permits.is_empty());
        assert!(node.configuration.peers.is_empty());
        if id != &d.source {
            assert!(node.configuration.local_egress.is_empty());
        }
    }
    // A stable query ID does not erase endpoint/service/source binding. Every
    // changed request produces different immutable execution data and digest.
    for changed in [
        Demand {
            target: BusinessTarget::Ip {
                address: ExecutionAddress("8.8.8.8".parse().unwrap()),
            },
            ..d.clone()
        },
        Demand {
            protocol: BusinessProtocol::Tcp,
            ..d.clone()
        },
        Demand {
            port: Some(5353.try_into().unwrap()),
            ..d.clone()
        },
        Demand {
            source: node(2),
            ..d.clone()
        },
    ] {
        let other = run(
            &c,
            &inventory(),
            &BTreeMap::from([(changed.id, changed.clone())]),
            &[],
        );
        let binding = &other.nodes[&changed.source].configuration.local_egress[&changed.id];
        assert_ne!(binding, &slice.configuration.local_egress[&d.id]);
        assert_eq!(binding.target, changed.target);
        assert_eq!(binding.protocol, changed.protocol);
        assert_eq!(binding.port, changed.port);
        assert_eq!(binding.source, changed.source);
        assert_ne!(other.nodes[&d.source].digest, slice.digest);
    }
}

#[test]
fn local_dns_deadlines_cover_domain_and_ip_rules_without_inference_after_evaluation() {
    let mut c = configuration();
    c.routing.egress.rules = vec![EgressRule {
        matcher: RouteMatch {
            domain_suffix: vec!["example.com".into()],
            ..Default::default()
        },
        ..direct_rule()
    }];
    let dns = [TrustedDnsAssociation {
        network_id: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
        configuration_revision: Revision::MIN,
        source: node(1),
        name: "api.example.com".into(),
        cname_chain: vec![],
        addresses: vec![ExecutionAddress("1.1.1.1".parse().unwrap())],
        expires_at_ms: 2000,
    }];
    for target in [
        external().target,
        BusinessTarget::Domain {
            name: "api.example.com".into(),
            address: dns[0].addresses[0],
        },
    ] {
        for deadline in [1500, 100_000, 200_000] {
            let d = Demand {
                target: target.clone(),
                expires_at_ms: deadline,
                ..external()
            };
            let result = run(&c, &inventory(), &BTreeMap::from([(d.id, d.clone())]), &dns);
            let binding = &result.nodes[&d.source].configuration.local_egress[&d.id];
            assert_eq!(binding.target, target);
            assert_eq!(binding.expires_at_ms, deadline.min(2000));
            assert_eq!(binding.action, LocalEgressAction::Direct);
        }
    }
    // A shorter exact-name association and a longer rule-suffix association
    // both bind a Domain demand; their independently derived bounds intersect.
    let d = Demand {
        target: BusinessTarget::Domain {
            name: "api.example.com".into(),
            address: dns[0].addresses[0],
        },
        ..external()
    };
    let mut associations = dns.to_vec();
    associations.push(TrustedDnsAssociation {
        name: "other.example.com".into(),
        expires_at_ms: 3000,
        ..dns[0].clone()
    });
    assert_eq!(
        run(
            &c,
            &inventory(),
            &BTreeMap::from([(d.id, d.clone())]),
            &associations
        )
        .nodes[&d.source]
            .configuration
            .local_egress[&d.id]
            .expires_at_ms,
        2000
    );
    let foreign_name = Demand {
        target: BusinessTarget::Domain {
            name: "foreign.example.com".into(),
            address: dns[0].addresses[0],
        },
        ..d.clone()
    };
    assert!(
        run(
            &c,
            &inventory(),
            &BTreeMap::from([(foreign_name.id, foreign_name)]),
            &dns
        )
        .nodes[&node(1)]
            .configuration
            .local_egress
            .is_empty()
    );
    c.routing.egress.rules.clear();
    let unmanaged = run(&c, &inventory(), &BTreeMap::from([(d.id, d.clone())]), &dns);
    let binding = &unmanaged.nodes[&d.source].configuration.local_egress[&d.id];
    assert_eq!(binding.action, LocalEgressAction::Unmanaged);
    assert_eq!(binding.expires_at_ms, 2000);
    let expired = [TrustedDnsAssociation {
        expires_at_ms: 1000,
        ..dns[0].clone()
    }];
    let unavailable = run(&c, &inventory(), &BTreeMap::from([(d.id, d)]), &expired);
    assert!(
        unavailable.nodes[&node(1)]
            .configuration
            .local_egress
            .is_empty()
    );
}

#[test]
fn local_routes_follow_public_then_profile_first_match_and_never_bypass_unavailable_exits() {
    let mut c = configuration();
    c.routing.egress.assignments = vec![ProfileAssignment {
        target: vec!["*".into()],
        allowed: vec!["work".into()],
        default: "work".into(),
        locked: true,
    }];
    c.routing.egress.profiles.insert(
        "work".into(),
        RoutingProfile {
            rules: vec![direct_rule()],
            fallback: RouteAction::Unmanaged,
        },
    );
    let d = Demand {
        profile: "work".into(),
        ..external()
    };
    let ds = BTreeMap::from([(d.id, d.clone())]);
    assert_eq!(
        run(&c, &inventory(), &ds, &[]).nodes[&d.source]
            .configuration
            .local_egress[&d.id]
            .action,
        LocalEgressAction::Direct
    );
    c.routing
        .egress
        .profiles
        .get_mut("work")
        .unwrap()
        .rules
        .clear();
    assert_eq!(
        run(&c, &inventory(), &ds, &[]).nodes[&d.source]
            .configuration
            .local_egress[&d.id]
            .action,
        LocalEgressAction::Unmanaged
    );
    c.routing.egress.rules = vec![direct_rule()];
    assert_eq!(
        run(&c, &inventory(), &ds, &[]).nodes[&d.source]
            .configuration
            .local_egress[&d.id]
            .action,
        LocalEgressAction::Direct
    );
    c.routing.egress.rules.insert(
        0,
        EgressRule {
            action: RuleAction::Exit {
                nodes: vec![format!("node:{}", node(4))],
                via: vec![],
            },
            ..direct_rule()
        },
    );
    c.access.clear();
    let unavailable = run(&c, &inventory(), &ds, &[]);
    assert!(
        unavailable.nodes[&d.source]
            .configuration
            .local_egress
            .is_empty()
    );
    assert_eq!(
        unavailable.nodes[&d.source].configuration.decisions[&d.id],
        DemandDecision::Unavailable {
            reason: UnavailableReason::Unauthorized
        }
    );
    c.routing.egress.rules = vec![direct_rule()];
    let invalid = Demand {
        profile: "mesh".into(),
        ..d
    };
    assert!(
        run(
            &c,
            &inventory(),
            &BTreeMap::from([(invalid.id, invalid)]),
            &[]
        )
        .nodes[&node(1)]
            .configuration
            .local_egress
            .is_empty()
    );
}

#[test]
fn mesh_targets_and_disabled_sources_never_receive_local_egress() {
    let mut c = configuration();
    c.routing.egress.rules = vec![direct_rule()];
    c.address_pools.ipv6 = vec!["fd00::/64".into()];
    c.nodes.get_mut(&node(2)).unwrap().addresses.ipv6 = Some("fd00::2".parse().unwrap());
    for target in [
        BusinessTarget::Mesh { node: node(2) },
        BusinessTarget::Ip {
            address: ExecutionAddress("100.64.0.2".parse().unwrap()),
        },
        BusinessTarget::Ip {
            address: ExecutionAddress("100.64.0.200".parse().unwrap()),
        },
        BusinessTarget::Ip {
            address: ExecutionAddress("::ffff:100.64.0.2".parse().unwrap()),
        },
        BusinessTarget::Ip {
            address: ExecutionAddress("::ffff:100.64.0.200".parse().unwrap()),
        },
        BusinessTarget::Ip {
            address: ExecutionAddress("fd00::2".parse().unwrap()),
        },
        BusinessTarget::Ip {
            address: ExecutionAddress("fd00::200".parse().unwrap()),
        },
    ] {
        let d = Demand {
            target,
            ..external()
        };
        for allowed in [true, false] {
            let mut config = c.clone();
            if !allowed {
                config.access.clear();
            }
            assert!(
                run(
                    &config,
                    &inventory(),
                    &BTreeMap::from([(d.id, d.clone())]),
                    &[]
                )
                .nodes
                .values()
                .all(|slice| slice.configuration.local_egress.is_empty())
            );
        }
    }
    c.nodes.get_mut(&node(1)).unwrap().enabled = false;
    let d = external();
    assert!(
        run(&c, &inventory(), &BTreeMap::from([(d.id, d)]), &[]).nodes[&node(1)]
            .configuration
            .local_egress
            .is_empty()
    );
}
