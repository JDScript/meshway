//! X12: a failed exit kernel change proves neither outcome. Each change path
//! is injected failing after the kernel applied it (`lost`: the tool's answer
//! and the next read fail) and failing before the kernel saw it (`failing`);
//! the owner reads the role back and completes the change, keeping the role
//! and its other flows.
use super::super::super::tool::command::faults::{FAILING_CHANGES, LOST_CHANGES};
use super::super::super::verification::{Owner, Reread, Verification, retried};
use super::cleanup::{create, tun};
use super::*;
use std::sync::atomic::Ordering::SeqCst;

fn arm(lost: bool) {
    if lost {
        LOST_CHANGES.store(1, SeqCst);
    } else {
        FAILING_CHANGES.store(1, SeqCst);
    }
}

fn label(lost: bool) -> &'static str {
    if lost { "lost" } else { "failing" }
}

/// A retirement as the owner retries it.
async fn retire(
    backend: &mut backend::Backend,
    incarnation: ExitIncarnation,
    operation: &'static str,
) -> std::result::Result<(), (crate::Error, &'static str)> {
    let (changes, _) = tokio::sync::watch::channel(None);
    let stop = CancellationToken::new();
    retried(
        backend,
        Owner::Exit,
        operation,
        &changes,
        &[&stop],
        move |backend: &mut backend::Backend| Box::pin(backend.failed(incarnation)),
        move |backend: &mut backend::Backend| Box::pin(backend.reread(incarnation)),
    )
    .await
}

#[tokio::test]
#[ignore = "requires an isolated Linux NFT/TUN namespace, a UDP echo service and NET_ADMIN"]
async fn failed_exit_changes_are_read_back_and_completed() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let remote: SocketAddrV4 = std::env::var("MESHWAY_X4_EXTERNAL")
        .unwrap()
        .parse()
        .unwrap();
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
    let exit_tun = tun(&kernel);
    kernel.quiesce().await.unwrap();
    let mut backend = create(&mut kernel, &exit_tun).await;
    let mark = backend.ledger.program.mark;
    let scope = ExitScope {
        binding: Digest::from_bytes([7; 32]),
        revoked: CancellationToken::new(),
    };
    let mut port = 17100;
    let mut reply = vec![0; 1280];
    // One confirmed projection, as the driver makes it.
    let mut confirmed = async |backend: &mut backend::Backend, id: u8, milliseconds| {
        port += 1;
        let tuple = OriginalTuple {
            source: "100.96.0.1".parse().unwrap(),
            destination: (*remote.ip()).into(),
            transport: ExitTransport::Udp {
                source_port: port,
                destination_port: remote.port(),
            },
        };
        backend
            .install(super::id(id), tuple, cutoff(milliseconds), scope.clone())
            .await
            .unwrap();
        exit_tun.send(&udp(tuple, b"x12")).await.unwrap();
        assert!(matches!(
            backend.confirm(super::id(id)).await.unwrap(),
            CtConfirmation::Confirmed { .. }
        ));
        tokio::time::timeout(Duration::from_secs(2), recv_ipv4(&exit_tun, &mut reply))
            .await
            .unwrap()
            .unwrap();
        tuple
    };
    let mut outcomes = Vec::new();
    for (index, lost) in [true, false].into_iter().enumerate() {
        let base = 10 * index as u8;
        // renew: the batch reads back present and runs once more.
        let tuple = confirmed(&mut backend, base + 1, 30_000).await;
        arm(lost);
        let renewed = backend
            .renew(super::id(base + 1), cutoff(30_000), scope.clone())
            .await;
        assert!(renewed.is_ok(), "renew {}: {renewed:?}", label(lost));
        assert!(matches!(backend.check().await, Verification::Holds));
        outcomes.push(format!("renew-{}-renewed", label(lost)));
        // retire: the failed attempt is read back and completed.
        arm(lost);
        assert!(backend.retire(super::id(base + 1)).await.is_err());
        let observed = backend.reread(super::id(base + 1)).await;
        assert!(matches!(observed, Reread::Start), "{observed}");
        // One the kernel never saw fails once more in the owner's retry.
        if !lost {
            FAILING_CHANGES.store(1, SeqCst);
        }
        retire(&mut backend, super::id(base + 1), "retire")
            .await
            .unwrap();
        assert!(
            conntrack::lookup_owned(tuple, mark)
                .await
                .unwrap()
                .is_none()
        );
        assert!(backend.ledger.projections.is_empty());
        assert!(matches!(backend.check().await, Verification::Holds));
        outcomes.push(format!("retire-{}-{observed}-completed", label(lost)));
        // maintain: an expired projection's retirement (its reply tuple
        // quarantined, a mutation) fails and is completed from the read-back.
        let tuple = confirmed(&mut backend, base + 2, 5_000).await;
        tokio::time::sleep(Duration::from_millis(5_100)).await;
        arm(lost);
        let (changes, _) = tokio::sync::watch::channel(None);
        let stop = CancellationToken::new();
        let maintained = retried(
            &mut backend,
            Owner::Exit,
            "maintain",
            &changes,
            &[&stop],
            |backend: &mut backend::Backend| Box::pin(backend.maintain()),
            |backend: &mut backend::Backend| Box::pin(backend.reread_maintain()),
        )
        .await;
        assert!(
            maintained.is_ok(),
            "maintain {}: {maintained:?}",
            label(lost)
        );
        assert_eq!(
            LOST_CHANGES.load(SeqCst) + FAILING_CHANGES.load(SeqCst),
            0,
            "the injected failure was reached"
        );
        assert!(
            conntrack::lookup_owned(tuple, mark)
                .await
                .unwrap()
                .is_none()
        );
        assert!(backend.ledger.projections.is_empty());
        assert!(matches!(backend.check().await, Verification::Holds));
        outcomes.push(format!("maintain-{}-completed", label(lost)));
    }
    // A read-back that differs (an element no projection owns) is not
    // retried: the owner stops as before.
    let tuple = confirmed(&mut backend, 30, 30_000).await;
    FAILING_CHANGES.store(1, SeqCst);
    assert!(backend.retire(id(30)).await.is_err());
    let foreign = OriginalTuple {
        transport: ExitTransport::Udp {
            source_port: 9,
            destination_port: 9,
        },
        ..tuple
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
    FAILING_CHANGES.store(1, SeqCst);
    let result = retire(&mut backend, id(30), "retire").await;
    assert!(matches!(result, Err((_, "verify"))), "{result:?}");
    println!(
        "X12_EXIT_CHANGES={},foreign-element-verify",
        outcomes.join(",")
    );
    drop(backend);
    drop(exit_tun);
    drop(kernel);
    tasks.shutdown().await.unwrap();
}

/// F6a B6: an aged quarantine deletion the kernel applied whose answer was
/// lost is completed: the retry reads the set, finds the element gone and
/// only writes the journal, instead of deleting it again until the role
/// stops. One the kernel never saw is deleted on the retry.
#[tokio::test]
#[ignore = "requires an isolated Linux NFT/TUN namespace and NET_ADMIN"]
async fn a_lost_quarantine_expiry_is_completed_from_the_read_back() {
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
    let exit_tun = tun(&kernel);
    kernel.quiesce().await.unwrap();
    let mut backend = create(&mut kernel, &exit_tun).await;
    let filter = backend.ledger.program.filter();
    for (port, lost) in [(4001, true), (4002, false)] {
        let retired = conntrack::RetiredTuple {
            tuple: conntrack::Tuple {
                source: "192.0.2.9".parse().unwrap(),
                destination: "100.96.0.1".parse().unwrap(),
                protocol: 17,
                source_key: 53,
                destination_key: port,
            },
            sequence: 0,
            retired_ms: 1,
        };
        let element = backend
            .ledger
            .program
            .element("add", retired.set(), vec![retired.key()]);
        objects::mutate(
            &mut backend.commands,
            &serde_json::json!({"nftables": [element]}),
        )
        .await
        .unwrap();
        backend.ledger.quarantine.push(retired.clone());
        arm(lost);
        let (changes, _) = tokio::sync::watch::channel(None);
        let stop = CancellationToken::new();
        let maintained = retried(
            &mut backend,
            Owner::Exit,
            "maintain",
            &changes,
            &[&stop],
            |backend: &mut backend::Backend| Box::pin(backend.maintain()),
            |backend: &mut backend::Backend| Box::pin(backend.reread_maintain()),
        )
        .await;
        assert!(maintained.is_ok(), "{}: {maintained:?}", label(lost));
        assert_eq!(LOST_CHANGES.load(SeqCst) + FAILING_CHANGES.load(SeqCst), 0);
        assert!(backend.ledger.quarantine.is_empty(), "{}", label(lost));
        let dump = objects::dump(&mut backend.commands, &backend.ledger.program)
            .await
            .unwrap();
        assert!(
            dump.elements
                .get(&format!("inet/{filter}/set/quarantine"))
                .is_none_or(Vec::is_empty),
            "{}",
            label(lost)
        );
        assert!(matches!(backend.check().await, Verification::Holds));
    }
    drop(backend);
    drop(exit_tun);
    drop(kernel);
    tasks.shutdown().await.unwrap();
}
