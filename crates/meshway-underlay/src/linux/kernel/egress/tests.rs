use super::*;
use crate::linux::{
    kernel::{Kernel, journal::TunRecord},
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

struct Setup {
    _temp: tempfile::TempDir,
    tasks: Tasks,
    kernel: Kernel,
    backend: Backend,
    device: Device,
    mesh: Ranges,
    changed: tokio::sync::watch::Receiver<u64>,
}

/// A Kernel with an owned client TUN and an initialized capture backend.
async fn setup() -> Setup {
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(
        meshway_private_storage::Directory::open(&temp.path().join("private"), "test.lock")
            .unwrap(),
    );
    let tasks = Tasks::new(64);
    let mut kernel = Kernel::open(directory.clone(), Default::default(), tasks.clone())
        .await
        .unwrap();
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
    let namespace = crate::linux::kernel::namespace::Identity::current().unwrap();
    let mut random = [0; 16];
    getrandom::fill(&mut random).unwrap();
    let mesh = Ranges::of(pools.iter().copied());
    let ledger = Ledger {
        version: 2,
        boot: namespace.boot,
        namespace_device: namespace.device,
        namespace_inode: namespace.inode,
        namespace_cookie: namespace.cookie,
        program: program::Program {
            installation: random.iter().map(|byte| format!("{byte:02x}")).collect(),
            mark: kernel.journal.mark.mask,
        },
        table: 24000,
        plan: full(),
        gate: Gate {
            ranges: Ranges::default(),
            mesh: Ranges::default(),
            device: None,
        },
        pending_gate: None,
        receipt: None,
        rules: Vec::new(),
        removed: false,
    };
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
        limit: 65_536,
        start: kernel.journal.end() + 1,
        monitored: true,
    };
    backend.initialize().await.unwrap();
    Setup {
        _temp: temp,
        tasks,
        kernel,
        backend,
        device,
        mesh,
        changed,
    }
}

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
    // First prove IPv6 has a working underlay route, so a later failure cannot
    // be credited to a namespace that never had external IPv6 connectivity.
    exchange(None, ipv6, b"before-v6").await;
    let Setup {
        _temp: temp,
        tasks,
        mut kernel,
        mut backend,
        device,
        mesh,
        changed,
    } = setup().await;
    let full = full();
    println!("X3C_EGRESS_NFT={}", backend.ledger.program.create());
    backend.stage(full.clone(), mesh.clone()).await.unwrap();
    backend
        .commit_staged(full.clone(), mesh.clone(), Some(device.clone()), || Ok(()))
        .await
        .unwrap();
    kernel.quiesce().await.unwrap();
    let conflict = kernel.expected.lock().unwrap().first_conflict.clone();
    assert!(conflict.is_none(), "{conflict:?}");
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

/// `count` disjoint, non-adjacent public IPv4 ranges (about two prefixes each)
/// plus `count / 4` IPv6 ones, avoiding the fixture's own networks; `seed`
/// shifts them so that two plans differ everywhere.
fn scaled(count: u32, seed: u32, server: std::net::Ipv4Addr) -> EgressCapturePlan {
    let mut v4 = vec![(server, server)];
    let step = (0xdf00_0000u32 - 0x0100_0000) / count;
    for i in 0..count {
        // /24-aligned: one or two prefixes each, like a merged country set.
        let start = (0x0100_0000 + i * step) & !0xff | (((seed % 7) + 1) << 8);
        let end = start + 256 * (1 + i % 2) - 1;
        let first = std::net::Ipv4Addr::from(start);
        if [0x64u8, 0x7f, 0xc6].contains(&first.octets()[0]) {
            continue;
        }
        v4.push((first, std::net::Ipv4Addr::from(end)));
    }
    v4.sort();
    let v6 = (0..count / 4)
        .map(|i| {
            let start = (0x2400u128 << 112) | (u128::from(i) << 80) | (u128::from(seed) << 64);
            (
                std::net::Ipv6Addr::from(start),
                std::net::Ipv6Addr::from(start | 0xffff),
            )
        })
        .collect();
    let mut plan = full();
    plan.generation = Digest::from_bytes([seed as u8; 32]);
    plan.default_capture = false;
    plan.managed_ipv4 = v4;
    plan.sink_ipv6 = v6;
    plan
}

#[tokio::test]
#[ignore = "requires isolated dual-stack Linux client/service namespaces and NET_ADMIN"]
async fn large_partitions_install_switch_read_back_and_retire() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let server: SocketAddr = std::env::var("MESHWAY_X3C_SERVER")
        .unwrap()
        .parse()
        .unwrap();
    let direct: SocketAddr = std::env::var("MESHWAY_X3C_DIRECT")
        .unwrap()
        .parse()
        .unwrap();
    let IpAddr::V4(managed) = server.ip() else {
        panic!("IPv4 fixture server");
    };
    // The namespace's own IPv6 addresses have settled: no address completes
    // duplicate detection later, during the measurement.
    for _ in 0..100 {
        let tentative = std::process::Command::new("/usr/sbin/ip")
            .args(["-6", "addr", "show", "dev", "eth0", "tentative"])
            .output()
            .unwrap();
        if tentative.stdout.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let Setup {
        _temp,
        tasks,
        mut kernel,
        mut backend,
        device,
        mesh,
        changed,
    } = setup().await;
    let sizes: Vec<u32> = std::env::var("MESHWAY_R3_SIZES")
        .unwrap_or_else(|_| "1000,6000,10000,13000".into())
        .split(',')
        .map(|size| size.parse().unwrap())
        .collect();
    let elapsed = |start: std::time::Instant| start.elapsed().as_secs_f64();
    for (round, size) in sizes.iter().enumerate() {
        let mut timings = Vec::new();
        for seed in [1u32, 2] {
            let plan = scaled(*size, seed + round as u32 * 2, managed);
            let ranges = plan.managed_ipv4.len() + plan.sink_ipv6.len();
            let routes = plan::parse(&plan).unwrap().routes().count();
            let start = std::time::Instant::now();
            backend.stage(plan.clone(), mesh.clone()).await.unwrap();
            let staged = elapsed(start);
            let start = std::time::Instant::now();
            backend
                .commit_staged(plan.clone(), mesh.clone(), Some(device.clone()), || Ok(()))
                .await
                .unwrap();
            let committed = elapsed(start);
            let start = std::time::Instant::now();
            backend.verify().await.unwrap();
            let verified = elapsed(start);
            let table = super::super::get_routes(&kernel.handle, 65_536)
                .await
                .unwrap()
                .into_iter()
                .filter(|route| super::super::route_table(route) == backend.ledger.table)
                .count();
            assert_eq!(table, routes);
            kernel.quiesce().await.unwrap();
            kernel.verify().await.unwrap();
            assert!(!changed.has_changed().unwrap());
            // The managed server goes to the TUN, a destination outside every
            // range stays on the host route.
            captured(kernel.tun.as_ref().unwrap(), server).await;
            exchange(None, direct, b"outside-the-partition").await;
            timings.push(serde_json::json!({
                "seed": seed, "ranges": ranges, "routes": routes,
                "stageSeconds": staged, "commitSeconds": committed, "readbackSeconds": verified,
            }));
        }
        let start = std::time::Instant::now();
        backend.pause().await.unwrap();
        let paused = elapsed(start);
        assert!(
            tokio::time::timeout(
                Duration::from_secs(1),
                tokio::net::TcpStream::connect(server)
            )
            .await
            .is_ok_and(|result| result.is_err()),
            "a paused managed destination is rejected at once"
        );
        println!(
            "R3_SCALE={}",
            serde_json::json!({"size": size, "switches": timings, "pauseSeconds": paused})
        );
    }
    let start = std::time::Instant::now();
    backend.retire().await.unwrap();
    let retired = start.elapsed().as_secs_f64();
    assert!(
        super::super::get_routes(&kernel.handle, 65_536)
            .await
            .unwrap()
            .iter()
            .all(|route| super::super::route_table(route) != backend.ledger.table)
    );
    kernel.quiesce().await.unwrap();
    exchange(None, server, b"after-retire").await;
    kernel.down().await.unwrap();
    tasks.shutdown().await.unwrap();
    println!(
        "R3_SCALE_RETIRE={}",
        serde_json::json!({"retireSeconds": retired})
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
    assert_eq!(
        ledger.gate.ranges.v4,
        [(
            std::net::Ipv4Addr::UNSPECIFIED,
            std::net::Ipv4Addr::BROADCAST
        )]
    );
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
        "X3C_CAPTURE_RESTART=positive-owner-sigkill,no-positive-tun,negative-default-kept,protected-server-live,explicit-down-releases"
    );
}

async fn exchange(kernel: Option<&Kernel>, remote: SocketAddr, body: &[u8]) {
    let socket = if remote.is_ipv4() {
        tokio::net::TcpSocket::new_v4()
    } else {
        tokio::net::TcpSocket::new_v6()
    }
    .unwrap();
    if let Some(kernel) = kernel
        && let Err(error) = kernel.shared.prepare(&socket2::SockRef::from(&socket))
    {
        let conflict = kernel.expected.lock().unwrap().first_conflict.clone();
        panic!("{error:?}: {conflict:?}");
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
        managed_ipv4: vec![(
            std::net::Ipv4Addr::UNSPECIFIED,
            std::net::Ipv4Addr::BROADCAST,
        )],
        default_capture: true,
        sink_ipv6: vec![(
            std::net::Ipv6Addr::UNSPECIFIED,
            std::net::Ipv6Addr::from(u128::MAX),
        )],
        unavailable: None,
    }
}
fn narrow(managed: IpAddr, direct: IpAddr) -> EgressCapturePlan {
    let (IpAddr::V4(managed), IpAddr::V4(_)) = (managed, direct) else {
        panic!("fixture IPv4 endpoints");
    };
    let mut plan = full();
    plan.generation = Digest::from_bytes([32; 32]);
    plan.default_capture = false;
    plan.managed_ipv4 = vec![(managed, managed)];
    plan
}

/// A consistent read-back holds; a read-back that lacks one of the table's
/// rules, or no table at all, is a change (only that ends kernel ownership;
/// a failed read is `Unavailable`, see `owner::unverified_tests`).
#[tokio::test]
#[ignore = "requires an isolated Linux network namespace, TUN, nft and NET_ADMIN"]
async fn verification_distinguishes_a_missing_rule_and_table_from_a_consistent_table() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let Setup {
        _temp,
        tasks,
        mut backend,
        ..
    } = setup().await;
    let verification = backend.check().await;
    assert!(
        matches!(verification, backend::Verification::Holds),
        "{verification}"
    );
    let table = backend.ledger.program.table();
    let nft = |arguments: &[&str]| {
        let output = std::process::Command::new("/usr/sbin/nft")
            .args(arguments)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        output.stdout
    };
    let listed: serde_json::Value =
        serde_json::from_slice(&nft(&["-j", "list", "table", "inet", &table])).unwrap();
    let rule = listed["nftables"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|entry| entry.get("rule"))
        .expect("the capture table has rules");
    let (chain, handle) = (
        rule["chain"].as_str().unwrap(),
        rule["handle"].as_u64().unwrap().to_string(),
    );
    nft(&["delete", "rule", "inet", &table, chain, "handle", &handle]);
    let verification = backend.check().await;
    assert!(
        matches!(
            verification,
            backend::Verification::Changed("objects_changed")
        ),
        "{verification}"
    );
    nft(&["delete", "table", "inet", &table]);
    let verification = backend.check().await;
    assert!(
        matches!(
            verification,
            backend::Verification::Changed("table_missing")
        ),
        "{verification}"
    );
    tasks.shutdown().await.unwrap();
}
