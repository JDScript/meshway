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
    // capture block. The re-proof reads it back: a conflict that stays.
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
    assert!(host.recovery_at().is_none());
    // The foreign rule's removal does not undo a proven conflict.
    ip(&["rule", "del", "pref", &priority]);
    assert!(matches!(host.recover().await, Err(Error::Conflict)));
    assert!(host.install(desired).await.is_err());
    println!("X13_FOREIGN_KEPT=conflict:reproof");
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
    assert!(host.recovery_at().is_none());
    drop(unreadable);
    ip(&["rule", "del", "pref", &priority]);
    println!("X13_FOREIGN_SEEN=conflict:monitor");
    drop(host);
    let mut host = Host::open(directory, Options::default()).await.unwrap();
    host.down().await.unwrap();
}
