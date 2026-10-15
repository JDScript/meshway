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
    let (connection, handle, unsolicited) = rtnetlink::new_connection().unwrap();
    drop(unsolicited);
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
