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
        mesh_sink_table: None,
    }
}

fn native_config(version: u8, phase: usize, server: bool) -> meshway_dataplane::LabConfig {
    use meshway_dataplane::*;
    let identity = Identity::from_private_key(
        PrivateKey::from_hex(&if server { "11" } else { "22" }.repeat(32)).unwrap(),
    );
    let other = Identity::from_private_key(
        PrivateKey::from_hex(&if server { "22" } else { "11" }.repeat(32)).unwrap(),
    );
    let id = if server { "server" } else { "client" };
    let peer = if server { "client" } else { "server" };
    let transport = if phase < 2 {
        TransportKind::NativeUdp
    } else {
        TransportKind::NativeTcp
    };
    let listen = server == phase.is_multiple_of(2);
    let ip = match (version, listen, server) {
        (4, true, true) | (4, false, false) => "172.30.247.20",
        (4, _, _) => "172.30.247.10",
        (6, true, true) | (6, false, false) => "fd74:6d77:247::20",
        (6, _, _) => "fd74:6d77:247::10",
        _ => unreachable!(),
    }
    .parse()
    .unwrap();
    let endpoint = SocketAddr::new(ip, 41871 + phase as u16 + if version == 6 { 4 } else { 0 });
    LabConfig {
        node_id: id.into(),
        private_key: PrivateKey::from_hex(&identity.private_key_hex()).unwrap(),
        public_key: identity.public_key_hex(),
        trusted_peers: vec![PeerConfig {
            node_id: peer.into(),
            public_key: other.public_key_hex(),
            transports: vec![transport],
        }],
        allowed_paths: vec![
            vec!["client".into(), "server".into()],
            vec!["server".into(), "client".into()],
        ],
        listeners: if listen {
            vec![ListenerConfig {
                transport,
                bind: endpoint,
                id: Some("native-listener".into()),
                allowed_peers: Some(vec![peer.into()]),
            }]
        } else {
            vec![]
        },
        outgoing: if listen {
            vec![]
        } else {
            vec![OutgoingCandidate {
                peer: peer.into(),
                transport,
                endpoint: endpoint.to_string(),
                priority: 0,
                id: Some("native-candidate".into()),
            }]
        },
        limits: Limits {
            reconnect_delay_ms: 100,
            handshake_timeout_ms: 1000,
            ..Default::default()
        },
    }
}

#[tokio::test]
#[ignore = "disposable Docker namespace native peer only"]
async fn native_peer() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_FIXTURE").as_deref(),
        Ok("1")
    );
    for version in [4, 6] {
        for phase in 0..4 {
            let runtime = meshway_dataplane::Runtime::start(native_config(version, phase, true))
                .await
                .unwrap();
            runtime
                .wait_for_peer("client", Duration::from_secs(60))
                .await
                .unwrap();
            // Explicit fixture coordination, independent of brief disconnects
            // while the client replaces its protected generation.
            let done = format!("/run/native-complete-{version}-{phase}");
            tokio::time::timeout(Duration::from_secs(30), async {
                while !std::path::Path::new(&done).exists() {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
            .unwrap();
            runtime.shutdown().await;
        }
    }
}

#[tokio::test]
#[ignore = "requires disposable Docker native peer plus dual-stack capture"]
async fn native_runtime_paths() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_FIXTURE").as_deref(),
        Ok("1")
    );
    for version in [4, 6] {
        for phase in 0..4 {
            let generation = PreparedGeneration::prepare(reservation(100 + phase as u64, 0))
                .await
                .unwrap();
            let factory = generation.factory();
            let upstream = if version == 4 {
                "172.30.247.20:41864"
            } else {
                "[fd74:6d77:247::20]:41864"
            }
            .parse()
            .unwrap();
            let protection = Arc::new(ManagedProtection::new(
                factory,
                SystemDnsSnapshot::plain_servers(vec![upstream]).unwrap(),
            ));
            let mut config = native_config(version, phase, false);
            // Same protected bootstrap seam used by Manager::resolve_targets;
            // Runtime receives only the resulting numeric candidate.
            for candidate in &mut config.outgoing {
                let endpoint: SocketAddr = candidate.endpoint.parse().unwrap();
                let targets = crate::managed_connections::sockets::Protection::resolve(
                    protection.as_ref(),
                    &format!("v{version}.underlay.fixture"),
                    endpoint.port(),
                )
                .await
                .unwrap();
                assert_eq!(targets, vec![endpoint]);
                candidate.endpoint = targets[0].to_string();
            }
            let runtime =
                meshway_dataplane::Runtime::start_with_socket_factory(config, protection.clone())
                    .await
                    .unwrap();
            runtime
                .wait_for_peer("server", Duration::from_secs(15))
                .await
                .unwrap();
            if phase < 2 {
                let socket = runtime.udp_sockets().remove(0);
                let observer: SocketAddr = if version == 4 {
                    "172.30.247.20:41865"
                } else {
                    "[fd74:6d77:247::20]:41865"
                }
                .parse()
                .unwrap();
                let observation = runtime
                    .discover_udp(socket.socket_id, observer, Duration::from_millis(1000))
                    .await
                    .unwrap();
                assert_eq!(
                    observation.outcome,
                    meshway_dataplane::UdpDiscoveryOutcome::Mapped
                );
                let mapped = observation.mapped_endpoint.unwrap();
                assert_eq!(mapped.port(), socket.local_addr.port());
                if socket.local_addr.ip().is_unspecified() {
                    // The existing fixture intentionally has two local source
                    // addresses to prove original VPN/RPDB behavior. An unbound
                    // dial socket follows the kernel's real source selection.
                    let allowed: Vec<std::net::IpAddr> = if version == 4 {
                        vec![
                            "172.30.247.10".parse().unwrap(),
                            "172.30.247.11".parse().unwrap(),
                        ]
                    } else {
                        vec![
                            "fd74:6d77:247::10".parse().unwrap(),
                            "fd74:6d77:247::11".parse().unwrap(),
                        ]
                    };
                    assert!(allowed.contains(&mapped.ip()));
                } else {
                    assert_eq!(mapped.ip(), socket.local_addr.ip());
                }
            }
            assert_eq!(
                runtime
                    .probe(
                        vec!["client".into(), "server".into()],
                        b"native-protected".to_vec()
                    )
                    .await
                    .unwrap()
                    .payload,
                b"native-protected"
            );
            generation.retire();
            // Rebinding with a retired capability must fail closed. The same
            // Runtime can then recover with a genuinely new prepared owner.
            assert!(
                runtime
                    .replace_native_socket_factory(protection.clone())
                    .await
                    .is_err()
            );
            let next = PreparedGeneration::prepare(reservation(200 + phase as u64, 0))
                .await
                .unwrap();
            let protection = Arc::new(ManagedProtection::new(
                next.factory(),
                SystemDnsSnapshot::plain_servers(vec![upstream]).unwrap(),
            ));
            runtime
                .replace_native_socket_factory(protection)
                .await
                .unwrap();
            runtime
                .wait_for_peer("server", Duration::from_secs(15))
                .await
                .unwrap();
            assert_eq!(
                runtime
                    .probe(
                        vec!["client".into(), "server".into()],
                        b"new-native-generation".to_vec()
                    )
                    .await
                    .unwrap()
                    .payload,
                b"new-native-generation"
            );
            runtime.shutdown().await;
            let origin = format!("http://v{version}.underlay.fixture:41861");
            let command = transport(&next.factory(), &origin, version, true);
            request(
                &command,
                &format!("{origin}/native-done?version={version}&phase={phase}"),
            )
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }
}

#[tokio::test]
#[ignore = "requires disposable Docker prepared generation"]
async fn managed_native_owner_lifecycle() {
    use crate::managed_connections::{
        Manager,
        tests::{fixture_at_port, snapshot},
    };
    use meshway_protocol::{ConnectionRuntimeState, ConnectionTransport};
    for transport in [
        ConnectionTransport::NativeUdp,
        ConnectionTransport::NativeTcp,
    ] {
        // Fixed, fixture-only ports in this isolated namespace. Do not use the
        // host test allocator, whose availability probe opens ordinary sockets.
        let port = if transport == ConnectionTransport::NativeUdp {
            26001
        } else {
            26002
        };
        let (server, key, _, _) = fixture_at_port(transport, port);
        let generation = PreparedGeneration::prepare(reservation(300, 0))
            .await
            .unwrap();
        let old = generation.factory();
        let upstreams =
            SystemDnsSnapshot::plain_servers(vec!["172.30.247.20:41864".parse().unwrap()]).unwrap();
        let mut manager = Manager::new(None, Duration::from_secs(30));
        manager
            .install_prepared_underlay(generation, upstreams)
            .await;
        manager
            .reconcile(
                snapshot(server.clone()),
                key.clone(),
                tokio::time::Instant::now(),
            )
            .await;
        assert_eq!(manager.observation().state, ConnectionRuntimeState::Applied);
        old.0.retire();
        tokio::time::timeout(Duration::from_secs(2), async {
            while manager.observation().state == ConnectionRuntimeState::Applied {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let next = PreparedGeneration::prepare(reservation(301, 0))
            .await
            .unwrap();
        manager
            .install_prepared_underlay(
                next,
                SystemDnsSnapshot::plain_servers(vec!["172.30.247.20:41864".parse().unwrap()])
                    .unwrap(),
            )
            .await;
        manager
            .reconcile(snapshot(server), key, tokio::time::Instant::now())
            .await;
        assert_eq!(manager.observation().state, ConnectionRuntimeState::Applied);
        manager
            .withdraw(meshway_protocol::ConnectionRuntimeReason::ShuttingDown)
            .await;
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
