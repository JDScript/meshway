//! Executed by tests/underlay_fixture/run.py inside disposable namespaces only.
use std::{io, net::SocketAddr, sync::Arc, time::Duration};

use tokio::io::AsyncWriteExt;

use super::{
    dns::{BootstrapResolver, SystemDnsSnapshot},
    http::ControllerHttp,
    *,
};
use crate::generated::{
    client::{ApiOpError, HttpClient, HttpTransport},
    types::EnrollAgentRequest,
};

fn reservation(generation: u64, foreign_bits: u32) -> Reservation {
    Reservation {
        owner: "isolated-rust-fixture".into(),
        generation,
        mark: MarkReservation::new(0x4d570000, 0xffff0000).unwrap(),
        foreign_bits,
        first_priority: 18000,
        last_priority: 18003,
        capture_tables: vec![18401, 18402],
    }
}

fn resolver(factory: &SocketFactory, version: u8) -> BootstrapResolver {
    let address = if version == 4 {
        "172.30.247.20:41864"
    } else {
        "[fd74:6d77:247::20]:41864"
    };
    BootstrapResolver::new(
        factory.clone(),
        SystemDnsSnapshot::plain_servers(vec![address.parse().unwrap()]).unwrap(),
    )
}

fn transport(
    factory: &SocketFactory,
    origin: &str,
    version: u8,
    trust: bool,
) -> Arc<ControllerHttp> {
    let mut roots = rustls::RootCertStore::empty();
    if trust {
        let contents = std::fs::read("/fixture-root.crt").unwrap();
        for certificate in rustls_pemfile::certs(&mut contents.as_slice()) {
            roots.add(certificate.unwrap()).unwrap();
        }
    }
    Arc::new(
        ControllerHttp::new(
            factory.clone(),
            resolver(factory, version),
            origin.parse().unwrap(),
            roots,
        )
        .unwrap(),
    )
}

async fn request(
    client: &ControllerHttp,
    url: &str,
) -> crate::generated::client::HttpResult<reqwest::Response> {
    client
        .execute(reqwest::Request::new(
            reqwest::Method::GET,
            url.parse().unwrap(),
        ))
        .await
}

#[tokio::test]
#[ignore = "requires isolated Docker RPDB/default capture and observer"]
async fn production_paths() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_FIXTURE").as_deref(),
        Ok("1")
    );
    let generation = PreparedGeneration::prepare(reservation(1, 0))
        .await
        .unwrap();
    let factory = generation.factory();
    for version in [4, 6] {
        let host = format!("v{version}.underlay.fixture");
        for (scheme, port) in [("http", 41861), ("https", 41862)] {
            let origin = format!("{scheme}://{host}:{port}");
            let client = transport(&factory, &origin, version, true);
            let response = request(&client, &format!("{origin}/actual")).await.unwrap();
            assert!(response.status().is_success());
            assert!(response.text().await.unwrap().contains("source"));
            // A second real request: no hidden pool or bypass connection path.
            assert!(
                request(&client, &format!("{origin}/actual"))
                    .await
                    .unwrap()
                    .text()
                    .await
                    .is_ok()
            );
            let generated = HttpClient::new()
                .with_base_url(&origin)
                .with_api_key("fixture-only")
                .with_transport(client.clone());
            let failure = generated
                .enroll_agent(EnrollAgentRequest {
                    agent_version: "fixture".into(),
                    name: "isolated".into(),
                    node_credential: "a".repeat(64),
                })
                .await
                .unwrap_err();
            match failure {
                ApiOpError::Api(error) => {
                    assert_eq!(error.status, 401);
                    assert!(error.raw_body.is_empty());
                }
                error => panic!("generated transport did not preserve status-first: {error:?}"),
            }
            if scheme == "https" {
                let bad = transport(&factory, &origin, version, false);
                assert!(request(&bad, &format!("{origin}/actual")).await.is_err());
            }
        }
        // Both outer UDP and TCP fallback for a truncated DNS response.
        let bootstrap = resolver(&factory, version);
        assert_eq!(
            bootstrap
                .resolve(Purpose::Carrier, "tcp.underlay.fixture")
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            bootstrap
                .resolve(Purpose::Carrier, "tcp.underlay.fixture")
                .await
                .unwrap()
                .len(),
            2
        );
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert_eq!(
            bootstrap
                .resolve(Purpose::Carrier, "tcp.underlay.fixture")
                .await
                .unwrap()
                .len(),
            2
        );
        let bind: SocketAddr = if version == 4 { "0.0.0.0:0" } else { "[::]:0" }
            .parse()
            .unwrap();
        let remote: SocketAddr = if version == 4 {
            "172.30.247.20:41865"
        } else {
            "[fd74:6d77:247::20]:41865"
        }
        .parse()
        .unwrap();
        let udp = factory.udp(Purpose::AuthenticationProbe, bind).unwrap();
        udp.connect(remote).await.unwrap();
        udp.send(b"real-outer-udp").await.unwrap();
        let mut bytes = [0; 64];
        assert_eq!(udp.recv_from(&mut bytes).await.unwrap().0, 14);

        // Server's unmarked dial triggers listener SYN-ACK, then verified accept.
        let listener = factory.listener(Purpose::Carrier, bind).unwrap();
        let local = listener.local_addr().unwrap();
        let source = if version == 4 {
            "172.30.247.10"
        } else {
            "fd74:6d77:247::10"
        };
        let origin = format!("http://v{version}.underlay.fixture:41861");
        let client = transport(&factory, &origin, version, true);
        let command = format!("{origin}/dial?address={source}&port={}", local.port());
        let accept_and_reply = async {
            let (mut accepted, _) = listener.accept().await.unwrap();
            accepted.write_all(b"inherited-mark").await.unwrap();
        };
        let ((), commanded) = tokio::join!(accept_and_reply, request(&client, &command));
        commanded.unwrap();
    }

    // Preserve external low bits and actually traverse the original VPN table.
    let vpn = PreparedGeneration::prepare(reservation(2, 0x20))
        .await
        .unwrap();
    let vpn_factory = vpn.factory();
    for (version, expected) in [(4, "172.30.247.11"), (6, "fd74:6d77:247::11")] {
        let origin = format!("http://v{version}.underlay.fixture:41861");
        let client = transport(&vpn_factory, &origin, version, true);
        let text = request(&client, &format!("{origin}/actual"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(text.contains(expected), "wrong VPN source: {text}");
    }
    vpn.retire();
    assert!(
        vpn_factory
            .tcp(Purpose::Controller, "172.30.247.20:41861".parse().unwrap())
            .await
            .is_err()
    );

    let origin = "http://v4.underlay.fixture:41861";
    let client = transport(&factory, origin, 4, true);
    let response = request(&client, &format!("{origin}/slow")).await.unwrap();
    let mut old_tcp = factory
        .tcp(Purpose::Carrier, "172.30.247.20:41861".parse().unwrap())
        .await
        .unwrap();
    let old_udp = factory
        .udp(Purpose::Stun, "0.0.0.0:0".parse().unwrap())
        .unwrap();
    // Keep listener wrappers alive across retirement: merely dropping an owner
    // handle is insufficient if the kernel can still emit new SYN-ACKs.
    let old_listeners = [
        factory
            .listener(Purpose::Carrier, "127.0.0.1:0".parse().unwrap())
            .unwrap(),
        factory
            .listener(Purpose::Carrier, "[::1]:0".parse().unwrap())
            .unwrap(),
    ];
    let stalled = resolver(&factory, 4);
    let waiting_dns = stalled.resolve(Purpose::Controller, "silent.underlay.fixture");
    let headers_url = format!("{origin}/headers");
    let waiting_http = request(&client, &headers_url);
    tokio::pin!(waiting_dns, waiting_http);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut waiting_dns)
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut waiting_http)
            .await
            .is_err()
    );
    generation.retire();
    for listener in &old_listeners {
        let address = listener.local_addr().unwrap();
        let connect = tokio::time::timeout(
            Duration::from_secs(1),
            tokio::net::TcpStream::connect(address),
        )
        .await
        .unwrap();
        assert_eq!(
            connect.unwrap_err().kind(),
            io::ErrorKind::ConnectionRefused
        );
        assert!(listener.accept().await.is_err());
    }
    assert!(response.bytes().await.is_err());
    assert!(old_tcp.write_all(b"must-not-send").await.is_err());
    assert!(
        old_udp
            .send_to(b"must-not-send", "172.30.247.20:41865".parse().unwrap())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(1), waiting_dns)
            .await
            .unwrap()
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(1), waiting_http)
            .await
            .unwrap()
            .is_err()
    );

    let new_generation = PreparedGeneration::prepare(reservation(3, 0))
        .await
        .unwrap();
    let new_factory = new_generation.factory();
    let new_client = transport(&new_factory, origin, 4, true);
    assert!(
        request(&new_client, &format!("{origin}/actual"))
            .await
            .unwrap()
            .bytes()
            .await
            .is_ok()
    );
    // No ambient unmarked getaddrinfo fallback on NXDOMAIN/upstream failure.
    assert!(
        resolver(&new_factory, 4)
            .resolve(Purpose::Controller, "missing.underlay.fixture")
            .await
            .is_err()
    );
    assert!(
        resolver(&new_factory, 4)
            .resolve(Purpose::Controller, "silent.underlay.fixture")
            .await
            .is_err()
    );

    // Same destinations in the same process must still hit the real sink.
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
            let socket = tokio::net::UdpSocket::bind(SocketAddr::new(
                if ip.is_ipv4() {
                    [0, 0, 0, 0].into()
                } else {
                    [0u16; 8].into()
                },
                0,
            ))
            .await
            .unwrap();
            socket
                .send_to(b"ordinary-captured", SocketAddr::new(ip, port))
                .await
                .unwrap();
            let mut bytes = [0; 64];
            assert!(
                tokio::time::timeout(Duration::from_millis(100), socket.recv_from(&mut bytes))
                    .await
                    .is_err()
            );
        }
    }
    println!("production socket/HTTP/TLS/DNS/generation paths verified");
}

#[tokio::test]
#[ignore = "runner drops capabilities and UID in an isolated namespace"]
async fn permission_denied() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_FIXTURE").as_deref(),
        Ok("1")
    );
    let error = match PreparedGeneration::prepare(reservation(99, 0)).await {
        Ok(_) => panic!("SO_MARK unexpectedly permitted after dropping capabilities"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}

#[tokio::test]
#[ignore = "isolated namespace installs an aliased external reserve-table rule"]
async fn alias_capture_reference_rejected() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_FIXTURE").as_deref(),
        Ok("1")
    );
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
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_FIXTURE").as_deref(),
        Ok("1")
    );
    for version in [4, 6] {
        let generation = PreparedGeneration::prepare(reservation(200 + version, 0))
            .await
            .unwrap();
        let factory = generation.factory();
        let host: std::net::IpAddr = if version == 4 {
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
        assert_eq!(
            healthy
                .resolve(Purpose::Controller, "backup.underlay.fixture")
                .await
                .unwrap()
                .len(),
            2
        );
        // A truncated answer at the configured backup uses protected TCP too.
        assert_eq!(
            healthy
                .resolve(Purpose::Controller, "tcpbackup.underlay.fixture")
                .await
                .unwrap()
                .len(),
            2
        );
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
