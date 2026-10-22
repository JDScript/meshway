//! X16: a dump the kernel did not finish (`EBUSY`, `NLM_F_DUMP_INTR`) is no
//! failed install. The install's route step reads the rules again and the
//! flow's first packet is answered; only a dump interrupted at every attempt
//! fails the install, which retires as any failed install and installs again.
use super::super::super::rtnl::DUMP_ATTEMPTS;
use super::super::super::tool::command::faults::INTERRUPTED_DUMPS;
use super::super::super::verification::{Owner, Verification, retried};
use super::cleanup::{create, tun};
use super::*;
use std::sync::atomic::Ordering::SeqCst;

fn tuple(source: &str, port: u16, remote: SocketAddrV4) -> OriginalTuple {
    OriginalTuple {
        source: source.parse().unwrap(),
        destination: (*remote.ip()).into(),
        transport: ExitTransport::Udp {
            source_port: port,
            destination_port: remote.port(),
        },
    }
}

/// The flow's first packet after its installation: out through the exit
/// TUN, confirmed, and its reply back.
async fn answered(
    backend: &mut backend::Backend,
    exit_tun: &meshway_tun::Tun,
    id: u8,
    tuple: OriginalTuple,
) {
    exit_tun.send(&udp(tuple, b"x16")).await.unwrap();
    assert!(matches!(
        backend.confirm(super::id(id)).await.unwrap(),
        CtConfirmation::Confirmed { .. }
    ));
    let mut reply = vec![0; 1280];
    tokio::time::timeout(Duration::from_secs(2), recv_ipv4(exit_tun, &mut reply))
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
#[ignore = "requires an isolated Linux NFT/TUN namespace, a UDP echo service and NET_ADMIN"]
async fn interrupted_dumps_are_read_again_and_the_first_packet_is_answered() {
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
    let scope = ExitScope {
        binding: Digest::from_bytes([7; 32]),
        revoked: CancellationToken::new(),
    };
    let mut outcomes = Vec::new();
    // A new pair: its route step dumps routes and rules; the first dump is
    // refused once (another dump on the socket) and read again.
    let first = tuple("100.96.0.1", 17201, remote);
    INTERRUPTED_DUMPS.store(1, SeqCst);
    backend
        .install(id(1), first, cutoff(30_000), scope.clone())
        .await
        .unwrap();
    assert_eq!(
        INTERRUPTED_DUMPS.load(SeqCst),
        0,
        "the interruption was reached"
    );
    answered(&mut backend, &exit_tun, 1, first).await;
    outcomes.push("interrupted-once-installed-answered");
    // Interrupted at every attempt: the read is unavailable, the install
    // fails at its route step, and the owner retires it like any failed one.
    let second = tuple("100.96.0.2", 17202, remote);
    INTERRUPTED_DUMPS.store(DUMP_ATTEMPTS, SeqCst);
    let failed = backend
        .install(id(2), second, cutoff(30_000), scope.clone())
        .await;
    assert!(matches!(failed, Err(crate::Error::Netlink)), "{failed:?}");
    assert_eq!(INTERRUPTED_DUMPS.load(SeqCst), 0);
    assert_eq!(backend.take_failed_step(id(2)), "route");
    let (changes, _) = tokio::sync::watch::channel(None);
    let stop = CancellationToken::new();
    retried(
        &mut backend,
        Owner::Exit,
        "install",
        &changes,
        &[&stop],
        |backend: &mut backend::Backend| Box::pin(backend.failed(id(2))),
        |backend: &mut backend::Backend| Box::pin(backend.reread(id(2))),
    )
    .await
    .unwrap();
    assert!(
        backend
            .ledger
            .projections
            .iter()
            .all(|projection| projection.incarnation == id(1))
    );
    assert!(matches!(backend.check().await, Verification::Holds));
    outcomes.push("interrupted-always-route-failed-retired");
    // The same flow's next attempt installs and is answered; the first flow
    // was kept throughout.
    backend
        .install(id(3), second, cutoff(30_000), scope.clone())
        .await
        .unwrap();
    answered(&mut backend, &exit_tun, 3, second).await;
    exit_tun.send(&udp(first, b"x16-kept")).await.unwrap();
    let mut reply = vec![0; 1280];
    tokio::time::timeout(Duration::from_secs(2), recv_ipv4(&exit_tun, &mut reply))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(backend.check().await, Verification::Holds));
    outcomes.push("reinstalled-answered-first-kept");
    println!("X16_EXIT_DUMPS={}", outcomes.join(","));
    drop(backend);
    drop(exit_tun);
    drop(kernel);
    tasks.shutdown().await.unwrap();
}
