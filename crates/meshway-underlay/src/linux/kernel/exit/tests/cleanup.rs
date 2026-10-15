//! X11: a failed or abandoned read proves nothing about the exit role's
//! ownership, and a failed cleanup is retried from what the kernel holds.
use super::super::super::verification::{Owner, Unverified, Verification};
use super::*;
use crate::Error;
use std::sync::atomic::Ordering::SeqCst;

/// A projection-free exit backend on a fresh TUN, as the owner creates it.
async fn create(kernel: &mut Kernel, tun: &meshway_tun::Tun) -> backend::Backend {
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).unwrap();
    let mark = if kernel.journal.mark.mask != 1 { 1 } else { 2 };
    let namespace = crate::linux::kernel::namespace::Identity::current().unwrap();
    let (store, previous) = storage::Store::open(kernel.store.directory())
        .await
        .unwrap();
    // A second installation follows a removed one only.
    assert!(previous.is_none_or(|ledger| ledger.removed));
    let ledger = storage::Ledger {
        version: 2,
        boot: namespace.boot,
        namespace_device: namespace.device,
        namespace_inode: namespace.inode,
        namespace_cookie: namespace.cookie,
        base_installation: kernel.journal.installation.clone(),
        network: "11111111-1111-4111-8111-111111111111".parse().unwrap(),
        node: "22222222-2222-4222-8222-222222222222".parse().unwrap(),
        plan: crate::target::RoleIdentity {
            generation: Digest::from_bytes([1; 32]),
            family: ExitFamily::Ipv4,
            nat: ExitNat::Masquerade,
        },
        program: program::Program {
            legacy: false,
            installation: nonce.iter().map(|byte| format!("{byte:02x}")).collect(),
            interface: tun.if_index().unwrap(),
            interface_name: tun.name().to_owned(),
            mark,
            pools: vec!["100.96.0.0/11".parse().unwrap()],
        },
        tun_name: tun.name().to_owned(),
        mtu: tun.mtu(),
        route_table: 20000,
        rules: Vec::new(),
        routes: Vec::new(),
        receipt: None,
        projections: Vec::new(),
        quarantine: Vec::new(),
        legacy_quarantine: Vec::new(),
        closing: false,
        removed: false,
        pending: true,
    };
    let routing = routing::Routing {
        handle: kernel.handle.clone(),
        expected: kernel.expected.clone(),
        limit: 16384,
        start: kernel.journal.start,
        end: kernel.journal.end(),
    };
    backend::Backend::create(
        store,
        ledger,
        routing,
        Arc::new(RwLock::new(BTreeMap::new())),
        vec![Ipv4Addr::LOCALHOST],
        CancellationToken::new(),
    )
    .await
    .unwrap()
}

/// An exit TUN the monitor expects, as the owner creates it.
fn tun(kernel: &Kernel) -> meshway_tun::Tun {
    let mut expected = kernel.expected.lock().unwrap();
    let tun = meshway_tun::Tun::create(meshway_tun::Options::new(1280).unwrap()).unwrap();
    expected.exit_tun = Some(tun.if_index().unwrap());
    expected.exit_tables.push(20000);
    tun
}

async fn removed(kernel: &Kernel) -> bool {
    let (_, ledger) = storage::Store::open(kernel.store.directory())
        .await
        .unwrap();
    ledger.is_some_and(|ledger| ledger.removed)
}

#[tokio::test]
#[ignore = "requires an isolated Linux NFT/TUN namespace and NET_ADMIN"]
async fn exit_audit_read_failures_are_unknown_and_a_failed_cleanup_completes_on_retry() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let temp = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let directory = Arc::new(Directory::open(temp.path(), "node.lock").unwrap());
    let tasks = Tasks::new(64);
    let mut kernel = Kernel::open(directory, Default::default(), tasks.clone())
        .await
        .unwrap();
    kernel
        .retain_negative(&["100.96.0.0/11".parse().unwrap()])
        .await
        .unwrap();
    let first_tun = tun(&kernel);
    kernel.quiesce().await.unwrap();
    let mut backend = create(&mut kernel, &first_tun).await;

    // Reads that fail are unknown, never a change: the role is kept, and
    // the first consistent read proves it again.
    let start = std::time::Instant::now();
    let mut unverified = Unverified::new(Owner::Exit);
    objects::FAILING_READS.store(5, SeqCst);
    for second in 0..5 {
        let verification = backend.check().await;
        assert!(
            matches!(verification, Verification::Unavailable(Error::Netlink)),
            "{verification}"
        );
        let at = start + Duration::from_secs(second);
        assert_eq!(unverified.record(&verification, at), Ok(()));
        assert!(unverified.unknown());
    }
    let verification = backend.check().await;
    assert!(
        matches!(verification, Verification::Holds),
        "{verification}"
    );
    assert_eq!(
        unverified.record(&verification, start + Duration::from_secs(5)),
        Ok(())
    );
    assert!(!unverified.unknown());
    // A read that succeeds and differs is a change at once.
    let foreign = OriginalTuple {
        source: "100.96.0.9".parse().unwrap(),
        destination: "192.0.2.9".parse().unwrap(),
        transport: ExitTransport::Udp {
            source_port: 9,
            destination_port: 9,
        },
    };
    let element = backend
        .ledger
        .program
        .element("add", foreign.set(), vec![foreign.key()]);
    objects::mutate(
        &mut backend.commands,
        &serde_json::json!({"nftables": [element]}),
    )
    .await
    .unwrap();
    let verification = backend.check().await;
    assert!(
        matches!(verification, Verification::Changed("flow_unowned")),
        "{verification}"
    );
    let element = backend
        .ledger
        .program
        .element("delete", foreign.set(), vec![foreign.key()]);
    objects::mutate(
        &mut backend.commands,
        &serde_json::json!({"nftables": [element]}),
    )
    .await
    .unwrap();
    assert!(matches!(backend.check().await, Verification::Holds));
    println!("X11_EXIT_AUDIT=unavailable-x5-kept,holds,foreign-element-changed");

    // A cleanup whose first read fails keeps every resource; the retry
    // reads the kernel again and completes.
    objects::FAILING_READS.store(1, SeqCst);
    assert!(matches!(backend.close(true).await, Err(Error::Netlink)));
    assert_eq!(
        objects::present(&mut backend.commands, &backend.ledger.program)
            .await
            .unwrap(),
        2,
        "the failed attempt removed nothing"
    );
    assert!(!removed(&kernel).await);
    backend.close_observed(true).await.unwrap();
    assert_eq!(
        objects::present(&mut backend.commands, &backend.ledger.program)
            .await
            .unwrap(),
        0
    );
    assert!(removed(&kernel).await);
    drop(backend);
    drop(first_tun);
    kernel.quiesce().await.unwrap();
    println!("X11_EXIT_CLEANUP_RETRY=first-attempt-failed-kept,retry-removed");

    // The role installs again; a cleanup that removed the tables but did
    // not confirm it is completed by the next attempt's own read.
    let second_tun = tun(&kernel);
    kernel.quiesce().await.unwrap();
    let mut again = create(&mut kernel, &second_tun).await;
    assert!(matches!(again.check().await, Verification::Holds));
    again.begin_close(true).await.unwrap();
    let tables: Vec<_> = again
        .ledger
        .program
        .tables()
        .into_iter()
        .map(|(family, name)| serde_json::json!({"delete": {"table": {"family": family, "name": name}}}))
        .collect();
    objects::apply(
        &mut again.commands,
        &serde_json::json!({"nftables": tables}),
    )
    .await
    .unwrap();
    assert!(!removed(&kernel).await);
    again.close_observed(true).await.unwrap();
    assert!(removed(&kernel).await);
    drop(again);
    drop(second_tun);
    kernel.quiesce().await.unwrap();
    println!("X11_EXIT_REINSTALL=installed-again,tables-gone-completed");
    kernel.down().await.unwrap();
    drop(kernel);
    tasks.shutdown().await.unwrap();
}
