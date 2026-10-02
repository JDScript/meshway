//! Protected socket/HTTP/carrier support in disposable namespace fixtures only.
use std::{io, net::SocketAddr, sync::Arc, time::Duration};

use tokio::io::AsyncWriteExt;

use super::{ManagedProtection, http::ControllerHttp};
use meshway_agent_underlay::{
    BootstrapResolver, CapturePrefix, LinuxUnderlay, Preparation, PreparedGeneration, Purpose,
    SocketFactory, SystemDnsSnapshot,
};
use meshway_control_client::{
    client::{ApiOpError, HttpClient, HttpTransport},
    types::EnrollAgentRequest,
};

type FixtureOwner = tokio::sync::Mutex<Option<(tempfile::TempDir, LinuxUnderlay)>>;
static OWNER: std::sync::OnceLock<FixtureOwner> = std::sync::OnceLock::new();
static TRANSPORTS: std::sync::Mutex<Vec<Arc<ControllerHttp>>> = std::sync::Mutex::new(Vec::new());

async fn prepared() -> PreparedGeneration {
    let mut fixture = OWNER
        .get_or_init(|| tokio::sync::Mutex::new(None))
        .lock()
        .await;
    if let Some((_, owner)) = fixture.as_ref() {
        return owner.socket_generation().await.unwrap();
    }
    let directory = tempfile::Builder::new().tempdir_in("/run").unwrap();
    let mut owner = LinuxUnderlay::prepare(Preparation {
        state_directory: directory.path(),
        identity: "isolated-app-fixture",
        network: "fixture-network",
    })
    .await
    .unwrap();
    owner
        .capture(&[
            CapturePrefix {
                prefix: "172.30.247.20/32".parse().unwrap(),
                tun: None,
            },
            CapturePrefix {
                prefix: "fd74:6d77:247::20/128".parse().unwrap(),
                tun: None,
            },
        ])
        .await
        .unwrap();
    // The test process keeps the real owner and private directory. Factories
    // retain its namespace lease; no reservation or device identity is fabricated.
    let generation = owner.socket_generation().await.unwrap();
    *fixture = Some((directory, owner));
    generation
}

async fn finish() {
    let transports = std::mem::take(&mut *TRANSPORTS.lock().unwrap());
    for transport in &transports {
        transport.join().await;
    }
    drop(transports);
    let (directory, owner) = OWNER.get().unwrap().lock().await.take().unwrap();
    owner.relinquish().await.unwrap();
    drop(directory);
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
    native_runtime_paths_inner().await;
    finish().await;
}

async fn native_runtime_paths_inner() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_FIXTURE").as_deref(),
        Ok("1")
    );
    for version in [4, 6] {
        for phase in 0..4 {
            let generation = prepared().await;
            let factory = generation.factory();
            let protection = Arc::new(ManagedProtection::new(factory, system_dns(version)));
            let mut config = native_config(version, phase, false);
            // Same protected bootstrap seam used by Manager::resolve_targets;
            // Runtime receives only the resulting numeric candidate.
            for candidate in &mut config.outgoing {
                let endpoint: SocketAddr = candidate.endpoint.parse().unwrap();
                let targets = meshway_agent_connections::sockets::Protection::resolve(
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
            let next = prepared().await;
            let protection = Arc::new(ManagedProtection::new(next.factory(), system_dns(version)));
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
    managed_native_owner_lifecycle_inner().await;
    finish().await;
}

async fn managed_native_owner_lifecycle_inner() {
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
        let generation = prepared().await;
        let old = generation.factory();
        let upstreams = system_dns(4);
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
        old.retire();
        tokio::time::timeout(Duration::from_secs(2), async {
            while manager.observation().state == ConnectionRuntimeState::Applied {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let next = prepared().await;
        manager.install_prepared_underlay(next, system_dns(4)).await;
        manager
            .reconcile(snapshot(server), key, tokio::time::Instant::now())
            .await;
        assert_eq!(manager.observation().state, ConnectionRuntimeState::Applied);
        manager
            .withdraw(meshway_protocol::ConnectionRuntimeReason::ShuttingDown)
            .await;
    }
}

fn system_dns(version: u8) -> SystemDnsSnapshot {
    let address = if version == 4 {
        "172.30.247.20"
    } else {
        "fd74:6d77:247::20"
    };
    std::fs::write("/etc/resolv.conf", format!("nameserver {address}\n")).unwrap();
    SystemDnsSnapshot::from_system().unwrap()
}
fn resolver(factory: &SocketFactory, version: u8) -> BootstrapResolver {
    BootstrapResolver::new(factory.clone(), system_dns(version))
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
    let transport = Arc::new(
        ControllerHttp::new(
            factory.clone(),
            resolver(factory, version),
            origin.parse().unwrap(),
            roots,
        )
        .unwrap(),
    );
    TRANSPORTS.lock().unwrap().push(transport.clone());
    transport
}

async fn request(
    client: &ControllerHttp,
    url: &str,
) -> meshway_control_client::client::HttpResult<reqwest::Response> {
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
    production_paths_inner().await;
    finish().await;
}

async fn production_paths_inner() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_FIXTURE").as_deref(),
        Ok("1")
    );
    let generation = prepared().await;
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

    let new_generation = prepared().await;
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
            assert!(!matches!(
                tokio::time::timeout(
                    Duration::from_millis(300),
                    tokio::net::TcpStream::connect(SocketAddr::new(ip, port))
                )
                .await,
                Ok(Ok(_))
            ));
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
            if socket
                .send_to(b"ordinary-captured", SocketAddr::new(ip, port))
                .await
                .is_err()
            {
                continue; // The owner's explicit reject-only capture has no fall-through.
            }
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
