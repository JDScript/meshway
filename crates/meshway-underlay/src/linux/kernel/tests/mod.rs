use super::*;

mod network_changes;
mod restart;
mod roaming;

#[test]
fn dns_anchor_address_conflicts_with_specific_routes_but_not_the_default_route() {
    use rtnetlink::packet_route::route::{RouteAddress, RouteAttribute};
    let address = "198.18.255.254".parse().unwrap();
    let mut route = RouteMessage::default();
    route.header.address_family = AddressFamily::Inet;
    assert!(!anchor::route_conflicts(address, None, &route));
    route.header.destination_prefix_length = 15;
    route
        .attributes
        .push(RouteAttribute::Destination(RouteAddress::Inet(
            "198.18.0.0".parse().unwrap(),
        )));
    assert!(anchor::route_conflicts(address, None, &route));
    route.attributes = vec![
        RouteAttribute::Destination(RouteAddress::Inet(address)),
        RouteAttribute::Oif(7),
    ];
    route.header.destination_prefix_length = 32;
    route.header.table = 255;
    route.header.protocol = rtnetlink::packet_route::route::RouteProtocol::Kernel;
    route.header.kind = rtnetlink::packet_route::route::RouteType::Local;
    assert!(anchor::route_conflicts(address, None, &route));
    assert!(anchor::route_conflicts(address, Some(8), &route));
    assert!(!anchor::route_conflicts(address, Some(7), &route));
    route.header.protocol = rtnetlink::packet_route::route::RouteProtocol::Static;
    assert!(anchor::route_conflicts(address, Some(7), &route));
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires an isolated Linux network namespace, private /run and NET_ADMIN"]
async fn dns_anchor_survives_suspend_and_owner_restart_until_explicit_removal() {
    assert_eq!(std::env::var("MESHWAY_DNS_ISOLATED").as_deref(), Ok("1"));
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(Directory::open(&temp.path().join("node"), "node.lock").unwrap());
    let mut reservation =
        meshway_dns::SystemDnsOwner::reserve(Default::default(), directory.clone())
            .await
            .unwrap();
    let tasks = Tasks::new(64);
    let mut kernel = Kernel::open(directory.clone(), Default::default(), tasks.clone())
        .await
        .unwrap();
    let sockets = dns_test_socket_ids();
    let guard = kernel.dns_anchor(&reservation).await.unwrap();
    let identity = guard.verify().await.unwrap();
    assert_concurrent_dns_anchor_reads(&kernel, guard.as_ref(), &identity).await;
    let reader_socket = dns_test_new_reader_socket(&sockets);
    assert_dns_reader_waiter_cancellation(guard.as_ref(), &identity).await;
    assert_dns_reader_drop(&mut kernel, &reservation, guard.as_ref(), &identity).await;
    assert_dns_anchor_installation(&kernel, &identity).await;
    kernel.suspend().await.unwrap();
    assert_eq!(guard.verify().await.unwrap(), identity);
    kernel.settle().await.unwrap();
    let mut active = Box::pin(guard.verify());
    assert!(futures_util::poll!(active.as_mut()).is_pending());
    let mut waiter = Box::pin(guard.verify());
    assert!(futures_util::poll!(waiter.as_mut()).is_pending());
    drop(kernel);
    tasks.shutdown().await.unwrap();
    // Shutdown closes the descriptor even while these borrowing futures have
    // not been polled again, and wakes both the active read and mutex waiter.
    assert!(!dns_test_socket_ids().contains(&reader_socket));
    assert!(active.await.is_err());
    assert!(waiter.await.is_err());
    assert!(guard.verify().await.is_err());
    drop(guard);
    let tasks = Tasks::new(64);
    let mut recovered = Kernel::open(directory.clone(), Default::default(), tasks.clone())
        .await
        .unwrap();
    let sockets = dns_test_socket_ids();
    let guard = recovered.dns_anchor(&reservation).await.unwrap();
    assert_eq!(guard.verify().await.unwrap(), identity);
    assert_concurrent_dns_anchor_reads(&recovered, guard.as_ref(), &identity).await;
    let reader_socket = dns_test_new_reader_socket(&sockets);
    assert_dns_anchor_installation(&recovered, &identity).await;
    recovered.remove_dns_anchor().await.unwrap();
    assert!(guard.verify().await.is_err());
    dns_test_wait_reader_closed(&reader_socket).await;
    recovered.down().await.unwrap();
    recovered.settle().await.unwrap();
    drop(recovered);
    tasks.shutdown().await.unwrap();
    reservation.release_uninstalled().await.unwrap();
    drop(guard);
    drop(reservation);
    assert_dns_reader_capacity_cleanup(directory).await;
}

async fn assert_concurrent_dns_anchor_reads(
    kernel: &Kernel,
    guard: &dyn meshway_dns::OwnedDnsLink,
    identity: &meshway_dns::DnsLinkIdentity,
) {
    // Current-thread join polls every branch before the I/O task can process
    // queued GETs. No error retry: both independent-reader and same-guard
    // serialization must work on the actual netlink sockets in every round.
    for round in 0..8 {
        let (checked, first, second) =
            tokio::join!(kernel.verify(), guard.verify(), guard.verify());
        assert!(
            checked.is_ok(),
            "Kernel dump failed in round {round}: {checked:?}"
        );
        assert_eq!(first.unwrap(), *identity);
        assert_eq!(second.unwrap(), *identity);
    }
}

async fn assert_dns_reader_waiter_cancellation(
    guard: &dyn meshway_dns::OwnedDnsLink,
    identity: &meshway_dns::DnsLinkIdentity,
) {
    let mut active = Box::pin(guard.verify());
    assert!(futures_util::poll!(active.as_mut()).is_pending());
    let mut waiter = Box::pin(guard.verify());
    assert!(futures_util::poll!(waiter.as_mut()).is_pending());
    drop(waiter); // No GET was submitted by this queued mutex waiter.
    assert_eq!(active.await.unwrap(), *identity);
    assert_eq!(guard.verify().await.unwrap(), *identity);
}

async fn assert_dns_reader_drop(
    kernel: &mut Kernel,
    reservation: &meshway_dns::SystemDnsReservation,
    kept: &dyn meshway_dns::OwnedDnsLink,
    identity: &meshway_dns::DnsLinkIdentity,
) {
    let sockets = dns_test_socket_ids();
    let reader = kernel.dns_anchor(reservation).await.unwrap();
    let socket = dns_test_new_reader_socket(&sockets);
    assert_eq!(reader.verify().await.unwrap(), *identity);
    drop(reader);
    dns_test_wait_reader_closed(&socket).await;
    assert_eq!(kept.verify().await.unwrap(), *identity);

    let sockets = dns_test_socket_ids();
    let reader = kernel.dns_anchor(reservation).await.unwrap();
    let socket = dns_test_new_reader_socket(&sockets);
    let mut pending = Box::pin(reader.verify());
    assert!(futures_util::poll!(pending.as_mut()).is_pending());
    drop(pending); // An interrupted dump must not leave a reusable reader FD.
    dns_test_wait_reader_closed(&socket).await;
    // ... but it proves nothing about the anchor (a host-wide RTNL stall
    // abandons reads): the anchor stays, and the next read opens a new FD.
    let ended = tokio::time::timeout(Duration::from_millis(200), reader.ended()).await;
    assert!(ended.is_err(), "an interrupted read ended the anchor");
    let sockets = dns_test_socket_ids();
    assert_eq!(reader.verify().await.unwrap(), *identity);
    assert_ne!(dns_test_new_reader_socket(&sockets), socket);
    // A deadline that elapses mid-dump (the observation timeout) is the same.
    for _ in 0..4 {
        let _ = tokio::time::timeout(Duration::ZERO, reader.verify()).await;
        assert_eq!(reader.verify().await.unwrap(), *identity);
    }
    drop(reader);
    assert_eq!(kept.verify().await.unwrap(), *identity);
    kernel.verify().await.unwrap();
}

async fn assert_dns_reader_capacity_cleanup(directory: Arc<Directory>) {
    let mut reservation =
        meshway_dns::SystemDnsOwner::reserve(Default::default(), directory.clone())
            .await
            .unwrap();
    // Kernel's request connection, monitor and change classifier exhaust
    // this existing task budget.
    let tasks = Tasks::new(3);
    let mut kernel = Kernel::open(directory, Default::default(), tasks.clone())
        .await
        .unwrap();
    let sockets = dns_test_socket_ids();
    assert!(matches!(
        kernel.dns_anchor(&reservation).await,
        Err(Error::Capacity)
    ));
    assert_eq!(
        dns_test_socket_ids(),
        sockets,
        "rejected reader leaked its FD"
    );
    assert!(kernel.journal.anchor.is_some());
    assert!(kernel.journal.pending.is_none());
    kernel.remove_dns_anchor().await.unwrap();
    kernel.down().await.unwrap();
    kernel.settle().await.unwrap();
    drop(kernel);
    tasks.shutdown().await.unwrap();
    reservation.release_uninstalled().await.unwrap();
}

fn dns_test_socket_ids() -> std::collections::BTreeSet<String> {
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|entry| match std::fs::read_link(entry.unwrap().path()) {
            Ok(path) => path
                .to_str()
                .filter(|target| target.starts_with("socket:["))
                .map(str::to_owned),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("reading socket identity failed: {error}"),
        })
        .collect()
}

fn dns_test_new_reader_socket(previous: &std::collections::BTreeSet<String>) -> String {
    let current = dns_test_socket_ids();
    let added: Vec<_> = current.difference(previous).cloned().collect();
    // Also fails deterministically with the old Handle::clone implementation,
    // even on a tiny fixture whose dump happens to fit into a single response.
    assert_eq!(
        added.len(),
        1,
        "one independent FD per guard, not per query"
    );
    added.into_iter().next().unwrap()
}

async fn dns_test_wait_reader_closed(socket: &str) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while dns_test_socket_ids().contains(socket) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("reader FD survived cancellation");
}

#[tokio::test]
#[ignore = "requires an isolated Linux network namespace, private /run and NET_ADMIN"]
async fn foreign_dns_anchor_mutation_blocks_cleanup_without_removing_the_interface() {
    assert_eq!(std::env::var("MESHWAY_DNS_ISOLATED").as_deref(), Ok("1"));
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(Directory::open(&temp.path().join("node"), "node.lock").unwrap());
    let reservation = meshway_dns::SystemDnsOwner::reserve(Default::default(), directory.clone())
        .await
        .unwrap();
    let tasks = Tasks::new(64);
    let mut kernel = Kernel::open(directory, Default::default(), tasks.clone())
        .await
        .unwrap();
    let guard = kernel.dns_anchor(&reservation).await.unwrap();
    let identity = guard.verify().await.unwrap();
    execute(
        kernel
            .handle
            .link()
            .set(
                rtnetlink::LinkUnspec::new_with_index(identity.index)
                    .alias("foreign-owner")
                    .build(),
            )
            .execute(),
    )
    .await
    .unwrap();
    let failure = guard.verify().await.unwrap_err();
    // The anchor's alias no longer matches: the failure names the check and
    // what showed it (the dump, or the monitor ending kernel ownership if it
    // saw the change first).
    match failure.check {
        "anchor_link" => assert_eq!(
            failure.source.as_deref(),
            Some("rtnetlink RTM_GETLINK"),
            "{failure}"
        ),
        "anchor_ended" => assert!(
            failure
                .actual
                .as_deref()
                .is_some_and(|actual| actual.contains("kernel ownership ended")),
            "{failure}"
        ),
        _ => panic!("unexpected check: {failure}"),
    }
    assert!(kernel.remove_dns_anchor().await.is_err());
    let links = collect(
        kernel
            .handle
            .link()
            .get()
            .match_name(identity.name)
            .execute(),
        kernel.options.max_kernel_objects,
    )
    .await
    .unwrap();
    assert_eq!(links.len(), 1);
    assert!(links[0].attributes.iter().any(|attribute| matches!(attribute, rtnetlink::packet_route::link::LinkAttribute::IfAlias(value) if value == "foreign-owner")));
    drop(kernel);
    tasks.shutdown().await.unwrap();
}

async fn assert_dns_anchor_installation(kernel: &Kernel, identity: &meshway_dns::DnsLinkIdentity) {
    use rtnetlink::packet_route::address::AddressScope;
    use rtnetlink::packet_route::link::{LinkAttribute, LinkFlags};

    // An independent kernel readback catches addrgenmode silently omitted from
    // NEWLINK, as well as the original pre-registration AF_SPEC rejection.
    let mode = std::fs::read_to_string(format!(
        "/proc/sys/net/ipv6/conf/{}/addr_gen_mode",
        identity.name
    ))
    .unwrap();
    assert_eq!(
        mode.trim(),
        "1",
        "anchor must disable IPv6 autoconfiguration"
    );
    let links = collect(
        kernel.handle.link().get().execute(),
        kernel.options.max_kernel_objects,
    )
    .await
    .unwrap();
    let link = links
        .iter()
        .find(|link| link.header.index == identity.index)
        .unwrap();
    assert!(
        link.header
            .flags
            .contains(LinkFlags::Up | LinkFlags::LowerUp)
    );
    let alias = format!("meshway-dns:{}", identity.installation);
    assert!(link.attributes.iter().any(|attribute| {
        matches!(attribute, LinkAttribute::IfAlias(value) if value == &alias)
    }));
    let addresses = get_addresses(&kernel.handle, kernel.options.max_kernel_objects)
        .await
        .unwrap();
    let addresses: Vec<_> = addresses
        .iter()
        .filter(|address| address.header.index == identity.index)
        .collect();
    assert_eq!(addresses.len(), 1, "no auto-generated IPv6 identity");
    assert_eq!(addresses[0].header.family, AddressFamily::Inet);
    assert_eq!(addresses[0].header.scope, AddressScope::Universe);
    assert_eq!(addresses[0].header.prefix_len, 32);
    assert!(addresses[0].attributes.iter().any(|attribute| {
        matches!(attribute, AddressAttribute::Local(IpAddr::V4(ip)) if *ip == identity.address)
    }));
}

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

/// The harness must provide a remote dual-stack TCP listener in an isolated
/// namespace. It is deliberately not a loopback listener (local RPDB wins).
#[tokio::test]
#[ignore = "isolated Linux NET_ADMIN fixture with remote IPv4/IPv6 TCP peers required"]
async fn negative_capture_marked_bypass_recovery_and_exact_down() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let peers: Vec<std::net::SocketAddr> = ["MESHWAY_UNDERLAY_PEER4", "MESHWAY_UNDERLAY_PEER6"]
        .into_iter()
        .map(|key| {
            std::env::var(key)
                .expect("remote peer fixture")
                .parse()
                .unwrap()
        })
        .collect();
    assert!(peers[0].is_ipv4() && peers[1].is_ipv6());
    assert!(peers.iter().all(|peer| !peer.ip().is_loopback()));
    let temp = tempfile::tempdir().unwrap();
    let directory =
        Arc::new(Directory::open(&temp.path().join("private"), "fixture.lock").unwrap());
    let options = Options::default();
    let tasks = Tasks::new(options.max_sockets * 4 + 32);
    let mut kernel = Kernel::open(directory.clone(), options.clone(), tasks.clone())
        .await
        .unwrap();
    let pools: Vec<_> = peers
        .iter()
        .map(|peer| ipnet::IpNet::new(peer.ip(), if peer.is_ipv4() { 32 } else { 128 }).unwrap())
        .collect();
    kernel.retain_negative(&pools[..1]).await.unwrap();
    kernel.retain_negative(&pools[1..]).await.unwrap();
    assert_eq!(kernel.journal.pools, pools, "old pools kept");
    for peer in &peers {
        let unmarked =
            tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(peer))
                .await
                .unwrap();
        assert!(
            unmarked.is_err(),
            "ordinary traffic to the same endpoint must hit the sink"
        );
        let stream = super::super::sockets::connect_tcp(
            kernel.shared.bootstrap(),
            *peer,
            None,
            &Default::default(),
            &Default::default(),
            Duration::from_secs(3),
        )
        .await
        .unwrap();
        drop(stream);
    }
    let raw = tokio::net::TcpSocket::new_v4().unwrap();
    let outside = (!kernel.shared.mark.mask).trailing_zeros();
    let outside = 1u32 << outside;
    socket2::SockRef::from(&raw).set_mark(outside).unwrap();
    kernel
        .shared
        .prepare(&socket2::SockRef::from(&raw))
        .unwrap();
    assert_eq!(
        socket2::SockRef::from(&raw).mark().unwrap() & !kernel.shared.mark.mask,
        outside
    );
    kernel.suspend().await.unwrap();
    assert!(kernel.tun.is_none());
    assert!(!kernel.journal.rules.is_empty());
    tasks.shutdown().await.unwrap();
    drop(kernel);

    let tasks = Tasks::new(options.max_sockets * 4 + 32);
    let mut recovered = Kernel::open(directory, options, tasks.clone())
        .await
        .unwrap();
    assert!(
        recovered.shared.execution.read().unwrap().is_none(),
        "journal never restores authority"
    );
    for peer in &peers {
        assert!(
            tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(peer))
                .await
                .unwrap()
                .is_err()
        );
    }
    recovered.down().await.unwrap();
    for peer in &peers {
        drop(
            tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(peer))
                .await
                .unwrap()
                .unwrap(),
        );
    }
    tasks.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "isolated Linux NET_ADMIN fixture required"]
async fn foreign_same_priority_is_retained_and_blocks_cleanup() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let temp = tempfile::tempdir().unwrap();
    let directory =
        Arc::new(Directory::open(&temp.path().join("private"), "fixture.lock").unwrap());
    let options = Options::default();
    let tasks = Tasks::new(options.max_sockets * 4 + 32);
    let mut kernel = Kernel::open(directory.clone(), options.clone(), tasks.clone())
        .await
        .unwrap();
    kernel
        .retain_negative(&["198.18.255.254/32".parse().unwrap()])
        .await
        .unwrap();
    let mut foreign = RuleSpec {
        ipv6: false,
        priority: kernel.journal.start + 1,
        table: 254,
        kind: RuleKind::End,
    }
    .message();
    foreign
        .attributes
        .retain(|a| !matches!(a, RuleAttribute::Protocol(_)));
    foreign.attributes.push(RuleAttribute::Protocol(243.into()));
    let mut request = kernel.handle.rule().add();
    *request.message_mut() = foreign.clone();
    request.execute().await.unwrap();
    assert!(kernel.down().await.is_err());
    let rules = get_rules(&kernel.handle, 16384).await.unwrap();
    assert!(
        rules
            .iter()
            .any(|rule| encode_rule(rule.clone()) == encode_rule(foreign.clone()))
    );
    // The test owns this one foreign fixture object; no production cleanup path
    // is allowed to remove it. Remove exactly that fixture before reopening.
    kernel.handle.rule().del(foreign).execute().await.unwrap();
    tasks.shutdown().await.unwrap();
    drop(kernel);
    let tasks = Tasks::new(options.max_sockets * 4 + 32);
    let mut recovered = Kernel::open(directory, options, tasks.clone())
        .await
        .unwrap();
    recovered.down().await.unwrap();
    tasks.shutdown().await.unwrap();
}
