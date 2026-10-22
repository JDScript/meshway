use super::io::ExitCutoff as Deadline;
use super::*;
use crate::linux::{kernel::Kernel, tasks::Tasks};
use meshway_configuration::execution::ExitNat;
use meshway_model::{Digest, exit::ExitFamily};
use meshway_private_storage::Directory;
use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, SocketAddrV4},
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

fn id(byte: u8) -> ExitIncarnation {
    ExitIncarnation::new([byte; 32])
}

#[tokio::test]
#[ignore = "requires isolated Linux exit/service veth namespaces and NET_ADMIN"]
async fn exit_nat_udp_real_reply_qualification_confirm_retire_and_expiry() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let remote: SocketAddrV4 = std::env::var("MESHWAY_X3_EXTERNAL")
        .unwrap()
        .parse()
        .unwrap();
    let external_source = std::env::var("MESHWAY_X3_EXPECT_SOURCE").unwrap();
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
    let tun = {
        let mut expected = kernel.expected.lock().unwrap();
        let tun = meshway_tun::Tun::create(meshway_tun::Options::new(1280).unwrap()).unwrap();
        expected.exit_tun = Some(tun.if_index().unwrap());
        expected.exit_tables.push(20000);
        tun
    };
    kernel.quiesce().await.unwrap();
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).unwrap();
    let mark = if kernel.journal.mark.mask != 1 { 1 } else { 2 };
    let namespace = crate::linux::kernel::namespace::Identity::current().unwrap();
    let (store, previous) = storage::Store::open(kernel.store.directory())
        .await
        .unwrap();
    assert!(previous.is_none());
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
            families: vec![ExitFamily::Ipv4],
            nat: ExitNat::Masquerade,
        },
        program: program::Program {
            legacy: false,
            installation: nonce.iter().map(|byte| format!("{byte:02x}")).collect(),
            interface: tun.if_index().unwrap(),
            interface_name: tun.name().to_owned(),
            mark,
            pools: vec!["100.96.0.0/11".parse().unwrap()],
            families: vec![ExitFamily::Ipv4],
            pools6: Vec::new(),
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
    println!("X3_NFT_PROGRAM={}", ledger.program.create());
    let mut backend = backend::Backend::create(
        store,
        ledger,
        routing,
        Arc::new(RwLock::new(BTreeMap::new())),
        vec![external_source.parse().unwrap(), Ipv4Addr::LOCALHOST.into()],
        CancellationToken::new(),
    )
    .await
    .unwrap();
    kernel.quiesce().await.unwrap();
    println!(
        "X3_JOURNAL_INSTALLED={}",
        serde_json::to_string(&backend.ledger).unwrap()
    );
    let recovered_receipt = objects::recover_create(&mut backend.commands, &backend.ledger.program)
        .await
        .unwrap();
    assert_eq!(
        recovered_receipt.objects,
        backend.ledger.receipt.as_ref().unwrap().objects
    );
    assert_eq!(
        recovered_receipt.handles,
        backend.ledger.receipt.as_ref().unwrap().handles
    );
    let tuple = OriginalTuple {
        source: "100.96.0.1".parse().unwrap(),
        destination: (*remote.ip()).into(),
        transport: ExitTransport::Udp {
            source_port: 17001,
            destination_port: remote.port(),
        },
    };
    let packet = udp(tuple, b"x3-qualified");
    let mut reply = vec![0; 1280];
    tun.send(&packet).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), recv_ipv4(&tun, &mut reply))
            .await
            .is_err()
    );
    let initial_deadline = cutoff(30_000);
    let scope = ExitScope {
        binding: Digest::from_bytes([2; 32]),
        revoked: CancellationToken::new(),
    };
    assert_eq!(
        backend
            .install(id(1), tuple, initial_deadline, scope.clone())
            .await
            .unwrap(),
        ProjectionAck { incarnation: id(1) }
    );
    kernel.quiesce().await.unwrap();
    tun.send(&packet).await.unwrap();
    assert!(matches!(
        backend.confirm(id(1)).await.unwrap(),
        CtConfirmation::Confirmed { .. }
    ));
    println!(
        "X3_CT_CONFIRMED={}",
        serde_json::to_string(&backend.ledger.projections[0].ct).unwrap()
    );
    let size = tokio::time::timeout(Duration::from_secs(2), recv_ipv4(&tun, &mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&reply[12..16], &remote.ip().octets());
    assert_eq!(&reply[16..20], &v4(tuple.source).octets());
    assert_eq!(
        &reply[28..size],
        format!("from={external_source}|x3-qualified").as_bytes()
    );
    backend
        .renew(id(1), cutoff(30_000), scope.clone())
        .await
        .unwrap();
    backend.retire(id(1)).await.unwrap();
    assert!(
        conntrack::lookup_owned(tuple, mark)
            .await
            .unwrap()
            .is_none()
    );
    tun.send(&packet).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), recv_ipv4(&tun, &mut reply))
            .await
            .is_err()
    );
    adversarial::conflicts(&mut backend, &tun, tuple).await;
    protocols::round_trips(&mut backend, &tun, tuple, &external_source).await;
    // A retired projection leaves the history once its conntrack is deleted
    // and its reply tuple quarantined: the same original tuple may be
    // projected again under a new incarnation.
    backend
        .install(id(2), tuple, cutoff(30_000), scope.clone())
        .await
        .unwrap();
    backend.retire(id(2)).await.unwrap();
    // The kernel deletes a projection's confirmed entry (masquerade, when the
    // outer address it translated to leaves on a roam); the flow's next
    // packet creates its successor. Retiring the projection retires that
    // successor too: it is not foreign, and cleanup completes.
    let roamed = OriginalTuple {
        transport: ExitTransport::Udp {
            source_port: 17005,
            destination_port: remote.port(),
        },
        ..tuple
    };
    let roamed_packet = udp(roamed, b"x3-roamed");
    backend
        .install(id(6), roamed, cutoff(30_000), scope.clone())
        .await
        .unwrap();
    kernel.quiesce().await.unwrap();
    tun.send(&roamed_packet).await.unwrap();
    assert!(matches!(
        backend.confirm(id(6)).await.unwrap(),
        CtConfirmation::Confirmed { .. }
    ));
    tokio::time::timeout(Duration::from_secs(2), recv_ipv4(&tun, &mut reply))
        .await
        .unwrap()
        .unwrap();
    let confirmed = backend
        .ledger
        .projections
        .iter()
        .find(|flow| flow.incarnation == id(6))
        .and_then(|flow| flow.ct.clone())
        .unwrap();
    conntrack::delete(roamed, &confirmed, mark).await.unwrap();
    tun.send(&roamed_packet).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), recv_ipv4(&tun, &mut reply))
        .await
        .unwrap()
        .unwrap();
    let successor = conntrack::lookup_owned(roamed, mark)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        successor.id, confirmed.id,
        "the flow's next packet made a new entry"
    );
    backend.retire(id(6)).await.unwrap();
    assert!(
        conntrack::lookup_owned(roamed, mark)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !backend
            .ledger
            .projections
            .iter()
            .any(|flow| flow.incarnation == id(6))
    );
    let expiring = OriginalTuple {
        transport: ExitTransport::Udp {
            source_port: 17002,
            destination_port: remote.port(),
        },
        ..tuple
    };
    backend
        .install(id(3), expiring, cutoff(5_000), scope)
        .await
        .unwrap();
    kernel.quiesce().await.unwrap();
    tokio::time::sleep(Duration::from_millis(2100)).await;
    tun.send(&udp(expiring, b"expired")).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), recv_ipv4(&tun, &mut reply))
            .await
            .is_err()
    );
    let missing = OriginalTuple {
        transport: ExitTransport::Udp {
            source_port: 17003,
            destination_port: remote.port(),
        },
        ..tuple
    };
    backend
        .install(
            id(4),
            missing,
            cutoff(30_000),
            ExitScope {
                binding: Digest::from_bytes([3; 32]),
                revoked: CancellationToken::new(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        backend.confirm(id(4)).await.unwrap(),
        CtConfirmation::ConfirmFailed { incarnation: id(4) }
    );
    tun.send(&udp(missing, b"confirm-failed")).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), recv_ipv4(&tun, &mut reply))
            .await
            .is_err()
    );
    let crash = OriginalTuple {
        transport: ExitTransport::Udp {
            source_port: 17004,
            destination_port: remote.port(),
        },
        ..tuple
    };
    backend
        .install(
            id(5),
            crash,
            cutoff(30_000),
            ExitScope {
                binding: Digest::from_bytes([4; 32]),
                revoked: CancellationToken::new(),
            },
        )
        .await
        .unwrap();
    tun.send(&udp(crash, b"crash-live")).await.unwrap();
    assert!(matches!(
        backend.confirm(id(5)).await.unwrap(),
        CtConfirmation::Confirmed { .. }
    ));
    tokio::time::timeout(Duration::from_secs(2), recv_ipv4(&tun, &mut reply))
        .await
        .unwrap()
        .unwrap();
    if std::env::var("MESHWAY_X3_CRASH").as_deref() == Ok("1") {
        std::fs::write(
            "/root/x3-crash.json",
            serde_json::to_vec(&CrashPoint {
                path: temp.path().to_owned(),
                pid: std::process::id(),
            })
            .unwrap(),
        )
        .unwrap();
        // The fixture kills this real process after the durable live CT state
        // above. No Rust destructor, down, or role cleanup runs on this branch.
        std::future::pending::<()>().await;
    }
    // Drop the process-owned objects without any role/down cleanup. The same
    // namespace owner must recover the persisted projection, not restore it.
    drop(backend);
    drop(tun);
    drop(kernel);
    tasks.shutdown().await.unwrap();
    let directory = Arc::new(Directory::open(temp.path(), "node.lock").unwrap());
    let tasks = Tasks::new(64);
    let mut recovered = Kernel::open(directory, Default::default(), tasks.clone())
        .await
        .unwrap();
    assert!(
        conntrack::lookup_owned(crash, mark)
            .await
            .unwrap()
            .is_none()
    );
    let (_, ledger) = storage::Store::open(recovered.store.directory())
        .await
        .unwrap();
    assert!(ledger.unwrap().removed);
    recovered.down().await.unwrap();
    tasks.shutdown().await.unwrap();
}

mod adversarial;
mod changes;
mod cleanup;
mod dumps;
mod protocols;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CrashPoint {
    path: std::path::PathBuf,
    pid: u32,
}

#[tokio::test]
#[ignore = "requires isolated Linux namespace after fixture SIGKILL of the live exit test"]
async fn exit_recovers_real_sigkill_live_ct_in_same_namespace() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let point: CrashPoint =
        serde_json::from_slice(&std::fs::read("/root/x3-crash.json").unwrap()).unwrap();
    assert!(!std::path::Path::new(&format!("/proc/{}", point.pid)).exists());
    let directory = Arc::new(Directory::open(&point.path, "node.lock").unwrap());
    let (store, prior) = storage::Store::open(directory.clone()).await.unwrap();
    let prior = prior.unwrap();
    assert!(!prior.removed && !prior.closing);
    let expected = if prior.program.legacy {
        let mut bytes = [0; 32];
        bytes[31] = 5;
        ExitIncarnation::new(bytes)
    } else {
        id(5)
    };
    let record = prior
        .projections
        .iter()
        .find(|flow| flow.incarnation == expected)
        .unwrap();
    assert!(!record.retired && record.ct.is_some());
    let mut commands = command::Commands::default();
    assert!(
        conntrack::lookup_owned(record.tuple, prior.program.mark)
            .await
            .unwrap()
            .is_some()
    );
    drop(store);
    let tasks = Tasks::new(64);
    let mut kernel = Kernel::open(directory, Default::default(), tasks.clone())
        .await
        .unwrap();
    assert!(
        conntrack::lookup_owned(record.tuple, prior.program.mark)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        objects::present(&mut commands, &prior.program)
            .await
            .unwrap(),
        0
    );
    let (store, recovered) = storage::Store::open(kernel.store.directory())
        .await
        .unwrap();
    assert!(recovered.unwrap().removed);
    drop(store);
    kernel.down().await.unwrap();
    drop(kernel);
    tasks.shutdown().await.unwrap();
    std::fs::remove_dir_all(&point.path).unwrap();
    println!(
        "X3_SIGKILL_RECOVERY=live-owned-ct-before,absent-after,private-tables-absent,journal-removed"
    );
}

pub(super) fn cutoff(milliseconds: u64) -> Deadline {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    Deadline {
        at: std::time::Instant::now() + std::time::Duration::from_millis(milliseconds),
        ms: now + milliseconds,
    }
}

async fn recv_ipv4(tun: &meshway_tun::Tun, packet: &mut [u8]) -> Result<usize, meshway_tun::Error> {
    loop {
        let size = tun.recv(packet).await?;
        if size > 0 && packet[0] >> 4 == 4 {
            return Ok(size);
        }
    }
}

fn udp(tuple: OriginalTuple, payload: &[u8]) -> Vec<u8> {
    let (_, source, destination) = tuple.fields();
    let mut packet = vec![0; 28 + payload.len()];
    packet[0] = 0x45;
    let length = u16::try_from(packet.len()).unwrap();
    packet[2..4].copy_from_slice(&length.to_be_bytes());
    packet[8] = 64;
    packet[9] = 17;
    packet[12..16].copy_from_slice(&v4(tuple.source).octets());
    packet[16..20].copy_from_slice(&v4(tuple.destination).octets());
    packet[20..22].copy_from_slice(&source.to_be_bytes());
    packet[22..24].copy_from_slice(&destination.to_be_bytes());
    packet[24..26].copy_from_slice(&(length - 20).to_be_bytes());
    packet[28..].copy_from_slice(payload);
    let mut sum: u32 = packet[..20]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| u32::from(u16::from_be_bytes([bytes[0], bytes[1]])))
        .sum();
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    packet[10..12].copy_from_slice(&(!(sum as u16)).to_be_bytes());
    packet
}

/// The IPv4 address of an IPv4 projection, for building its packets.
pub(super) fn v4(address: std::net::IpAddr) -> Ipv4Addr {
    match address {
        std::net::IpAddr::V4(address) => address,
        std::net::IpAddr::V6(_) => panic!("an IPv4 projection"),
    }
}

/// The IPv6 half of an exit role: the NAT66 program, the IPv6 forward and
/// ICMPv6 rules and one source's /128 pair rules and route install, read
/// back exactly and leave nothing. The namespace provides an IPv6 route to
/// `MESHWAY_X6_REMOTE` and IPv6 forwarding.
#[tokio::test]
#[ignore = "requires isolated Linux NFT/TUN namespace with IPv6 forwarding and NET_ADMIN"]
async fn exit_ipv6_program_rules_and_routes_install_and_leave_nothing() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let remote: std::net::IpAddr = std::env::var("MESHWAY_X6_REMOTE").unwrap().parse().unwrap();
    let temp = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let directory = Arc::new(Directory::open(temp.path(), "node.lock").unwrap());
    let tasks = Tasks::new(64);
    let mut kernel = Kernel::open(directory, Default::default(), tasks.clone())
        .await
        .unwrap();
    kernel
        .retain_negative(&[
            "100.96.0.0/11".parse().unwrap(),
            "fd79::/48".parse().unwrap(),
        ])
        .await
        .unwrap();
    let tun = {
        let mut expected = kernel.expected.lock().unwrap();
        let tun = meshway_tun::Tun::create(meshway_tun::Options::new(1280).unwrap()).unwrap();
        expected.exit_tun = Some(tun.if_index().unwrap());
        expected.exit_tables.push(20000);
        tun
    };
    kernel.quiesce().await.unwrap();
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).unwrap();
    let mark = if kernel.journal.mark.mask != 1 { 1 } else { 2 };
    let namespace = crate::linux::kernel::namespace::Identity::current().unwrap();
    let (store, _) = storage::Store::open(kernel.store.directory())
        .await
        .unwrap();
    let families = vec![ExitFamily::Ipv4, ExitFamily::Ipv6];
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
            families: families.clone(),
            nat: ExitNat::Masquerade,
        },
        program: program::Program {
            legacy: false,
            installation: nonce.iter().map(|byte| format!("{byte:02x}")).collect(),
            interface: tun.if_index().unwrap(),
            interface_name: tun.name().to_owned(),
            mark,
            pools: vec!["100.96.0.0/11".parse().unwrap()],
            families,
            pools6: vec!["fd79::/48".parse().unwrap()],
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
    let mut backend = backend::Backend::create(
        store,
        ledger,
        routing,
        Arc::new(RwLock::new(BTreeMap::new())),
        vec![std::net::Ipv6Addr::LOCALHOST.into()],
        CancellationToken::new(),
    )
    .await
    .unwrap();
    // IPv4 and IPv6: one forward and one ICMP rule each.
    assert_eq!(backend.ledger.rules.len(), 4);
    let source: std::net::IpAddr = "fd79::5".parse().unwrap();
    backend
        .routing
        .source(source, remote, &mut backend.ledger, &mut backend.store)
        .await
        .unwrap();
    backend
        .routing
        .forwards(&backend.ledger, source, remote)
        .await
        .unwrap();
    assert_eq!(backend.ledger.routes.len(), 1);
    kernel.quiesce().await.unwrap();
    backend
        .routing
        .remove(&mut backend.ledger, &mut backend.store, true)
        .await
        .unwrap();
    assert!(backend.ledger.rules.is_empty() && backend.ledger.routes.is_empty());
    let mut commands = command::Commands::default();
    objects::mutate(&mut commands, &serde_json::json!({"nftables": backend.ledger.program.tables().into_iter().map(|(family, name)| serde_json::json!({"delete": {"table": {"family": family, "name": name}}})).collect::<Vec<_>>()})).await.unwrap();
    kernel.quiesce().await.unwrap();
    drop(tasks);
}
