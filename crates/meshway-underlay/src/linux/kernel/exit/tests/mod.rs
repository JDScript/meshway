use super::io::ExitCutoff as Deadline;
use super::*;
use crate::linux::{
    kernel::{Kernel, journal::Journal},
    tasks::Tasks,
};
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
    let (boot, namespace_device, namespace_inode) = Journal::identity().unwrap();
    let (store, previous) = storage::Store::open(kernel.store.directory())
        .await
        .unwrap();
    assert!(previous.is_none());
    let ledger = storage::Ledger {
        version: 2,
        boot,
        namespace_device,
        namespace_inode,
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
    println!("X3_NFT_PROGRAM={}", ledger.program.create());
    let mut backend = backend::Backend::create(
        store,
        ledger,
        routing,
        Arc::new(RwLock::new(BTreeMap::new())),
        vec![external_source.parse().unwrap(), Ipv4Addr::LOCALHOST],
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
        destination: *remote.ip(),
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
    assert_eq!(&reply[16..20], &tuple.source.octets());
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
        conntrack::lookup_owned(&mut backend.commands, tuple, mark)
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
    assert!(
        backend
            .install(id(2), tuple, cutoff(30_000), scope.clone())
            .await
            .is_err()
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
    let mut commands = command::Commands::default();
    assert!(
        conntrack::lookup_owned(&mut commands, crash, mark)
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
        conntrack::lookup_owned(&mut commands, record.tuple, prior.program.mark)
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
        conntrack::lookup_owned(&mut commands, record.tuple, prior.program.mark)
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
    packet[12..16].copy_from_slice(&tuple.source.octets());
    packet[16..20].copy_from_slice(&tuple.destination.octets());
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
