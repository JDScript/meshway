//! Host resources under netmap snapshots: a same-content renewal touches
//! nothing, a larger serial moves only the resources whose exact purpose
//! changed, and the end of the lease closes descriptors and the exit role.
#![cfg(target_os = "linux")]

mod support;

use meshway_authority::{Clock, Deadline, Started};
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
    let server = Server::start(temp.path()).await;
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

    // Same content: the lease is raised in place for every clone; the Host
    // swaps nothing, and the old snapshot keeps minting operations.
    let before = snapshot.expires_at_ms();
    let renewed = sync(&mut control, &script, &first, false, HOUR).await;
    assert!(renewed.same_netmap(&snapshot));
    assert!(snapshot.expires_at_ms() > before);
    host.install(renewed.clone()).await.unwrap();
    assert_eq!(host.tun().unwrap().if_index().unwrap(), interface);
    udp.local_addr().unwrap();
    keep.local_addr().unwrap();
    sockets.listener(&snapshot, "keep").unwrap();

    // A larger serial changing only listener "udp".
    network.listeners.insert("udp".into(), listener(41003));
    let second = network.compile(Some(&first));
    let next = sync(&mut control, &script, &second, true, HOUR).await;
    assert!(snapshot.ensure_current().is_err());
    // Until the Host accepts the replacement no descriptor has I/O authority,
    // yet none is closed: the replacement is not a withdrawal.
    assert!(keep.local_addr().is_err() && udp.local_addr().is_err());
    tokio::time::sleep(Duration::from_millis(100)).await;
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
    println!("N1G_NETMAP_RESOURCES=renewal-noop,serial-diff,retained-tun");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an isolated Linux namespace, /dev/net/tun and NET_ADMIN"]
async fn netmap_lease_end_closes_descriptors_and_keeps_the_negative_boundary() {
    let temp = isolated();
    let server = Server::start(temp.path()).await;
    let (mut control, script) = server.enroll(temp.path()).await;
    let network = Network::new(&control, false);
    let netmap = network.compile(None);
    let snapshot = sync(&mut control, &script, &netmap, true, 4_000).await;
    let mut host = host(&temp).await;
    host.install(snapshot.clone()).await.unwrap();
    let keep = host
        .sockets()
        .datagram_listener(&snapshot, "keep")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(4_300)).await;
    assert!(snapshot.ensure_current().is_err());
    assert!(host.tun().is_none());
    assert!(keep.local_addr().is_err());
    released(41002).await;
    // Expiry is not a suspension: the capture block is still installed and an
    // expired grant is never revived by the same content.
    assert!(host.install(snapshot.clone()).await.is_err());
    drop(keep);
    let _ = control.shutdown().await;
    host.shutdown(false).await.unwrap();
    server.close().await;
    println!("N1G_NETMAP_EXPIRY=descriptors-closed,tun-inactive,no-revival");
}

fn deadline(clock: &Arc<Clock>, milliseconds: u64) -> Deadline {
    let started = Started::now(clock).unwrap();
    Deadline::from_response(started.clone(), started.wall() + milliseconds).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an isolated Linux exit namespace (ip_forward=1, loose RPF), nft and NET_ADMIN"]
async fn netmap_exit_role_survives_renewal_and_unchanged_serial_and_ends_with_its_lease() {
    let temp = isolated();
    let server = Server::start(temp.path()).await;
    let (mut control, script) = server.enroll(temp.path()).await;
    let mut network = Network::new(&control, true);
    let first = network.compile(None);
    let role_plan = first.netmap.exit_role.clone().expect("exit role");
    let snapshot = sync(&mut control, &script, &first, true, HOUR).await;
    let mut host = host(&temp).await;
    host.install(snapshot.clone()).await.unwrap();
    let role = host.install_netmap_exit_role(&role_plan).await.unwrap();
    let clock = Arc::new(Clock::new().unwrap());
    let scope = ExitScope {
        binding: meshway_model::Digest::from_bytes([72; 32]),
        revoked: CancellationToken::new(),
    };
    // One new projection per probe: the exit's own decision for source 2 on
    // its installed netmap (an Internet destination; no packet is sent).
    let mut next_port = 17_000u16;
    let mut project = |source: &str| {
        next_port += 1;
        let tuple = OriginalTuple {
            source: source.parse().unwrap(),
            destination: "8.8.8.8".parse().unwrap(),
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
            deadline(&clock, 20_000),
            scope.clone(),
        )
    };
    project("100.64.0.2").await.unwrap();
    // Another source address is not an exit grant.
    assert!(project("100.64.0.9").await.is_err());

    // Same content: nothing changes and the instance-free role is unchanged.
    let renewed = sync(&mut control, &script, &first, false, HOUR).await;
    host.install(renewed).await.unwrap();
    assert!(role.is_active());
    project("100.64.0.2").await.unwrap();

    // A larger serial with the same role: the replaced target mints no
    // operation, but the role outlives three maintenance ticks until the Host
    // accepts the continuation.
    network.listeners.insert("udp".into(), listener(41003));
    let second = network.compile(Some(&first));
    assert_eq!(second.netmap.exit_role, first.netmap.exit_role);
    let next = sync(&mut control, &script, &second, true, HOUR).await;
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(role.is_active());
    assert!(project("100.64.0.2").await.is_err());
    host.install(next).await.unwrap();
    assert!(role.is_active() && host.exit_role().is_some());
    project("100.64.0.2").await.unwrap();

    // Revoking the source's Access: no new projection, the role remains.
    network.access = false;
    let third = network.compile(Some(&second));
    let revoked = sync(&mut control, &script, &third, true, HOUR).await;
    host.install(revoked).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(role.is_active());
    assert!(project("100.64.0.2").await.is_err());

    // A short lease: when it ends, the owner stops and cleans up exactly.
    network.access = true;
    let fourth = network.compile(Some(&third));
    let short = sync(&mut control, &script, &fourth, true, 3_000).await;
    host.install(short).await.unwrap();
    tokio::time::timeout(Duration::from_secs(6), async {
        while role.is_active() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("exit role ends with its lease");
    role.retire().await.unwrap();
    assert!(host.exit_role().is_none());
    let _ = control.shutdown().await;
    host.shutdown(false).await.unwrap();
    server.close().await;
    println!("N1G_NETMAP_EXIT=renewal-noop,serial-retained,access-denied,lease-end-cleanup");
}
