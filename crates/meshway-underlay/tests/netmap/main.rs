//! Host resources under netmap snapshots: a same-content renewal touches
//! nothing, a larger serial moves only the resources whose exact purpose
//! changed, and the end of the lease closes descriptors and the exit role.
#![cfg(target_os = "linux")]

mod support;

use meshway_underlay::{ExitIncarnation, ExitScope, ExitTransport, Host, OriginalTuple};
use std::{net::UdpSocket, sync::Arc, time::Duration};
use support::{HOUR, Network, Server, listener, sync};
use tokio_util::sync::CancellationToken;

fn isolated() -> tempfile::TempDir {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let temp = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    temp
}

async fn host(temp: &tempfile::TempDir) -> Host {
    let directory = Arc::new(
        meshway_private_storage::Directory::open(&temp.path().join("underlay"), "test.lock")
            .unwrap(),
    );
    Host::open(directory, Default::default()).await.unwrap()
}

/// The protected descriptor still owns its port.
fn held(port: u16) -> bool {
    UdpSocket::bind(("0.0.0.0", port)).is_err()
}

async fn released(port: u16) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while held(port) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("closed descriptor releases its port");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an isolated Linux namespace, /dev/net/tun and NET_ADMIN"]
async fn netmap_renewal_keeps_descriptors_and_a_new_serial_moves_only_changed_ones() {
    let temp = isolated();
    let server = Server::start().await;
    let (mut control, script) = server.enroll(temp.path()).await;
    let mut network = Network::new(&control, false);
    let first = network.compile(None);
    let snapshot = sync(&mut control, &script, &first, true, HOUR / 2).await;
    let mut host = host(&temp).await;
    host.install(snapshot.clone()).await.unwrap();
    let interface = host.tun().unwrap().if_index().unwrap();
    let sockets = host.sockets();
    let udp = sockets.datagram_listener(&snapshot, "udp").await.unwrap();
    let keep = sockets.datagram_listener(&snapshot, "keep").await.unwrap();
    assert!(held(41001) && held(41002));

    // Same content: the Host changes nothing.
    let renewed = sync(&mut control, &script, &first, false, HOUR).await;
    assert_eq!(renewed.reference(), snapshot.reference());
    host.install(renewed.clone()).await.unwrap();
    assert_eq!(host.tun().unwrap().if_index().unwrap(), interface);
    udp.local_addr().unwrap();
    keep.local_addr().unwrap();
    sockets.listener(&snapshot, "keep").unwrap();

    // A larger serial changing only listener "udp".
    network.listeners.insert("udp".into(), listener(41003));
    let second = network.compile(Some(&first));
    let next = sync(&mut control, &script, &second, true, HOUR).await;
    // Nothing changes until the content is applied to the Host.
    keep.local_addr().unwrap();
    udp.local_addr().unwrap();
    assert!(held(41001) && held(41002));
    host.install(next.clone()).await.unwrap();
    assert_eq!(host.tun().unwrap().if_index().unwrap(), interface);
    keep.local_addr().unwrap();
    assert!(udp.local_addr().is_err());
    released(41001).await;
    assert!(held(41002));
    let moved = sockets.datagram_listener(&next, "udp").await.unwrap();
    assert!(held(41003));
    drop(moved);
    drop((udp, keep));
    let _ = control.shutdown().await;
    host.shutdown(false).await.unwrap();
    server.close().await;
    println!("N1G_NETMAP_RESOURCES=renewal-noop,serial-diff,kept-tun");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an isolated Linux namespace, /dev/net/tun and NET_ADMIN"]
async fn withdrawal_closes_descriptors_and_keeps_the_negative_boundary() {
    let temp = isolated();
    let server = Server::start().await;
    let (mut control, script) = server.enroll(temp.path()).await;
    let network = Network::new(&control, false);
    let netmap = network.compile(None);
    let snapshot = sync(&mut control, &script, &netmap, true, HOUR).await;
    let mut host = host(&temp).await;
    host.install(snapshot.clone()).await.unwrap();
    let keep = host
        .sockets()
        .datagram_listener(&snapshot, "keep")
        .await
        .unwrap();
    // Engine's empty apply (lease end or revocation).
    host.suspend().await.unwrap();
    assert!(host.tun().is_none());
    assert!(keep.local_addr().is_err());
    released(41002).await;
    drop(keep);
    let _ = control.shutdown().await;
    host.shutdown(false).await.unwrap();
    server.close().await;
    println!("N1G_NETMAP_WITHDRAWAL=descriptors-closed,tun-inactive");
}

/// A destination the isolated exit namespace routes (the fixture's external
/// service): each projection asks the kernel for its forwarding decision.
fn routable() -> std::net::Ipv4Addr {
    let external: std::net::SocketAddrV4 = std::env::var("MESHWAY_X4_EXTERNAL")
        .expect("MESHWAY_X4_EXTERNAL")
        .parse()
        .unwrap();
    *external.ip()
}

fn cutoff(milliseconds: u64) -> meshway_underlay::ExitCutoff {
    let now = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    meshway_underlay::ExitCutoff {
        at: std::time::Instant::now() + Duration::from_millis(milliseconds),
        ms: now + milliseconds,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an isolated Linux exit namespace (ip_forward=1, loose RPF), nft and NET_ADMIN"]
async fn netmap_exit_role_survives_renewal_and_unchanged_serial_and_ends_on_withdrawal() {
    let temp = isolated();
    let server = Server::start().await;
    let (mut control, script) = server.enroll(temp.path()).await;
    let mut network = Network::new(&control, true);
    let first = network.compile(None);
    let role_plan = first.netmap.exit_role.clone().expect("exit role");
    let snapshot = sync(&mut control, &script, &first, true, HOUR).await;
    let mut host = host(&temp).await;
    host.install(snapshot.clone()).await.unwrap();
    let role = host.install_netmap_exit_role(&role_plan).await.unwrap();
    let scope = ExitScope {
        binding: meshway_model::Digest::from_bytes([72; 32]),
        revoked: CancellationToken::new(),
    };
    // One new projection per probe: the exit's own decision for source 2 on
    // its installed netmap (no packet is sent).
    let mut next_port = 17_000u16;
    let mut project_to = |source: &str, destination: std::net::Ipv4Addr| {
        next_port += 1;
        let tuple = OriginalTuple {
            source: source.parse().unwrap(),
            destination,
            transport: ExitTransport::Udp {
                source_port: next_port,
                destination_port: 42424,
            },
        };
        let mut id = [0u8; 32];
        id[..2].copy_from_slice(&next_port.to_be_bytes());
        role.install(
            ExitIncarnation::new(id),
            tuple,
            cutoff(20_000),
            scope.clone(),
        )
    };
    // Tuple Access admission is SecureNode's flow table alone
    // (dataplane `netmap_exit`); the role owner installs its projections.
    project_to("100.64.0.2", routable()).await.unwrap();
    // A destination this host cannot forward to (the namespace has no
    // default route) is refused by the kernel's own route decision; the
    // role keeps serving.
    assert!(matches!(
        project_to("100.64.0.2", "8.8.8.8".parse().unwrap()).await,
        Err(meshway_underlay::Error::Target)
    ));
    assert!(role.is_active());
    let mut project = |source: &str| project_to(source, routable());

    // Same content: nothing changes and the instance-free role is unchanged.
    let renewed = sync(&mut control, &script, &first, false, HOUR).await;
    host.install(renewed).await.unwrap();
    assert!(role.is_active());
    project("100.64.0.2").await.unwrap();

    // A larger serial with the same role keeps it across the apply.
    network.listeners.insert("udp".into(), listener(41003));
    let second = network.compile(Some(&first));
    assert_eq!(second.netmap.exit_role, first.netmap.exit_role);
    let next = sync(&mut control, &script, &second, true, HOUR).await;
    host.install(next).await.unwrap();
    assert!(role.is_active() && host.exit_role().is_some());
    project("100.64.0.2").await.unwrap();

    // Revoking the source's Access changes no exit role: the role remains.
    network.access = false;
    let third = network.compile(Some(&second));
    assert_eq!(third.netmap.exit_role, first.netmap.exit_role);
    let revoked = sync(&mut control, &script, &third, true, HOUR).await;
    host.install(revoked).await.unwrap();
    assert!(role.is_active() && host.exit_role().is_some());

    // Engine's empty apply: the owner stops and cleans up exactly.
    host.suspend().await.unwrap();
    tokio::time::timeout(Duration::from_secs(6), async {
        while role.is_active() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("exit role ends on withdrawal");
    role.retire().await.unwrap();
    assert!(host.exit_role().is_none());
    let _ = control.shutdown().await;
    host.shutdown(false).await.unwrap();
    server.close().await;
    println!("N1G_NETMAP_EXIT=renewal-noop,serial-kept,access-independent,withdrawal-cleanup");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an isolated Linux exit namespace (ip_forward=1, loose RPF), nft and NET_ADMIN"]
async fn netmap_exit_projection_capacity_refuses_one_flow_and_keeps_the_role() {
    let temp = isolated();
    let server = Server::start().await;
    let (mut control, script) = server.enroll(temp.path()).await;
    let network = Network::new(&control, true);
    let netmap = network.compile(None);
    let role_plan = netmap.netmap.exit_role.clone().expect("exit role");
    let snapshot = sync(&mut control, &script, &netmap, true, HOUR).await;
    let mut host = host(&temp).await;
    host.install(snapshot).await.unwrap();
    let role = host.install_netmap_exit_role(&role_plan).await.unwrap();
    let scope = ExitScope {
        binding: meshway_model::Digest::from_bytes([73; 32]),
        revoked: CancellationToken::new(),
    };
    let incarnation = |port: u16| {
        let mut id = [0u8; 32];
        id[..2].copy_from_slice(&port.to_be_bytes());
        ExitIncarnation::new(id)
    };
    let tuple = |port: u16| OriginalTuple {
        source: "100.64.0.2".parse().unwrap(),
        destination: routable(),
        transport: ExitTransport::Udp {
            source_port: port,
            destination_port: 42424,
        },
    };
    // Fill the owner's live projection budget; the next flow is refused.
    let mut port = 18_000u16;
    let refused = loop {
        port += 1;
        assert!(port < 18_000 + 1024, "projection capacity is bounded");
        match role
            .install(
                incarnation(port),
                tuple(port),
                cutoff(120_000),
                scope.clone(),
            )
            .await
        {
            Ok(_) => {}
            Err(error) => break error,
        }
    };
    assert!(matches!(refused, meshway_underlay::Error::Capacity));
    // SecureNode retires the refused flow. It never reached the kernel: the
    // retirement completes and the role keeps serving every installed flow.
    role.retire_flow(incarnation(port)).await.unwrap();
    assert!(role.is_active());
    role.retire_flow(incarnation(18_001)).await.unwrap();
    role.install(
        incarnation(port + 1),
        tuple(port + 1),
        cutoff(120_000),
        scope.clone(),
    )
    .await
    .unwrap();
    assert!(role.is_active());
    host.suspend().await.unwrap();
    tokio::time::timeout(Duration::from_secs(6), async {
        while role.is_active() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("exit role ends on withdrawal");
    role.retire().await.unwrap();
    let _ = control.shutdown().await;
    host.shutdown(false).await.unwrap();
    server.close().await;
    println!("X8_NETMAP_EXIT_CAPACITY=refused-one,role-kept,slot-reused");
}
