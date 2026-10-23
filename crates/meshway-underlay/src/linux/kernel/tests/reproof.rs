//! X13: ownership ended on unavailable reads is proved again.
use super::*;
use crate::linux::Host;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Node 1 (`100.64.0.1`) routed through exit Node 2: its content owns a TUN,
/// capture routes and an egress capture owner, so the capture owner's
/// periodic verification runs.
fn source_desired() -> Desired {
    use meshway_configuration::{ProfileAssignment, RouteAction, RoutingProfile};
    use meshway_test_server::configuration::{compile_one, network, node, sel, tag_node, trusted};
    let (source, exit) = (node(1), node(2));
    let mut cfg = network("x13.internal");
    for (id, index, exit_enabled) in [(source, 1, false), (exit, 2, true)] {
        cfg.nodes.insert(
            id,
            tag_node(
                &format!("n{index}"),
                index,
                Default::default(),
                exit_enabled,
            ),
        );
    }
    cfg.routing.egress.profiles.insert(
        "exit".into(),
        RoutingProfile {
            rules: vec![],
            fallback: RouteAction::Exit {
                nodes: vec![sel(exit)],
                via: vec![],
            },
        },
    );
    cfg.routing.egress.assignments.push(ProfileAssignment {
        target: vec![sel(source)],
        allowed: vec!["exit".into()],
        default: "exit".into(),
        locked: false,
    });
    let inventory =
        std::collections::BTreeMap::from([(source, trusted(1, false)), (exit, trusted(2, true))]);
    let network_id = "00000000-0000-4001-8000-000000000013".parse().unwrap();
    let netmap = compile_one(network_id, &cfg, &inventory, source, None);
    Desired::new(&Arc::new(netmap), &Default::default()).unwrap()
}

/// Every kernel tool read fails while this lasts (a host too stalled to
/// answer within the tools' budget).
struct Unreadable;
impl Unreadable {
    fn start() -> Self {
        use crate::linux::kernel::tool::command::faults::FAILING_TOOL_READS;
        FAILING_TOOL_READS.store(u32::MAX, std::sync::atomic::Ordering::SeqCst);
        Self
    }
}
impl Drop for Unreadable {
    fn drop(&mut self) {
        use crate::linux::kernel::tool::command::faults::FAILING_TOOL_READS;
        FAILING_TOOL_READS.store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

/// X13: kernel objects unreadable for the whole 30 s limit end ownership as
/// unavailable, a transient block: the Host proves its ownership again on
/// its backoff, and once the kernel reads back as its journal records it
/// opens a new term on the same journal, the same published channels and
/// the same Control relay origin, and the same content installs again
/// without a restart. A re-proof that reads back a foreign object makes the
/// end a conflict that stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "isolated Linux NET_ADMIN namespace with TUN and nft required"]
async fn an_unavailable_ownership_end_is_proved_again_and_a_foreign_object_keeps_it_ended() {
    use crate::OwnershipEnd;
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(Directory::open(&temp.path().join("private"), "test.lock").unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let _ = stream.read(&mut request).await;
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await;
        }
    });
    let mut host = Host::open(directory.clone(), Options::default())
        .await
        .unwrap();
    host.attach_control(&origin).await.unwrap();
    let client = host
        .client_builder()
        .unwrap()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let relayed = async |client: &reqwest::Client| match client.get(&origin).send().await {
        Ok(response) => response.text().await.is_ok_and(|text| text == "ok"),
        Err(_) => false,
    };
    assert!(relayed(&client).await);
    let desired = source_desired();
    host.install(desired.clone()).await.unwrap();
    let first_tun = host.tun().unwrap().name().to_owned();
    let installation = host.kernel.journal.installation.clone();
    let network_changes = host.network_changes();
    let kernel_changes = host.kernel_changes();
    let exit_cleanups = host.exit_cleanups();

    let unreadable = Unreadable::start();
    let started = std::time::Instant::now();
    tokio::time::timeout(std::time::Duration::from_secs(45), host.ended())
        .await
        .expect("30 s of unreadable verification ends ownership");
    assert_eq!(
        host.ownership_ended(),
        Some(OwnershipEnd::Unavailable {
            check: "verify_unavailable"
        })
    );
    println!(
        "X13_ENDED_UNAVAILABLE_AFTER_MS={}",
        started.elapsed().as_millis()
    );
    // Failed closed meanwhile: nothing installs, no TUN, no relay.
    assert!(host.tun().is_none());
    assert!(host.install(desired.clone()).await.is_err());
    assert!(!relayed(&client).await);
    let first_due = host
        .recovery_at()
        .expect("an unavailable end is proved again");
    // Still unreadable: the re-proof fails and stays unavailable, later.
    let error = host.recover().await.unwrap_err();
    assert!(matches!(error, Error::Netlink | Error::Io(_)), "{error:?}");
    assert!(matches!(
        host.ownership_ended(),
        Some(OwnershipEnd::Unavailable { .. })
    ));
    let second_due = host.recovery_at().unwrap();
    assert!(second_due > first_due);
    assert!(second_due <= std::time::Instant::now() + std::time::Duration::from_secs(2));

    // Readable again and as the journals record it: a new term.
    drop(unreadable);
    host.recover().await.unwrap();
    assert!(host.ownership_ended().is_none() && host.recovery_at().is_none());
    assert_eq!(
        host.kernel.journal.installation, installation,
        "same journal"
    );
    host.install(desired.clone()).await.unwrap();
    assert!(host.tun().is_some());
    host.kernel.shared.check().unwrap();
    assert!(
        relayed(&client).await,
        "the same Control client relays again"
    );
    for closed in [
        network_changes.has_changed().is_err(),
        kernel_changes.has_changed().is_err(),
        exit_cleanups.has_changed().is_err(),
    ] {
        assert!(!closed, "published channels continue across terms");
    }
    println!(
        "X13_REPROVED=journal-kept,content-reinstalled,relay-kept,tun={first_tun}->{}",
        host.tun().unwrap().name()
    );

    // Unavailable again; meanwhile another writer adds a rule inside the
    // capture block. The re-proof reads it back: a conflict that stays
    // while the rule is there, read again every 60 s (RC-1 Q2).
    let unreadable = Unreadable::start();
    tokio::time::timeout(std::time::Duration::from_secs(45), host.ended())
        .await
        .expect("ends again");
    let priority = (host.kernel.journal.end() - 1).to_string();
    let ip = |arguments: &[&str]| {
        let status = std::process::Command::new("/usr/sbin/ip")
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success(), "ip {arguments:?}");
    };
    // A first, still unreadable re-proof ends the old term's monitor too:
    // only the next re-proof can see the rule.
    assert!(host.recover().await.is_err());
    ip(&["rule", "add", "pref", &priority, "lookup", "main"]);
    drop(unreadable);
    assert!(matches!(host.recover().await, Err(Error::Conflict)));
    assert_eq!(
        host.ownership_ended(),
        Some(OwnershipEnd::Conflict { check: "reproof" })
    );
    let reread = host.recovery_at().expect("a proven conflict is read again");
    let wait = reread.saturating_duration_since(std::time::Instant::now());
    assert!(
        wait > std::time::Duration::from_secs(50) && wait <= std::time::Duration::from_secs(60),
        "read again every 60 s, not {wait:?}"
    );
    // Read again with the rule still there: still a conflict, the foreign
    // rule untouched.
    assert!(matches!(host.recover().await, Err(Error::Conflict)));
    assert!(host.ownership().borrow().is_blocked());
    assert!(host.install(desired.clone()).await.is_err());
    let rules = std::process::Command::new("/usr/sbin/ip")
        .args(["rule", "show", "pref", &priority])
        .output()
        .unwrap();
    assert!(
        !rules.stdout.is_empty(),
        "a re-read never removes a foreign object"
    );
    println!("X13_FOREIGN_KEPT=conflict:reproof");
    // The evidence gone, the next read-only re-proof owns a new term again.
    ip(&["rule", "del", "pref", &priority]);
    host.recover().await.unwrap();
    assert!(host.ownership_ended().is_none() && host.ownership().borrow().is_healthy());
    host.install(desired).await.unwrap();
    host.kernel.shared.check().unwrap();
    println!("RC1_CONFLICT_REREAD=recovered-after-removal");
    drop(host);
    server.abort();
    // A restart (a new Host) owns the same journal again.
    let mut host = Host::open(directory.clone(), Options::default())
        .await
        .unwrap();
    assert_eq!(host.kernel.journal.installation, installation);
    host.install(source_desired()).await.unwrap();

    // A foreign rule the ended term's monitor still sees is proof enough:
    // the unavailable end becomes a conflict before any re-proof.
    let unreadable = Unreadable::start();
    tokio::time::timeout(std::time::Duration::from_secs(45), host.ended())
        .await
        .expect("ends again");
    ip(&["rule", "add", "pref", &priority, "lookup", "main"]);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while matches!(
            host.ownership_ended(),
            Some(OwnershipEnd::Unavailable { .. })
        ) {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the monitor reads the rule");
    assert_eq!(
        host.ownership_ended(),
        Some(OwnershipEnd::Conflict { check: "monitor" })
    );
    assert!(host.ownership().borrow().is_blocked());
    assert!(host.recovery_at().is_some(), "read again every 60 s");
    drop(unreadable);
    ip(&["rule", "del", "pref", &priority]);
    println!("X13_FOREIGN_SEEN=conflict:monitor");
    drop(host);
    let mut host = Host::open(directory, Options::default()).await.unwrap();
    host.down().await.unwrap();
}

/// U2: a suspend whose route deletion fails on a stalled host (the lab's
/// DNS withdrawal right after a re-proof) leaves its journal step pending
/// and the change's outcome unread. That ends the term as unavailable;
/// the Host's re-proof completes the step from the kernel's state in its
/// next term, and the same content installs again, without a restart.
/// A process start finding the step on disk completes it the same way.
/// Before: the term held, every later install refused the journal
/// ("invalid or requires recovery") forever, and a restart refused it too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "isolated Linux NET_ADMIN namespace with TUN and nft required"]
async fn a_suspend_interrupted_with_its_step_pending_is_reproved_and_installs_again() {
    use crate::OwnershipEnd;
    use crate::linux::kernel::tool::command::faults::FAILING_ROUTE_DELETES;
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(Directory::open(&temp.path().join("private"), "test.lock").unwrap());
    let mut host = Host::open(directory.clone(), Options::default())
        .await
        .unwrap();
    let desired = source_desired();
    host.install(desired.clone()).await.unwrap();
    let installation = host.kernel.journal.installation.clone();

    FAILING_ROUTE_DELETES.store(1, std::sync::atomic::Ordering::SeqCst);
    let error = host.suspend().await.unwrap_err();
    println!("U2_SUSPEND_FAILED={error}");
    assert!(matches!(
        host.kernel.journal.pending,
        Some(Pending::DeleteRoute { .. })
    ));
    assert_eq!(
        host.ownership_ended(),
        Some(OwnershipEnd::Unavailable {
            check: "suspend_unavailable"
        }),
        "a step left pending ends the term; install after it: {:?}",
        host.install(desired.clone()).await.err()
    );
    assert!(!host.ownership().borrow().is_healthy() && !host.ownership().borrow().is_blocked());
    let due = host
        .recovery_at()
        .expect("an unavailable end is proved again");
    assert!(due <= std::time::Instant::now() + std::time::Duration::from_secs(2));
    let started = std::time::Instant::now();
    host.recover().await.unwrap();
    assert!(host.kernel.journal.pending.is_none());
    assert_eq!(host.kernel.journal.installation, installation);
    assert!(host.kernel.journal.routes.is_empty() && host.kernel.journal.tun.is_none());
    host.install(desired.clone()).await.unwrap();
    assert!(host.tun().is_some());
    host.kernel.shared.check().unwrap();
    println!(
        "U2_REPROVED=step-completed,content-reinstalled,elapsed_ms={}",
        started.elapsed().as_millis()
    );

    // The same interruption, then a process restart.
    FAILING_ROUTE_DELETES.store(1, std::sync::atomic::Ordering::SeqCst);
    assert!(host.suspend().await.is_err());
    host.kernel.settle().await.unwrap();
    drop(host);
    // The dropped Host's stopped tasks release the directory lock shortly.
    let opening = std::time::Instant::now();
    let mut host = loop {
        match Host::open(directory.clone(), Options::default()).await {
            Err(Error::Storage(meshway_private_storage::Error::InUse))
                if opening.elapsed() < std::time::Duration::from_secs(5) =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            opened => break opened.unwrap(),
        }
    };
    assert!(host.kernel.journal.pending.is_none());
    assert_eq!(host.kernel.journal.installation, installation);
    host.install(desired).await.unwrap();
    assert!(host.tun().is_some());
    println!("U2_RESTART=step-completed,content-reinstalled");
    host.down().await.unwrap();
}

/// U2b: a suspend whose opening read-back fails (a stalled host) before any
/// journal step still closes the Mesh TUN, and the kernel removes its routes
/// with it. The term ends as unavailable; the re-proof's new term reconciles
/// the journal's TUN and route records from the kernel and the same content
/// installs again. Before: the term held with a journal recording a route
/// the kernel no longer had, and every later install read that back as a
/// routing protection conflict ("resources kept"), retried forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "isolated Linux NET_ADMIN namespace with TUN and nft required"]
async fn a_suspend_whose_read_fails_after_closing_the_tun_is_reproved_and_installs_again() {
    use crate::OwnershipEnd;
    use crate::linux::kernel::tool::command::faults::FAILING_SUSPEND_READS;
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(Directory::open(&temp.path().join("private"), "test.lock").unwrap());
    let mut host = Host::open(directory.clone(), Options::default())
        .await
        .unwrap();
    let desired = source_desired();
    host.install(desired.clone()).await.unwrap();
    let installation = host.kernel.journal.installation.clone();
    assert!(!host.kernel.journal.routes.is_empty());

    FAILING_SUSPEND_READS.store(1, std::sync::atomic::Ordering::SeqCst);
    let error = host.suspend().await.unwrap_err();
    println!("U2B_SUSPEND_FAILED={error}");
    assert!(host.kernel.journal.pending.is_none());
    assert!(host.tun().is_none());
    assert_eq!(
        host.ownership_ended(),
        Some(OwnershipEnd::Unavailable {
            check: "suspend_unavailable"
        }),
        "a closed TUN the journal still records ends the term; install after it: {:?}",
        host.install(desired.clone()).await.err()
    );
    let due = host
        .recovery_at()
        .expect("an unavailable end is proved again");
    assert!(due <= std::time::Instant::now() + std::time::Duration::from_secs(2));
    let started = std::time::Instant::now();
    host.recover().await.unwrap();
    assert_eq!(host.kernel.journal.installation, installation);
    assert!(host.kernel.journal.routes.is_empty() && host.kernel.journal.tun.is_none());
    host.install(desired).await.unwrap();
    assert!(host.tun().is_some());
    host.kernel.shared.check().unwrap();
    println!(
        "U2B_REPROVED=journal-reconciled,content-reinstalled,elapsed_ms={}",
        started.elapsed().as_millis()
    );
    host.down().await.unwrap();
}

/// X15: a monitor drain that misses its 3 s deadline is an unavailable end
/// with its own cause (re-proved like any other), not a conflict; a
/// read-back that differs still is one. The stopped request connection's
/// end: `protection::tests`.
#[test]
fn a_missed_drain_ends_ownership_as_unavailable() {
    use crate::OwnershipEnd;
    for operation in ["install", "repair"] {
        assert_eq!(
            super::super::ended(operation, &Error::Drain),
            OwnershipEnd::Unavailable {
                check: "drain_unavailable"
            }
        );
    }
    assert_eq!(
        super::super::ended("install", &Error::Conflict),
        OwnershipEnd::Conflict { check: "install" }
    );
}
