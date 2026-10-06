//! Actual protected descriptors in an explicitly disposable Linux namespace.
use super::*;
use meshway_configuration::{
    Input,
    execution::{BusinessProtocol, BusinessTarget},
};
use meshway_private_storage::Directory;
use meshway_underlay::Host;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn inode(network: &str, local: SocketAddr, peer: Option<SocketAddr>) -> String {
    // Only this test process's own network namespace and descriptors are read.
    let text = std::fs::read_to_string(format!("/proc/self/net/{network}")).unwrap();
    let inode = text
        .lines()
        .skip(1)
        .find_map(|line| {
            let columns: Vec<_> = line.split_whitespace().collect();
            let port =
                |value: &str| u16::from_str_radix(value.rsplit(':').next().unwrap(), 16).unwrap();
            (port(columns[1]) == local.port()
                && port(columns[2]) == peer.map_or(0, |peer| peer.port()))
            .then(|| format!("socket:[{}]", columns[9]))
        })
        .expect("actual owned socket in this network namespace");
    assert!(
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| {
                std::fs::read_link(entry.path())
                    .is_ok_and(|target| target.to_string_lossy() == inode)
            }),
        "socket must belong to this test PID"
    );
    inode
}

async fn fixture(protocol: TransportProtocol) -> support::Fixture {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let mut fixture = resources::setup(protocol).await;
    let ids = fixture.ids();
    let instance = fixture.controls[0].execution_snapshot().unwrap().instance();
    fixture.controls[0]
        .create_demand(&meshway_protocol::DemandRequest {
            instance,
            request_id: support::request(),
            target: BusinessTarget::Mesh { node: ids[2] },
            protocol: BusinessProtocol::Udp,
            port: Some(42424.try_into().unwrap()),
            profile: "mesh".into(),
            duration_seconds: 120,
        })
        .await
        .unwrap();
    for index in 0..3 {
        fixture.refresh(index).await;
    }
    let cfg = fixture.identities[2].snapshot().configuration().unwrap();
    let local = cfg
        .listeners
        .values()
        .find_map(|listener| match listener.declaration() {
            Listener::NativeTcp(value) => {
                Some(SocketAddr::new(value.listen, value.listen_port.get()))
            }
            Listener::NativeUdp(value) => {
                Some(SocketAddr::new(value.listen, value.listen_port.get()))
            }
            _ => None,
        })
        .unwrap();
    fixture
        .extra_tcp
        .retain(|socket| socket.local_addr().unwrap() != local);
    fixture
        .extra_udp
        .retain(|socket| socket.local_addr().unwrap() != local);
    fixture
}

async fn host(fixture: &support::Fixture, directory: &std::path::Path) -> Host {
    let directory = Arc::new(Directory::open(directory, "test.lock").unwrap());
    let mut host = Host::open(
        directory,
        "https://127.0.0.1:8443",
        meshway_underlay::Options {
            bootstrap_nameservers: Some(vec!["192.0.2.53:53".parse().unwrap()]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    host.install(fixture.identities[2].snapshot().clone())
        .await
        .unwrap();
    host
}

async fn remove_b(fixture: &mut support::Fixture, host: &mut Host) {
    let ids = fixture.ids();
    fixture
        .configure(|cfg| cfg.access = vec![resources::direct_rule(&ids)])
        .await;
    let old = fixture.identities[2].snapshot().clone();
    let next = fixture.refresh(2).await;
    let previous = next.snapshot().continuation_from(&old).unwrap();
    assert_ne!(
        previous.dialers,
        next.snapshot().configuration().unwrap().dialers
    );
    host.install(next.snapshot().clone()).await.unwrap();
    assert!(
        old.ensure_current().is_err(),
        "physical retention never revives old authority"
    );
}

async fn restore_b(fixture: &mut support::Fixture, host: &mut Host) {
    let ids = fixture.ids();
    fixture
        .configure(|cfg| {
            cfg.access.push(meshway_configuration::AccessRule {
                remark: String::new(),
                src: vec![format!("node:{}", ids[2])],
                dst: vec![format!("node:{}", ids[1])],
                ip: vec!["udp:42424".into()],
                bidirectional: false,
            })
        })
        .await;
    let next = fixture.refresh(2).await;
    host.install(next.snapshot().clone()).await.unwrap();
}

async fn bind_change(fixture: &mut support::Fixture, host: &mut Host, stream: bool) {
    let id = fixture.ids()[2];
    fixture
        .configure(|cfg| {
            let mut dialers = cfg.defaults.dialers.clone();
            if stream {
                dialers.native_tcp.as_mut().unwrap().local_address = "127.0.0.1".into();
            } else {
                dialers.native_udp.as_mut().unwrap().local_address = "127.0.0.1".into();
            }
            cfg.nodes.get_mut(&id).unwrap().dialers = Input::Value(dialers);
        })
        .await;
    host.install(fixture.refresh(2).await.snapshot().clone())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires an isolated Linux network namespace, private /run and NET_ADMIN"]
async fn actual_host_udp_resource_tokens_keep_unrelated_fd_and_never_revive_retired_fd() {
    let mut fixture = fixture(TransportProtocol::NativeUdp).await;
    let ids = fixture.ids();
    let temp = tempfile::tempdir().unwrap();
    let mut host = host(&fixture, &temp.path().join("underlay")).await;
    let factory = host.sockets();
    let old = fixture.identities[2].snapshot().clone();
    let a = format!("{}/udp", ids[0]);
    let b = format!("{}/udp", ids[1]);
    let listener = factory.datagram_listener(&old, "udp").await.unwrap();
    let retained = factory
        .datagram_candidate(&old, &a, 0)
        .await
        .unwrap()
        .remove(0);
    let retired = factory
        .datagram_candidate(&old, &b, 0)
        .await
        .unwrap()
        .remove(0);
    let listener_address = listener.local_addr().unwrap();
    let local = retained.endpoint.local_addr().unwrap();
    let listener_inode = inode("udp", listener_address, None);
    let original_inode = inode("udp", local, None);
    remove_b(&mut fixture, &mut host).await;
    // Negative control with the old whole-map Host branch fails right here:
    // the original descriptor's token has already been permanently cancelled.
    assert_eq!(listener.local_addr().unwrap(), listener_address);
    assert_eq!(retained.endpoint.local_addr().unwrap(), local);
    assert_eq!(inode("udp", listener_address, None), listener_inode);
    assert_eq!(inode("udp", local, None), original_inode);
    assert!(retired.endpoint.local_addr().is_err());
    assert!(factory.datagram_candidate(&old, &a, 0).await.is_err());
    let bytes = b"fresh owner on the original protected UDP fd";
    let target = retained.targets[0];
    futures_util::future::poll_fn(|cx| retained.endpoint.poll_send_to(cx, bytes, target))
        .await
        .unwrap();
    let socket = fixture
        .extra_udp
        .iter()
        .find(|socket| socket.local_addr().unwrap() == target)
        .unwrap();
    let mut received = [0; 128];
    let (len, _) = tokio::time::timeout(Duration::from_secs(2), socket.recv_from(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&received[..len], bytes);
    restore_b(&mut fixture, &mut host).await;
    assert!(retired.endpoint.local_addr().is_err());
    let new_b = factory
        .datagram_candidate(fixture.identities[2].snapshot(), &b, 0)
        .await
        .unwrap();
    assert!(!new_b.is_empty());
    assert_eq!(inode("udp", local, None), original_inode);
    bind_change(&mut fixture, &mut host, false).await;
    assert!(retained.endpoint.local_addr().is_err());
    assert_eq!(inode("udp", listener_address, None), listener_inode);
    let rollback = fixture.ids()[2];
    fixture
        .configure(|cfg| cfg.nodes.get_mut(&rollback).unwrap().dialers = Input::Omitted)
        .await;
    host.install(fixture.refresh(2).await.snapshot().clone())
        .await
        .unwrap();
    assert!(retained.endpoint.local_addr().is_err());
    assert!(retired.endpoint.local_addr().is_err());
    assert_eq!(listener.local_addr().unwrap(), listener_address);
    eprintln!(
        "FD-PROOF udp retained={original_inode} listener={listener_inode}; remove/add/bind/rollback old guards rejected"
    );
    host.shutdown(false).await.unwrap();
    assert!(listener.local_addr().is_err());
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires an isolated Linux network namespace, private /run and NET_ADMIN"]
async fn actual_host_tcp_resource_tokens_keep_unrelated_fd_and_never_revive_retired_fd() {
    let mut fixture = fixture(TransportProtocol::NativeTcp).await;
    let ids = fixture.ids();
    let temp = tempfile::tempdir().unwrap();
    let mut host = host(&fixture, &temp.path().join("underlay")).await;
    let factory = host.sockets();
    let old = fixture.identities[2].snapshot().clone();
    let a = format!("{}/tcp", ids[0]);
    let b = format!("{}/tcp", ids[1]);
    let listener = factory
        .listen(&factory.listener(&old, "tcp").unwrap())
        .await
        .unwrap();
    let frozen = factory.candidate(&old, &a, 0).unwrap();
    let mut retained = factory.connect(&frozen).await.unwrap();
    let retired = factory
        .connect(&factory.candidate(&old, &b, 0).unwrap())
        .await
        .unwrap();
    let local = retained.local_addr().unwrap();
    let target = retained.peer_addr().unwrap();
    let (mut receiving, _) = fixture
        .extra_tcp
        .iter()
        .find(|listener| listener.local_addr().unwrap() == target)
        .unwrap()
        .accept()
        .await
        .unwrap();
    let listener_address = listener.local_addr().unwrap();
    let original_inode = inode("tcp", local, Some(target));
    let listener_inode = inode("tcp", listener_address, None);
    // Accepted streams carry the listener's exact resource purpose too.
    let incoming = TcpStream::connect(listener_address).await.unwrap();
    let (accepted, _) = listener.accept().await.unwrap();
    let accepted_address = accepted.local_addr().unwrap();
    remove_b(&mut fixture, &mut host).await;
    assert_eq!(retained.local_addr().unwrap(), local);
    assert_eq!(inode("tcp", local, Some(target)), original_inode);
    assert_eq!(inode("tcp", listener_address, None), listener_inode);
    assert_eq!(accepted.local_addr().unwrap(), accepted_address);
    assert!(retired.local_addr().is_err());
    assert!(factory.connect(&frozen).await.is_err());
    retained.write_all(b"retained").await.unwrap();
    let mut bytes = [0; 8];
    tokio::time::timeout(Duration::from_secs(2), receiving.read_exact(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes, b"retained");
    restore_b(&mut fixture, &mut host).await;
    assert!(retired.local_addr().is_err());
    assert_eq!(inode("tcp", local, Some(target)), original_inode);
    let new_b = factory
        .connect(
            &factory
                .candidate(fixture.identities[2].snapshot(), &b, 0)
                .unwrap(),
        )
        .await
        .unwrap();
    bind_change(&mut fixture, &mut host, true).await;
    assert!(retained.local_addr().is_err());
    assert!(new_b.local_addr().is_err());
    assert_eq!(listener.local_addr().unwrap(), listener_address);
    let relay = ids[2];
    fixture
        .configure(|cfg| {
            let Listener::NativeTcp(listener) = cfg
                .nodes
                .get_mut(&relay)
                .unwrap()
                .listeners
                .get_mut("tcp")
                .unwrap()
            else {
                panic!("TCP listener")
            };
            listener.socket.receive_buffer = 65_536;
        })
        .await;
    host.install(fixture.refresh(2).await.snapshot().clone())
        .await
        .unwrap();
    assert!(listener.local_addr().is_err());
    assert!(accepted.local_addr().is_err());
    drop(incoming);
    eprintln!(
        "FD-PROOF tcp retained={original_inode} listener={listener_inode}; original frozen handle/removed/bind/listener options rejected"
    );
    host.shutdown(false).await.unwrap();
    fixture.close().await;
}
