//! Kernel conformance of the roaming slice: change classification, capture
//! objects a network manager removes, and a network address inside a pool.
use super::*;
use std::time::Duration;

fn isolated() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
}

async fn foreign() -> (Handle, tokio::task::JoinHandle<()>) {
    let (connection, handle) = rtnl::connect().unwrap();
    (handle, tokio::spawn(connection))
}

async fn dummy(handle: &Handle, name: &str) -> u32 {
    handle
        .link()
        .add(rtnetlink::LinkDummy::new(name).up().build())
        .execute()
        .await
        .unwrap();
    use futures_util::TryStreamExt;
    handle
        .link()
        .get()
        .match_name(name)
        .execute()
        .try_next()
        .await
        .unwrap()
        .unwrap()
        .header
        .index
}

/// Wait until no change was classified for a full coalescing period and more.
async fn settled(changes: &mut tokio::sync::watch::Receiver<crate::NetworkChange>) {
    loop {
        changes.borrow_and_update();
        if tokio::time::timeout(Duration::from_millis(800), changes.changed())
            .await
            .is_err()
        {
            return;
        }
    }
}

#[tokio::test]
#[ignore = "isolated Linux NET_ADMIN namespace required"]
async fn external_changes_are_classified_as_path_or_refresh() {
    isolated();
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(Directory::open(&temp.path().join("node"), "node.lock").unwrap());
    let host = crate::Host::open(directory, Options::default())
        .await
        .unwrap();
    let mut changes = host.network_changes();
    let (handle, task) = foreign().await;
    // A new up interface with carrier changes the path fingerprint.
    let index = dummy(&handle, "r1-roam").await;
    tokio::time::timeout(Duration::from_secs(3), changes.wait_for(|c| c.path >= 1))
        .await
        .unwrap()
        .unwrap();
    settled(&mut changes).await;
    let before = *changes.borrow();
    assert!(before.class.is_some());
    // A link-scope address is no preferred global address: a refresh.
    let mut add = handle
        .address()
        .add(index, "169.254.77.1".parse().unwrap(), 16);
    add.message_mut().header.scope = rtnetlink::packet_route::address::AddressScope::Link;
    add.execute().await.unwrap();
    tokio::time::timeout(
        Duration::from_secs(3),
        changes.wait_for(|c| c.epoch > before.epoch),
    )
    .await
    .unwrap()
    .unwrap();
    settled(&mut changes).await;
    let refreshed = *changes.borrow();
    assert_eq!(
        refreshed.path, before.path,
        "a link-scope address is a refresh"
    );
    assert_eq!(refreshed.class, Some(crate::NetworkChangeClass::Refresh));
    // A global address on an external up interface is a path change.
    handle
        .address()
        .add(index, "192.0.2.77".parse().unwrap(), 24)
        .execute()
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(3),
        changes.wait_for(|c| c.path > refreshed.path),
    )
    .await
    .unwrap()
    .unwrap();
    host.kernel.shared.check().unwrap();
    handle.link().del(index).execute().await.unwrap();
    host.shutdown(false).await.unwrap();
    task.abort();
}

/// Wait for the next classified change after `before` and return its class.
async fn next_class(
    changes: &mut tokio::sync::watch::Receiver<crate::NetworkChange>,
    what: &str,
) -> crate::NetworkChangeClass {
    let before = changes.borrow_and_update().epoch;
    let change = *tokio::time::timeout(
        Duration::from_secs(3),
        changes.wait_for(|change| change.epoch > before),
    )
    .await
    .unwrap_or_else(|_| panic!("no change classified: {what}"))
    .unwrap();
    settled(changes).await;
    change.class.unwrap()
}

/// The VM case of OPS13: the Server is reached over IPv6 (unchanged), the
/// peer over IPv4; a second IPv4 address (IFA_F_SECONDARY) and a default
/// route source moved to it are path changes. An IPv6 temporary address
/// (IFA_F_TEMPORARY, the same flag bit) and a route toward no reference are
/// refreshes.
#[tokio::test]
#[ignore = "isolated Linux NET_ADMIN namespace required"]
async fn addresses_by_family_and_routes_toward_remotes_classify_the_path() {
    use rtnetlink::{
        RouteMessageBuilder,
        packet_route::address::{AddressAttribute, AddressFlags},
    };
    use std::net::{Ipv4Addr, Ipv6Addr};
    isolated();
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(Directory::open(&temp.path().join("node"), "node.lock").unwrap());
    let host = crate::Host::open(directory, Options::default())
        .await
        .unwrap();
    let mut changes = host.network_changes();
    let (handle, task) = foreign().await;
    let index = dummy(&handle, "r3-roam").await;
    tokio::time::timeout(Duration::from_secs(3), changes.wait_for(|c| c.path >= 1))
        .await
        .unwrap()
        .unwrap();
    settled(&mut changes).await;
    let primary: Ipv4Addr = "192.0.2.77".parse().unwrap();
    let secondary: Ipv4Addr = "192.0.2.78".parse().unwrap();
    let gateway: Ipv4Addr = "192.0.2.1".parse().unwrap();
    handle
        .address()
        .add(index, primary.into(), 24)
        .execute()
        .await
        .unwrap();
    assert_eq!(
        next_class(&mut changes, "primary IPv4 address").await,
        crate::NetworkChangeClass::Path
    );
    // The Server over IPv6, on the same interface; its route never changes.
    let mut add = handle
        .address()
        .add(index, "2001:db8:1::7".parse().unwrap(), 64);
    add.message_mut()
        .attributes
        .push(AddressAttribute::Flags(AddressFlags::Nodad));
    add.execute().await.unwrap();
    assert_eq!(
        next_class(&mut changes, "IPv6 address").await,
        crate::NetworkChangeClass::Path
    );
    host.kernel
        .shared
        .network
        .control_reached("2001:db8:1::443".parse().unwrap());
    // The peer over IPv4, its route sourced from the primary address.
    let toward_peer = |source: Ipv4Addr| {
        RouteMessageBuilder::<Ipv4Addr>::new()
            .destination_prefix("198.51.100.0".parse().unwrap(), 24)
            .gateway(gateway)
            .pref_source(source)
            .output_interface(index)
            .build()
    };
    handle
        .route()
        .add(toward_peer(primary))
        .execute()
        .await
        .unwrap();
    next_class(&mut changes, "route toward the peer's prefix").await;
    host.sockets()
        .observe_remotes(["198.51.100.9".parse::<IpAddr>().unwrap()]);
    tokio::time::sleep(Duration::from_millis(300)).await;
    // IFA_F_SECONDARY: a second address of the same IPv4 prefix is effective.
    handle
        .address()
        .add(index, secondary.into(), 24)
        .execute()
        .await
        .unwrap();
    assert_eq!(
        next_class(&mut changes, "secondary IPv4 address").await,
        crate::NetworkChangeClass::Path,
        "an IPv4 secondary address is an effective address"
    );
    // Only the IPv4 source toward the peer moves; the IPv6 route to the
    // Server, links and addresses are as they were.
    handle
        .route()
        .add(toward_peer(secondary))
        .replace()
        .execute()
        .await
        .unwrap();
    assert_eq!(
        next_class(&mut changes, "IPv4 source toward the peer").await,
        crate::NetworkChangeClass::Path,
        "the marked IPv4 route toward a remote is part of the path"
    );
    // A route toward no reference destination.
    handle
        .route()
        .add(
            RouteMessageBuilder::<Ipv4Addr>::new()
                .destination_prefix("203.0.113.0".parse().unwrap(), 24)
                .gateway(gateway)
                .output_interface(index)
                .build(),
        )
        .execute()
        .await
        .unwrap();
    assert_eq!(
        next_class(&mut changes, "route toward no reference").await,
        crate::NetworkChangeClass::Refresh
    );
    // IFA_F_TEMPORARY: the kernel derives a privacy address from a managed
    // one (`use_tempaddr` of the fixture namespace); the managed address is
    // a path change, the temporary one's removal only a refresh. On its own
    // interface: a temporary address is the preferred source on its
    // interface, so there it would be the Server route's real source.
    let temporaries = dummy(&handle, "r3-temp").await;
    assert_eq!(
        next_class(&mut changes, "second interface").await,
        crate::NetworkChangeClass::Path
    );
    let mut add = handle
        .address()
        .add(temporaries, "2001:db8:2::1".parse().unwrap(), 64);
    add.message_mut().attributes.push(AddressAttribute::Flags(
        AddressFlags::Nodad | AddressFlags::Managetempaddr,
    ));
    add.execute().await.unwrap();
    assert_eq!(
        next_class(&mut changes, "managed IPv6 address").await,
        crate::NetworkChangeClass::Path
    );
    let temporary = {
        use futures_util::TryStreamExt;
        let mut addresses = handle
            .address()
            .get()
            .set_link_index_filter(temporaries)
            .execute();
        let mut found = None;
        while let Some(address) = addresses.try_next().await.unwrap() {
            let flags = address
                .attributes
                .iter()
                .find_map(|attribute| match attribute {
                    AddressAttribute::Flags(flags) => Some(flags.bits()),
                    _ => None,
                });
            let in_prefix = address.attributes.iter().any(|attribute| {
                matches!(attribute, AddressAttribute::Address(IpAddr::V6(ip))
                    if ip.segments()[..4] == "2001:db8:2::".parse::<Ipv6Addr>().unwrap().segments()[..4])
            });
            if in_prefix && flags.is_some_and(|flags| flags & 0x01 != 0) {
                found = Some(address);
            }
        }
        found.expect("the kernel created a temporary address (use_tempaddr=2)")
    };
    handle.address().del(temporary).execute().await.unwrap();
    assert_eq!(
        next_class(&mut changes, "temporary IPv6 address removed").await,
        crate::NetworkChangeClass::Refresh,
        "an IPv6 temporary address is not followed"
    );
    host.kernel.shared.check().unwrap();
    handle.link().del(temporaries).execute().await.unwrap();
    handle.link().del(index).execute().await.unwrap();
    host.shutdown(false).await.unwrap();
    task.abort();
}

#[tokio::test]
#[ignore = "isolated Linux NET_ADMIN namespace required"]
async fn own_capture_rules_removed_outside_their_owner_are_reinstalled_boundedly() {
    isolated();
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(Directory::open(&temp.path().join("node"), "node.lock").unwrap());
    let mut host = crate::Host::open(directory, Options::default())
        .await
        .unwrap();
    let pools = ["100.96.0.0/16".parse().unwrap()];
    host.kernel.retain_negative(&pools).await.unwrap();
    host.kernel.quiesce().await.unwrap();
    let mut repairs = host.repairs();
    repairs.borrow_and_update();
    let (handle, task) = foreign().await;
    let flush = async |host: &crate::Host| {
        // A network manager removing every rule it does not configure.
        let owned: Vec<_> = host
            .kernel
            .journal
            .rules
            .iter()
            .map(|record| record.observed.clone())
            .collect();
        for rule in get_rules(&handle, 65_536).await.unwrap() {
            if owned.contains(&encode_rule(rule.clone())) {
                handle.rule().del(rule).execute().await.unwrap();
            }
        }
    };
    for _ in 0..3 {
        flush(&host).await;
        tokio::time::timeout(Duration::from_secs(3), repairs.changed())
            .await
            .unwrap()
            .unwrap();
        repairs.borrow_and_update();
        host.repair().await.unwrap();
        host.kernel.verify().await.unwrap();
        host.kernel.shared.check().unwrap();
    }
    // The fourth removal within a minute is no longer a cleanup to undo.
    flush(&host).await;
    tokio::time::timeout(Duration::from_secs(3), repairs.changed())
        .await
        .unwrap()
        .unwrap();
    assert!(host.repair().await.is_err());
    assert!(host.kernel.shared.check().is_err());
    task.abort();
}

/// F6a B1: an own rule removed outside its owner (the end rule other own
/// rules jump to, which leaves them unresolved) whose first repair read
/// fails is still owed: the repair is due again, and its next attempt
/// reinstalls the rule alone. Reads that keep failing end ownership as
/// unavailable instead of leaving a healthy-looking owner without the rule.
#[tokio::test]
#[ignore = "isolated Linux NET_ADMIN namespace required"]
async fn a_repair_whose_read_fails_stays_owed_and_completes() {
    use crate::linux::kernel::tool::command::faults::FAILING_RULE_READS;
    use std::sync::atomic::Ordering::SeqCst;
    isolated();
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(Directory::open(&temp.path().join("node"), "node.lock").unwrap());
    let mut host = crate::Host::open(directory, Options::default())
        .await
        .unwrap();
    let pools = ["100.96.0.0/16".parse().unwrap()];
    host.kernel.retain_negative(&pools).await.unwrap();
    host.kernel.quiesce().await.unwrap();
    let mut repairs = host.repairs();
    repairs.borrow_and_update();
    let (handle, task) = foreign().await;
    let remove_one = async |host: &crate::Host| {
        let owned = host.kernel.journal.rules[0].observed.clone();
        for rule in get_rules(&handle, 65_536).await.unwrap() {
            if encode_rule(rule.clone()) == owned {
                handle.rule().del(rule).execute().await.unwrap();
            }
        }
    };
    remove_one(&host).await;
    tokio::time::timeout(Duration::from_secs(3), repairs.changed())
        .await
        .unwrap()
        .unwrap();
    repairs.borrow_and_update();
    FAILING_RULE_READS.store(1, SeqCst);
    assert!(host.repair().await.is_err());
    FAILING_RULE_READS.store(0, SeqCst);
    assert!(host.ownership_ended().is_none());
    let due = host.repair_at().expect("the failed repair stays owed");
    assert!(due > std::time::Instant::now());
    assert!(
        host.kernel.verify().await.is_err(),
        "the rule is still missing"
    );
    host.repair().await.unwrap();
    assert!(host.repair_at().is_none());
    host.kernel.verify().await.unwrap();
    host.kernel.shared.check().unwrap();

    // Reads failing on every attempt: the owner ends as unavailable.
    remove_one(&host).await;
    tokio::time::timeout(Duration::from_secs(3), repairs.changed())
        .await
        .unwrap()
        .unwrap();
    FAILING_RULE_READS.store(u32::MAX, SeqCst);
    for _ in 1..super::super::MAX_REPAIR_READS {
        assert!(host.repair().await.is_err());
        assert!(host.repair_at().is_some());
    }
    assert!(host.repair().await.is_err());
    FAILING_RULE_READS.store(0, SeqCst);
    assert_eq!(
        host.ownership_ended(),
        Some(crate::OwnershipEnd::Unavailable {
            check: "repair_unavailable"
        })
    );
    assert!(host.repair_at().is_none());
    assert!(host.recovery_at().is_some());
    task.abort();
}

#[tokio::test]
#[ignore = "isolated Linux NET_ADMIN namespace required"]
async fn an_external_address_inside_a_mesh_pool_fails_closed_until_it_leaves() {
    isolated();
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(Directory::open(&temp.path().join("node"), "node.lock").unwrap());
    let mut host = crate::Host::open(directory, Options::default())
        .await
        .unwrap();
    let pools = ["100.96.0.0/16".parse().unwrap()];
    host.kernel.retain_negative(&pools).await.unwrap();
    host.kernel.quiesce().await.unwrap();
    let mut overlap = host.pool_overlap();
    let (handle, task) = foreign().await;
    let index = dummy(&handle, "r1-overlap").await;
    handle
        .address()
        .add(index, "100.96.9.9".parse().unwrap(), 32)
        .execute()
        .await
        .unwrap();
    let found = tokio::time::timeout(Duration::from_secs(3), overlap.wait_for(Option::is_some))
        .await
        .unwrap()
        .unwrap()
        .clone()
        .unwrap();
    assert_eq!(found.address, "100.96.9.9".parse::<IpAddr>().unwrap());
    assert_eq!(found.interface, "r1-overlap");
    // Not a permanent conflict: installation refuses, removal proceeds.
    host.kernel.shared.check().unwrap();
    assert!(host.kernel.verify_capture(&pools).await.is_err());
    host.kernel.verify().await.unwrap();
    handle
        .address()
        .del({
            use futures_util::TryStreamExt;
            let mut addresses = handle
                .address()
                .get()
                .set_link_index_filter(index)
                .execute();
            let mut found = None;
            while let Some(address) = addresses.try_next().await.unwrap() {
                let pool = address.attributes.iter().any(|attribute| {
                    matches!(attribute, rtnetlink::packet_route::address::AddressAttribute::Local(ip)
                        if *ip == "100.96.9.9".parse::<IpAddr>().unwrap())
                });
                if pool {
                    found = Some(address);
                }
            }
            found.unwrap()
        })
        .execute()
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), overlap.wait_for(Option::is_none))
        .await
        .unwrap()
        .unwrap();
    host.kernel.verify_capture(&pools).await.unwrap();
    handle.link().del(index).execute().await.unwrap();
    host.down().await.unwrap();
    host.shutdown(false).await.unwrap();
    task.abort();
}

/// F6a B7: a new ownership term reads the pool overlap as it is now. An
/// external address still inside a retained pool opens the term failed
/// closed on that overlap (not a conflict); an overlap that left while no
/// term watched is withdrawn by the next term's publication.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "isolated Linux NET_ADMIN namespace required"]
async fn a_new_ownership_term_publishes_the_pool_overlap_it_reads() {
    use crate::linux::kernel::tool::command::faults::FAILING_RULE_READS;
    use std::sync::atomic::Ordering::SeqCst;
    isolated();
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(Directory::open(&temp.path().join("node"), "node.lock").unwrap());
    let mut host = crate::Host::open(directory, Options::default())
        .await
        .unwrap();
    let pools = ["100.96.0.0/16".parse().unwrap()];
    host.kernel.retain_negative(&pools).await.unwrap();
    host.kernel.quiesce().await.unwrap();
    let mut overlap = host.pool_overlap();
    let (handle, task) = foreign().await;
    let index = dummy(&handle, "b7-overlap").await;
    handle
        .address()
        .add(index, "100.96.9.9".parse().unwrap(), 32)
        .execute()
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), overlap.wait_for(Option::is_some))
        .await
        .unwrap()
        .unwrap();
    let unavailable = crate::OwnershipEnd::Unavailable {
        check: "verify_unavailable",
    };

    // The address stays: the new term holds and starts on the overlap.
    host.kernel.shared.end(unavailable);
    host.recover().await.unwrap();
    assert!(host.ownership_ended().is_none());
    let found = overlap.borrow_and_update().clone().unwrap();
    assert_eq!(found.address, "100.96.9.9".parse::<IpAddr>().unwrap());
    assert!(host.kernel.verify_capture(&pools).await.is_err());
    host.kernel.verify().await.unwrap();
    host.kernel.shared.check().unwrap();

    // Ended again; the first re-proof (unreadable) closes the term and its
    // monitor, then the address leaves unobserved.
    host.kernel.shared.end(unavailable);
    FAILING_RULE_READS.store(1, SeqCst);
    assert!(host.recover().await.is_err());
    FAILING_RULE_READS.store(0, SeqCst);
    handle.link().del(index).execute().await.unwrap();
    assert!(overlap.borrow().is_some(), "no term observed the departure");
    host.recover().await.unwrap();
    assert!(overlap.borrow_and_update().is_none());
    host.kernel.verify_capture(&pools).await.unwrap();
    host.down().await.unwrap();
    host.shutdown(false).await.unwrap();
    task.abort();
}
