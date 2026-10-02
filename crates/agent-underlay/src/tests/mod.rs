//! Private reservation reproductions in disposable kernel namespaces.
use super::{
    generation::{MarkReservation, Reservation},
    *,
};
use std::{io, net::SocketAddr, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn reservation(generation: u64, foreign_bits: u32) -> Reservation {
    Reservation {
        owner: "isolated-rust-fixture".into(),
        generation,
        mark: MarkReservation::new(0x4d570000, 0xffff0000).unwrap(),
        foreign_bits,
        first_priority: 18000,
        last_priority: 18003,
        capture_tables: vec![18401, 18402],
        mesh_sink_table: None,
    }
}

fn fixture() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_FIXTURE").as_deref(),
        Ok("1")
    );
}

#[tokio::test]
#[ignore = "runner drops capabilities and UID in an isolated namespace"]
async fn permission_denied() {
    fixture();
    let error = match PreparedGeneration::prepare(reservation(99, 0)).await {
        Ok(_) => panic!("SO_MARK unexpectedly permitted after dropping capabilities"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}

#[tokio::test]
#[ignore = "isolated namespace installs an aliased external reserve-table rule"]
async fn alias_capture_reference_rejected() {
    fixture();
    let error = match PreparedGeneration::prepare(reservation(100, 0)).await {
        Ok(_) => panic!("aliased external reserve-table reference was admitted"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("owned capture table referenced outside guarded block"),
        "{error}"
    );
}

#[tokio::test]
#[ignore = "isolated namespace provides SERVFAIL primary and configured DNS alternatives"]
async fn dns_configured_backup_paths() {
    fixture();
    for version in [4, 6] {
        let generation = PreparedGeneration::prepare(reservation(200 + version, 0))
            .await
            .unwrap();
        let factory = generation.factory();
        let host = if version == 4 {
            "172.30.247.20"
        } else {
            "fd74:6d77:247::20"
        }
        .parse()
        .unwrap();
        let make = |backup_port| {
            BootstrapResolver::new(
                factory.clone(),
                SystemDnsSnapshot::plain_servers(vec![
                    SocketAddr::new(host, 41866),
                    SocketAddr::new(host, backup_port),
                ])
                .unwrap(),
            )
        };
        let healthy = make(41864);
        for name in ["backup.underlay.fixture", "tcpbackup.underlay.fixture"] {
            assert_eq!(
                healthy
                    .resolve(Purpose::Controller, name)
                    .await
                    .unwrap()
                    .len(),
                2
            );
        }
        let failing = make(41867);
        let error = failing
            .resolve(Purpose::Controller, "allfail.underlay.fixture")
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "bootstrap DNS response: {}",
                hickory_proto::op::ResponseCode::ServFail
            )
        );
        for (name, code) in [
            (
                "nxdomainbackup.underlay.fixture",
                hickory_proto::op::ResponseCode::NXDomain,
            ),
            (
                "refused.underlay.fixture",
                hickory_proto::op::ResponseCode::Refused,
            ),
        ] {
            let error = healthy
                .resolve(Purpose::Controller, name)
                .await
                .unwrap_err();
            assert_eq!(error.to_string(), format!("bootstrap DNS response: {code}"));
        }
        let waiting = healthy.resolve(Purpose::Controller, "cancelbackup.underlay.fixture");
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut waiting)
                .await
                .is_err()
        );
        generation.retire();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), waiting)
                .await
                .unwrap()
                .unwrap_err()
                .kind(),
            io::ErrorKind::ConnectionAborted
        );
    }
    println!(
        "UR2 protected same-scope DNS fallback, terminal responses and generation cancel verified"
    );
}

#[tokio::test]
#[ignore = "real external low bits and original VPN table in both IP families"]
async fn foreign_mark_paths() {
    fixture();
    let generation = PreparedGeneration::prepare(reservation(2, 0x20))
        .await
        .unwrap();
    let factory = generation.factory();
    for (host, expected) in [
        ("172.30.247.20", "172.30.247.11"),
        ("fd74:6d77:247::20", "fd74:6d77:247::11"),
    ] {
        let remote = SocketAddr::new(host.parse().unwrap(), 41861);
        let mut socket = factory.tcp(Purpose::Controller, remote).await.unwrap();
        socket
            .write_all(b"GET /actual HTTP/1.1\r\nHost: fixture\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut bytes = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), socket.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains(expected), "wrong VPN source: {text}");
    }
    generation.retire();
    assert!(
        factory
            .tcp(Purpose::Controller, "172.30.247.20:41861".parse().unwrap())
            .await
            .is_err()
    );
    // Same destinations from ordinary sockets must traverse the real fixture TUN.
    for host in ["172.30.247.20", "fd74:6d77:247::20"] {
        let ip: std::net::IpAddr = host.parse().unwrap();
        for port in [41861, 41862, 41864] {
            assert!(
                tokio::time::timeout(
                    Duration::from_millis(300),
                    tokio::net::TcpStream::connect(SocketAddr::new(ip, port))
                )
                .await
                .is_err()
            );
        }
        for port in [41864, 41865] {
            let bind = SocketAddr::new(
                if ip.is_ipv4() {
                    [0, 0, 0, 0].into()
                } else {
                    [0u16; 8].into()
                },
                0,
            );
            let socket = tokio::net::UdpSocket::bind(bind).await.unwrap();
            socket
                .send_to(b"ordinary-captured", SocketAddr::new(ip, port))
                .await
                .unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(100), socket.recv_from(&mut [0; 64]))
                    .await
                    .is_err()
            );
        }
    }
}
