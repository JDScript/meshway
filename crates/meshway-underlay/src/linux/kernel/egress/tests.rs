use super::*;
use crate::linux::{
    kernel::{
        Kernel,
        journal::{Journal, TunRecord},
    },
    tasks::Tasks,
};
use backend::Backend;
use meshway_configuration::execution::EgressCapturePlan;
use meshway_model::Digest;
use plan::Ranges;
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use storage::{Device, Gate, Ledger, Store};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
#[ignore = "requires isolated dual-stack Linux client/service namespaces and NET_ADMIN"]
async fn default_capture_protected_endpoints_ipv6_sink_and_switch_without_direct_window() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let server: SocketAddr = std::env::var("MESHWAY_X3C_SERVER")
        .unwrap()
        .parse()
        .unwrap();
    let peer: SocketAddr = std::env::var("MESHWAY_X3C_PEER").unwrap().parse().unwrap();
    let ipv6: SocketAddr = std::env::var("MESHWAY_X3C_IPV6").unwrap().parse().unwrap();
    let direct: SocketAddr = std::env::var("MESHWAY_X3C_DIRECT")
        .unwrap()
        .parse()
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(
        meshway_private_storage::Directory::open(&temp.path().join("private"), "test.lock")
            .unwrap(),
    );
    let tasks = Tasks::new(64);
    let mut kernel = Kernel::open(directory.clone(), Default::default(), tasks.clone())
        .await
        .unwrap();
    // First prove IPv6 has a working underlay route, so a later failure cannot
    // be credited to a namespace that never had external IPv6 connectivity.
    exchange(None, ipv6, b"before-v6").await;
    let pools = vec![
        "100.96.0.0/11".parse().unwrap(),
        "fd42::/64".parse().unwrap(),
    ];
    kernel.retain_negative(&pools).await.unwrap();
    let addresses = meshway_configuration::NodeAddresses {
        ipv4: Some("100.96.0.2".parse().unwrap()),
        ipv6: Some("fd42::2".parse().unwrap()),
    };
    let tun = {
        let mut expected = kernel.expected.lock().unwrap();
        expected.changing = true;
        let tun = meshway_tun::Tun::create(
            meshway_tun::Options::new(1280)
                .unwrap()
                .with_ipv4(addresses.ipv4.unwrap(), 32)
                .unwrap()
                .with_ipv6(addresses.ipv6.unwrap(), 128)
                .unwrap(),
        )
        .unwrap();
        expected.tun = Some((
            tun.if_index().unwrap(),
            tun.name().to_owned(),
            tun.mtu(),
            vec![
                addresses.ipv4.unwrap().into(),
                addresses.ipv6.unwrap().into(),
            ],
        ));
        tun
    };
    let device = Device {
        index: tun.if_index().unwrap(),
        name: tun.name().to_owned(),
        source: addresses.ipv4.unwrap(),
    };
    kernel.journal.tun = Some(TunRecord {
        name: device.name.clone(),
        ifindex: device.index,
        mtu: tun.mtu(),
        addresses: addresses.clone(),
    });
    kernel.store.persist(&kernel.journal).await.unwrap();
    kernel.tun = Some(tun);
    kernel
        .address_ready(device.index, &addresses)
        .await
        .unwrap();
    kernel.quiesce().await.unwrap();
    kernel.expected.lock().unwrap().changing = false;
    let mut changed = kernel.shared.network_changes.subscribe();
    changed.borrow_and_update();
    let (store, old) = Store::open(directory).await.unwrap();
    assert!(old.is_none());
    let (boot, namespace_device, namespace_inode) = Journal::identity().unwrap();
    let full = full();
    let mut random = [0; 16];
    getrandom::fill(&mut random).unwrap();
    let mesh = Ranges {
        v4: vec![pools[0]],
        v6: vec![pools[1]],
    };
    let ledger = Ledger {
        version: 1,
        boot,
        namespace_device,
        namespace_inode,
        program: program::Program {
            installation: random.iter().map(|byte| format!("{byte:02x}")).collect(),
            mark: kernel.journal.mark.mask,
        },
        table: 24000,
        plan: full.clone(),
        gate: Gate {
            ranges: Ranges::default(),
            mesh: Ranges::default(),
            device: None,
        },
        pending_gate: None,
        receipt: None,
        routes: Vec::new(),
        rules: Vec::new(),
        removed: false,
    };
    println!("X3C_EGRESS_NFT={}", ledger.program.create());
    let commands =
        super::super::tool::command::Commands::recorded(super::super::tool::child::Fence {
            directory: store.directory.clone(),
            installation: ledger.program.installation.clone(),
        });
    let mut backend = Backend {
        store,
        ledger,
        commands,
        handle: kernel.handle.clone(),
        expected: kernel.expected.clone(),
        limit: 16384,
        start: kernel.journal.end() + 1,
        monitored: true,
    };
    backend.initialize().await.unwrap();
    backend.stage(full.clone(), mesh.clone()).await.unwrap();
    backend
        .commit_staged(full.clone(), mesh.clone(), Some(device.clone()), || Ok(()))
        .await
        .unwrap();
    kernel.quiesce().await.unwrap();
    kernel.verify().await.unwrap();
    assert!(
        !changed.has_changed().unwrap(),
        "owned capture mutations are not external network changes"
    );
    exchange(Some(&kernel), server, b"protected-server").await;
    exchange(Some(&kernel), peer, b"protected-peer").await;
    let flushed = std::process::Command::new("/usr/sbin/ip")
        .args(["-6", "neigh", "flush", "dev", "eth0"])
        .status()
        .unwrap();
    assert!(flushed.success());
    exchange(Some(&kernel), ipv6, b"protected-v6-fresh-neighbour").await;
    let broadcast = tokio::net::UdpSocket::bind("0.0.0.0:0").await.unwrap();
    broadcast.set_broadcast(true).unwrap();
    broadcast
        .send_to(
            b"leak-broadcast",
            (std::net::Ipv4Addr::BROADCAST, server.port()),
        )
        .await
        .unwrap();
    for remote in [server, peer] {
        captured(kernel.tun.as_ref().unwrap(), remote).await;
    }
    assert!(
        tokio::time::timeout(Duration::from_secs(1), tokio::net::TcpStream::connect(ipv6))
            .await
            .is_ok_and(|result| result.is_err())
    );

    let stop = tokio_util::sync::CancellationToken::new();
    let ending = stop.clone();
    let remote = server;
    let traffic = tokio::spawn(async move {
        let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await.unwrap();
        let mut tick = tokio::time::interval(Duration::from_millis(2));
        loop {
            tokio::select! { _ = ending.cancelled() => break, _ = tick.tick() => { let _ = socket.send_to(b"leak-switch", remote).await; } }
        }
    });
    let narrow = narrow(server.ip(), direct.ip());
    for plan in [narrow.clone(), full.clone(), narrow.clone()] {
        backend.stage(plan.clone(), mesh.clone()).await.unwrap();
        backend
            .commit_staged(plan, mesh.clone(), Some(device.clone()), || Ok(()))
            .await
            .unwrap();
    }
    stop.cancel();
    traffic.await.unwrap();
    exchange(Some(&kernel), server, b"assert-no-leak").await;
    exchange(None, direct, b"explicit-direct").await;
    if std::env::var("MESHWAY_X3C_CAPTURE_CRASH").as_deref() == Ok("1") {
        backend.stage(full.clone(), mesh.clone()).await.unwrap();
        backend
            .commit_staged(full.clone(), mesh.clone(), Some(device.clone()), || Ok(()))
            .await
            .unwrap();
        std::fs::write(
            "/root/x3c-capture-crash.json",
            serde_json::to_vec(&CrashPoint {
                path: temp.path().join("private"),
                pid: std::process::id(),
            })
            .unwrap(),
        )
        .unwrap();
        std::future::pending::<()>().await;
    }
    backend.pause().await.unwrap();
    // No positive route exists; overlapping managed destinations stay denied.
    assert!(
        tokio::time::timeout(
            Duration::from_secs(1),
            tokio::net::TcpStream::connect(server)
        )
        .await
        .is_ok_and(|result| result.is_err())
    );
    exchange(Some(&kernel), server, b"protected-negative").await;
    kernel.quiesce().await.unwrap();
    kernel.verify().await.unwrap();
    assert!(!changed.has_changed().unwrap());
    println!(
        "X3C_EGRESS_JOURNAL={}",
        serde_json::to_string(&backend.ledger).unwrap()
    );
    backend.retire().await.unwrap();
    kernel.quiesce().await.unwrap();
    exchange(None, server, b"after-retire").await;
    kernel.down().await.unwrap();
    tasks.shutdown().await.unwrap();
    println!(
        "X3C_EGRESS=server-peer-protected,ordinary-same-ip-captured,ipv6-sink,union-switch-no-leak,explicit-direct,negative-fallback,clean-retire"
    );
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CrashPoint {
    path: std::path::PathBuf,
    pid: u32,
}

#[tokio::test]
#[ignore = "requires isolated namespace after SIGKILL of a positive /0 capture owner"]
async fn egress_capture_crash_keeps_negative_until_explicit_down() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let point: CrashPoint =
        serde_json::from_slice(&std::fs::read("/root/x3c-capture-crash.json").unwrap()).unwrap();
    assert!(!std::path::Path::new(&format!("/proc/{}", point.pid)).exists());
    let server: SocketAddr = std::env::var("MESHWAY_X3C_SERVER")
        .unwrap()
        .parse()
        .unwrap();
    let directory =
        Arc::new(meshway_private_storage::Directory::open(&point.path, "test.lock").unwrap());
    let tasks = Tasks::new(64);
    let mut kernel = Kernel::open(directory.clone(), Default::default(), tasks.clone())
        .await
        .unwrap();
    assert!(kernel.tun.is_none());
    let (store, ledger) = Store::open(directory).await.unwrap();
    let ledger = ledger.unwrap();
    assert!(!ledger.removed && ledger.gate.device.is_none());
    assert_eq!(ledger.gate.ranges.v4, vec!["0.0.0.0/0".parse().unwrap()]);
    assert!(ledger.routes.is_empty());
    drop(store);
    if let Ok(Ok(_)) = tokio::time::timeout(
        Duration::from_millis(250),
        tokio::net::TcpStream::connect(server),
    )
    .await
    {
        panic!("restart released managed traffic to the underlay");
    }
    exchange(Some(&kernel), server, b"restart-protected").await;
    kernel.down().await.unwrap();
    exchange(None, server, b"down-releases-capture").await;
    tasks.shutdown().await.unwrap();
    println!(
        "X3C_CAPTURE_RESTART=positive-owner-sigkill,no-positive-tun,negative-default-retained,protected-server-live,explicit-down-releases"
    );
}

async fn exchange(kernel: Option<&Kernel>, remote: SocketAddr, body: &[u8]) {
    let socket = if remote.is_ipv4() {
        tokio::net::TcpSocket::new_v4()
    } else {
        tokio::net::TcpSocket::new_v6()
    }
    .unwrap();
    if let Some(kernel) = kernel {
        kernel
            .shared
            .prepare(&socket2::SockRef::from(&socket))
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut stream = socket.connect(remote).await.unwrap();
        stream.write_all(body).await.unwrap();
        let mut reply = vec![0; body.len()];
        stream.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply, body);
    })
    .await
    .unwrap();
}
async fn captured(tun: &meshway_tun::Tun, remote: SocketAddr) {
    let mut connect = Box::pin(tokio::net::TcpStream::connect(remote));
    let mut packet = vec![0; 1280];
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            tokio::select! {
                result = &mut connect => panic!("ordinary connect escaped capture: {result:?}"),
                size = tun.recv(&mut packet) => {
                    let size = size.unwrap();
                    if size >= 40 && packet[0] == 0x45 && packet[9] == 6 && u16::from_be_bytes([packet[22], packet[23]]) == remote.port() { break; }
                }
            }
        }
    }).await.unwrap();
}
fn full() -> EgressCapturePlan {
    EgressCapturePlan {
        generation: Digest::from_bytes([31; 32]),
        managed_ipv4: vec!["0.0.0.0/0".into()],
        default_capture: true,
        direct_ipv4: Vec::new(),
        unmanaged_ipv4: Vec::new(),
        sink_ipv6: vec!["::/0".into()],
        unavailable: None,
    }
}
fn narrow(managed: IpAddr, direct: IpAddr) -> EgressCapturePlan {
    let (IpAddr::V4(managed), IpAddr::V4(direct)) = (managed, direct) else {
        panic!("fixture IPv4 endpoints");
    };
    let mut plan = full();
    plan.generation = Digest::from_bytes([32; 32]);
    plan.default_capture = false;
    plan.managed_ipv4 = vec![format!("{managed}/32")];
    plan.direct_ipv4 = vec![format!("{direct}/32")];
    let mut remaining = vec!["0.0.0.0/0".parse::<ipnet::Ipv4Net>().unwrap()];
    while let Some(net) = remaining.pop() {
        if !net.contains(&managed) && !net.contains(&direct) {
            plan.unmanaged_ipv4.push(net.to_string());
        } else if net.prefix_len() < 32 {
            remaining.extend(net.subnets(net.prefix_len() + 1).unwrap());
        }
    }
    plan
}
