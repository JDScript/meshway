use super::*;
use crate::linux::{
    kernel::{Kernel, TunRecord, collect, execute, get_routes, plan::encode_route},
    tasks::Tasks,
};
use meshway_private_storage::Directory;
use rtnetlink::RouteMessageBuilder;
use std::{sync::Arc, time::Duration};

fn local(ip: IpAddr, index: u32) -> RouteMessage {
    let mut route = match ip {
        IpAddr::V4(ip) => RouteMessageBuilder::<Ipv4Addr>::new()
            .destination_prefix(ip, 32)
            .build(),
        IpAddr::V6(ip) => RouteMessageBuilder::<Ipv6Addr>::new()
            .destination_prefix(ip, 128)
            .build(),
    };
    route.header.table = 255;
    route.header.kind = RouteType::Local;
    route.header.protocol = RouteProtocol::Kernel;
    route.header.scope = if ip.is_ipv4() {
        RouteScope::Host
    } else {
        RouteScope::Universe
    };
    route.attributes.push(RouteAttribute::Oif(index));
    route
}

fn foreign_local(ip: IpAddr, index: u32) -> RouteMessage {
    let mut route = local(ip, index);
    route.header.protocol = RouteProtocol::Static;
    // Linux supplies the user-route default metric when IPv6 metric is zero.
    // Specify it explicitly so deletion can use the exact created message.
    if ip.is_ipv6() {
        route.attributes.push(RouteAttribute::Priority(1024));
    }
    route
}

#[test]
fn only_exact_owned_host_routes_escape_conflict_checks() {
    for (pool, ip, peer, outside) in [
        ("100.96.0.0/16", "100.96.0.1", "100.96.0.2", "192.0.2.1"),
        (
            "fd12:3456::/64",
            "fd12:3456::1",
            "fd12:3456::2",
            "2001:db8::1",
        ),
    ] {
        let pools = [pool.parse().unwrap()];
        let ip: IpAddr = ip.parse().unwrap();
        let addresses = [ip];
        let owned = Some(Owned {
            index: 7,
            addresses: &addresses,
        });
        let original = local(ip, 7);
        assert!(!route_conflicts(&pools, owned, &original));
        assert!(route_conflicts(&pools, None, &original));
        assert!(route_conflicts(&pools, owned, &local(ip, 8)));
        assert!(route_conflicts(
            &pools,
            owned,
            &local(peer.parse().unwrap(), 7)
        ));
        assert!(!route_conflicts(
            &pools,
            owned,
            &local(outside.parse().unwrap(), 8)
        ));
        for mutation in 0..7 {
            let mut route = original.clone();
            match mutation {
                0 => route.header.protocol = RouteProtocol::Static,
                1 => route.header.destination_prefix_length -= 1,
                2 => route.header.kind = RouteType::Unicast,
                3 => route.attributes.push(RouteAttribute::Gateway(match ip {
                    IpAddr::V4(ip) => RouteAddress::Inet(ip),
                    IpAddr::V6(ip) => RouteAddress::Inet6(ip),
                })),
                4 => route.header.source_prefix_length = 1,
                5 => route.attributes.push(RouteAttribute::Oif(7)),
                6 => {
                    route.header.destination_prefix_length = 0;
                    route
                        .attributes
                        .retain(|a| !matches!(a, RouteAttribute::Destination(_)));
                }
                _ => unreachable!(),
            }
            assert!(
                route_conflicts(&pools, owned, &route),
                "mutation {mutation}"
            );
        }
        let mut route = local(peer.parse().unwrap(), 7);
        route.header.table = 254;
        assert!(
            !route_conflicts(&pools, owned, &route),
            "ordinary main-table route follows capture"
        );
        route.header.table = 255;
        route.header.kind = RouteType::Throw;
        assert!(!route_conflicts(&pools, owned, &route));

        let mut address = AddressMessage::default();
        address.header.family = family(ip);
        address.header.index = 7;
        address.header.prefix_len = host_prefix(ip);
        address.header.scope = AddressScope::Universe;
        address.attributes.push(AddressAttribute::Address(ip));
        assert!(!address_conflicts(&pools, owned, &address));
        address.header.index = 8;
        assert!(address_conflicts(&pools, owned, &address));
        address.header.index = 7;
        address
            .attributes
            .push(AddressAttribute::Local(peer.parse().unwrap()));
        assert!(address_conflicts(&pools, owned, &address));
    }
}

/// This fixture mutates only its isolated netns, using exact cleanup messages.
#[tokio::test]
#[ignore = "requires isolated Linux netns, /dev/net/tun, IPv6 and NET_ADMIN; MESHWAY_UNDERLAY_ISOLATED=1"]
async fn local_route_preflight_and_live_address_conflicts_preserve_foreign_resources() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    for ipv6 in [false, true] {
        for address_conflict in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let directory =
                Arc::new(Directory::open(&temp.path().join("owner"), "fixture.lock").unwrap());
            let tasks = Tasks::new(64);
            let mut kernel = Kernel::open(directory.clone(), Default::default(), tasks.clone())
                .await
                .unwrap();
            let pools = vec![
                if ipv6 {
                    "fd12:3456::/64"
                } else {
                    "100.96.0.0/16"
                }
                .parse()
                .unwrap(),
            ];
            let ip: IpAddr = if ipv6 { "fd12:3456::1" } else { "100.96.0.1" }
                .parse()
                .unwrap();
            let peer: IpAddr = if ipv6 { "fd12:3456::2" } else { "100.96.0.2" }
                .parse()
                .unwrap();
            let loopback = collect(kernel.handle.link().get().match_name("lo").execute(), 1024)
                .await
                .unwrap()[0]
                .header
                .index;
            let foreign = foreign_local(peer, loopback);
            execute(kernel.handle.route().add(foreign.clone()).execute())
                .await
                .unwrap();
            // A local route without a corresponding interface address used to
            // pass installation preflight. This is the very production helper.
            assert!(matches!(
                kernel.verify_capture(&pools).await,
                Err(crate::Error::Conflict)
            ));
            let actual = get_routes(&kernel.handle, 1024)
                .await
                .unwrap()
                .into_iter()
                .find(|r| encode_route(r.clone()) == encode_route(foreign.clone()))
                .unwrap();
            execute(kernel.handle.route().del(actual).execute())
                .await
                .unwrap();
            kernel.verify_capture(&pools).await.unwrap();

            // Real kernel-generated local routes for an owned TUN are allowed,
            // including after its tentative IPv6 address has become ready.
            let addresses = meshway_configuration::NodeAddresses {
                ipv4: if let IpAddr::V4(ip) = ip {
                    Some(ip)
                } else {
                    None
                },
                ipv6: if let IpAddr::V6(ip) = ip {
                    Some(ip)
                } else {
                    None
                },
            };
            let options = match ip {
                IpAddr::V4(ip) => meshway_tun::Options::new(1280)
                    .unwrap()
                    .with_ipv4(ip, 32)
                    .unwrap(),
                IpAddr::V6(ip) => meshway_tun::Options::new(1280)
                    .unwrap()
                    .with_ipv6(ip, 128)
                    .unwrap(),
            };
            {
                let mut expected = kernel.expected.lock().unwrap();
                expected.changing = true;
                let tun = meshway_tun::Tun::create(options).unwrap();
                let index = tun.if_index().unwrap();
                expected.tun = Some((index, tun.name().into(), tun.mtu(), vec![ip]));
                kernel.journal.tun = Some(TunRecord {
                    name: tun.name().into(),
                    ifindex: index,
                    mtu: tun.mtu(),
                    addresses: addresses.clone(),
                });
                kernel.tun = Some(tun);
            }
            kernel
                .address_ready(kernel.tun.as_ref().unwrap().if_index().unwrap(), &addresses)
                .await
                .unwrap();
            kernel.quiesce().await.unwrap();
            kernel.expected.lock().unwrap().changing = false;
            kernel.retain_negative(&pools).await.unwrap();
            // verify() is also used by retained-TUN handoff and explicit down.
            kernel.verify().await.unwrap();
            let outside = foreign_local(
                if ipv6 { "2001:db8::53" } else { "192.0.2.53" }
                    .parse()
                    .unwrap(),
                loopback,
            );
            execute(kernel.handle.route().add(outside.clone()).execute())
                .await
                .unwrap();
            kernel.quiesce().await.unwrap();
            kernel.verify().await.unwrap();
            execute(kernel.handle.route().del(outside).execute())
                .await
                .unwrap();

            if address_conflict {
                execute(
                    kernel
                        .handle
                        .address()
                        .add(loopback, peer, host_prefix(peer))
                        .execute(),
                )
                .await
                .unwrap();
            } else {
                execute(kernel.handle.route().add(foreign.clone()).execute())
                    .await
                    .unwrap();
            }
            tokio::time::timeout(Duration::from_secs(3), kernel.shared.conflict.cancelled())
                .await
                .unwrap();
            assert!(kernel.verify().await.is_err());
            assert!(kernel.down().await.is_err());
            // Only this test's fixture owner removes the exact foreign object.
            if address_conflict {
                let address = super::super::get_addresses(&kernel.handle, 1024).await.unwrap().into_iter().find(|a| a.header.index == loopback && a.attributes.iter().any(|a| matches!(a, AddressAttribute::Address(ip) | AddressAttribute::Local(ip) if *ip == peer))).unwrap();
                execute(kernel.handle.address().del(address).execute())
                    .await
                    .unwrap();
            } else {
                let route = get_routes(&kernel.handle, 1024)
                    .await
                    .unwrap()
                    .into_iter()
                    .find(|r| encode_route(r.clone()) == encode_route(foreign.clone()))
                    .expect("production down preserved foreign route");
                execute(kernel.handle.route().del(route).execute())
                    .await
                    .unwrap();
            }
            drop(kernel);
            tasks.shutdown().await.unwrap();
            let tasks = Tasks::new(64);
            let mut recovered = Kernel::open(directory, Default::default(), tasks.clone())
                .await
                .unwrap();
            recovered.down().await.unwrap();
            drop(recovered);
            tasks.shutdown().await.unwrap();
        }
    }
}
