//! The capture block's RPDB allocation, verification and positive routes,
//! computed without a kernel.
use super::*;

fn defaults() -> Vec<RuleMessage> {
    let mut rules = Vec::new();
    for family in [AddressFamily::Inet, AddressFamily::Inet6] {
        for (priority, table) in [(0, 255), (32766, 254)] {
            let mut rule = RuleMessage::default();
            rule.header.family = family;
            rule.header.action = rtnetlink::packet_route::rule::RuleAction::ToTable;
            rule.header.table = table;
            rule.attributes.push(RuleAttribute::Priority(priority));
            rules.push(rule);
        }
    }
    rules
}

#[test]
fn duplicate_priority_and_foreign_table_reference_block_cleanup() {
    let mut rules = defaults();
    let mut journal = allocate(&rules, &[], 64).unwrap();
    let spec = RuleSpec {
        ipv6: false,
        priority: journal.start,
        table: journal.table,
        kind: RuleKind::End,
    };
    let message = spec.message();
    journal.rules.push(RuleRecord {
        spec,
        observed: encode_rule(message.clone()),
    });
    rules.push(message.clone());
    verify_rules(&journal, &rules).unwrap();
    let mut foreign = message;
    foreign.header.action = rtnetlink::packet_route::rule::RuleAction::Unreachable;
    rules.push(foreign);
    assert!(verify_rules(&journal, &rules).is_err());
    rules.pop();
    let mut foreign = rules.last().unwrap().clone();
    foreign
        .attributes
        .retain(|a| !matches!(a, RuleAttribute::Priority(_)));
    foreign.attributes.push(RuleAttribute::Priority(32000));
    rules.push(foreign);
    assert!(verify_rules(&journal, &rules).is_err());
}

#[test]
fn early_custom_rule_and_exact_zero_mark_selector_refuse_installation() {
    let mut rules = defaults();
    let mut early = rules[1].clone();
    early.attributes = vec![RuleAttribute::Priority(2)];
    rules.push(early);
    assert!(allocate(&rules, &[], 64).is_err());
    rules.pop();
    let mut marked = rules[1].clone();
    marked
        .attributes
        .extend([RuleAttribute::FwMark(0), RuleAttribute::FwMask(u32::MAX)]);
    rules.push(marked);
    assert!(allocate(&rules, &[], 64).is_err());
}

#[test]
fn retained_tun_requires_exact_active_routes_not_the_retired_pool_union() {
    let active: Vec<ipnet::IpNet> = vec![
        "100.96.0.0/16".parse().unwrap(),
        "fd12:3456::/64".parse().unwrap(),
    ];
    let retired = "100.97.0.0/16".parse().unwrap();
    let addresses = meshway_configuration::NodeAddresses {
        ipv4: Some("100.96.0.1".parse().unwrap()),
        ipv6: Some("fd12:3456::1".parse().unwrap()),
    };
    let mut journal = allocate(&defaults(), &[], 64).unwrap();
    journal.pools = active.iter().copied().chain([retired]).collect();
    for pool in &active {
        let source = if pool.addr().is_ipv4() {
            IpAddr::V4(addresses.ipv4.unwrap())
        } else {
            IpAddr::V6(addresses.ipv6.unwrap())
        };
        let spec = RouteSpec {
            pool: *pool,
            table: journal.table,
            ifindex: 7,
            source,
        };
        journal.routes.push(RouteRecord {
            observed: encode_route(spec.message().unwrap()),
            spec,
        });
    }
    assert!(live_routes_match(&journal, 7, &active, &addresses));
    assert!(
        !live_routes_match(&journal, 8, &active, &addresses),
        "a replacement device cannot inherit the old route proof"
    );
    assert!(
        !live_routes_match(&journal, 7, &journal.pools, &addresses),
        "negative-only retired pools are not live routes"
    );
    let mut reordered = active.clone();
    reordered.reverse();
    assert!(same_pools(&active, &reordered));
    assert!(!same_pools(&active, &journal.pools));
    journal.routes[0].spec.source = "100.96.0.2".parse().unwrap();
    assert!(
        !live_routes_match(&journal, 7, &active, &addresses),
        "preferred source changes require installation"
    );
}

fn dual_stack_pools() -> meshway_configuration::AddressPools {
    meshway_configuration::AddressPools {
        ipv4: vec!["100.96.0.0/16".into()],
        ipv6: vec!["fd12:3456::/64".into()],
    }
}

#[test]
fn nullable_families_keep_complete_capture_and_only_assigned_positive_routes() {
    let published = dual_stack_pools();
    for assigned in [
        "100.96.0.1".parse::<IpAddr>().unwrap(),
        "fd12:3456::1".parse().unwrap(),
    ] {
        let addresses = meshway_configuration::NodeAddresses {
            ipv4: match assigned {
                IpAddr::V4(ip) => Some(ip),
                IpAddr::V6(_) => None,
            },
            ipv6: match assigned {
                IpAddr::V6(ip) => Some(ip),
                IpAddr::V4(_) => None,
            },
        };
        let pools = plan::pools(&published, &addresses, 64).unwrap();
        assert_eq!(
            pools,
            vec![
                "100.96.0.0/16".parse().unwrap(),
                "fd12:3456::/64".parse().unwrap()
            ]
        );
        let mut journal = allocate(&defaults(), &[], 64).unwrap();
        journal.pools = pools.clone();
        let routes = plan::positive_routes(&pools, &addresses, journal.table, 7);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].source, assigned);
        assert_eq!(routes[0].pool.addr().is_ipv4(), assigned.is_ipv4());
        for spec in routes {
            journal.routes.push(RouteRecord {
                observed: encode_route(spec.message().unwrap()),
                spec,
            });
        }
        assert!(live_routes_match(&journal, 7, &pools, &addresses));

        // A route using an underlay source for the unassigned family is not an
        // acceptable substitute for the negative boundary, even in our table.
        let missing = pools
            .iter()
            .find(|pool| pool.addr().is_ipv4() != assigned.is_ipv4())
            .copied()
            .unwrap();
        let source = if missing.addr().is_ipv4() {
            "192.0.2.1"
        } else {
            "2001:db8::1"
        }
        .parse()
        .unwrap();
        let spec = RouteSpec {
            pool: missing,
            source,
            table: journal.table,
            ifindex: 7,
        };
        journal.routes.push(RouteRecord {
            observed: encode_route(spec.message().unwrap()),
            spec,
        });
        assert!(!live_routes_match(&journal, 7, &pools, &addresses));
    }
}

#[test]
fn unaddressed_relays_and_explicit_empty_pools_have_no_positive_routes() {
    let addresses = meshway_configuration::NodeAddresses {
        ipv4: None,
        ipv6: None,
    };
    let all = plan::pools(&dual_stack_pools(), &addresses, 64).unwrap();
    let empty = plan::pools(
        &meshway_configuration::AddressPools {
            ipv4: vec![],
            ipv6: vec![],
        },
        &addresses,
        64,
    )
    .unwrap();
    assert_eq!(all.len(), 2);
    assert!(empty.is_empty());
    let mut journal = allocate(&defaults(), &[], 64).unwrap();
    journal.pools = all.clone();
    for pools in [&all, &empty] {
        assert!(plan::positive_routes(pools, &addresses, journal.table, 7).is_empty());
        assert!(live_routes_match(&journal, 7, pools, &addresses));
    }
    // Equal positive-route sets alone do not authorize retaining an interface
    // across a changed capture configuration. Control continuity also compares
    // the complete current pool sets, while the old negative union survives.
    assert!(!same_pools(&all, &empty));
    assert_eq!(journal.pools, all);
}

#[test]
fn assigned_addresses_outside_published_family_pools_are_rejected() {
    let published = dual_stack_pools();
    for addresses in [
        meshway_configuration::NodeAddresses {
            ipv4: Some("100.97.0.1".parse().unwrap()),
            ipv6: None,
        },
        meshway_configuration::NodeAddresses {
            ipv4: None,
            ipv6: Some("fd12:3457::1".parse().unwrap()),
        },
    ] {
        assert!(matches!(
            plan::pools(&published, &addresses, 64),
            Err(Error::Target)
        ));
    }
    let assigned = meshway_configuration::NodeAddresses {
        ipv4: Some("100.96.0.1".parse().unwrap()),
        ipv6: None,
    };
    let missing_family = meshway_configuration::AddressPools {
        ipv4: vec![],
        ipv6: published.ipv6,
    };
    assert!(matches!(
        plan::pools(&missing_family, &assigned, 64),
        Err(Error::Target)
    ));
    assert!(matches!(
        plan::pools(
            &meshway_configuration::AddressPools {
                ipv4: vec![],
                ipv6: vec![]
            },
            &assigned,
            64
        ),
        Err(Error::Target)
    ));
}

#[test]
fn retired_overlapping_pool_never_becomes_a_positive_covering_route() {
    let addresses = meshway_configuration::NodeAddresses {
        ipv4: Some("100.96.128.1".parse().unwrap()),
        ipv6: None,
    };
    let published = meshway_configuration::AddressPools {
        ipv4: vec!["100.96.128.0/17".into()],
        ipv6: vec![],
    };
    let active = plan::pools(&published, &addresses, 64).unwrap();
    let retired = "100.96.0.0/16".parse().unwrap();
    let mut journal = allocate(&defaults(), &[], 64).unwrap();
    journal.pools = vec![retired, active[0]];
    for spec in plan::positive_routes(&active, &addresses, journal.table, 7) {
        journal.routes.push(RouteRecord {
            observed: encode_route(spec.message().unwrap()),
            spec,
        });
    }
    assert!(live_routes_match(&journal, 7, &active, &addresses));
    let old_remainder: IpAddr = "100.96.1.1".parse().unwrap();
    assert!(
        journal
            .routes
            .iter()
            .all(|route| !route.spec.pool.contains(&old_remainder))
    );
    journal.routes[0].spec.pool = retired;
    journal.routes[0].observed = encode_route(journal.routes[0].spec.message().unwrap());
    assert!(
        !live_routes_match(&journal, 7, &active, &addresses),
        "equal counts do not excuse an unexpected covering route"
    );
}
