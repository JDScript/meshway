//! Real controller + CLI + carrier socket discovery, entirely on loopback.
use super::*;
use meshway_protocol::{
    EndpointDiscoveryState, ReportEndpointDiscoveryRequest, ReportEndpointDiscoveryResponse,
    UdpDiscoveryResourceKind,
};

fn run_discovery(path: &Path, log: &Path, observer: std::net::SocketAddr) -> Process {
    let file = File::create(log).unwrap();
    std::fs::set_permissions(log, std::fs::Permissions::from_mode(0o600)).unwrap();
    Process(
        Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
            .args([
                "run",
                "--apply-connections",
                "--interval-seconds",
                "1",
                "--connection-lease-seconds",
                "10",
                "--stun-server",
                &observer.to_string(),
                "--state",
            ])
            .arg(path)
            .stdout(Stdio::from(file.try_clone().unwrap()))
            .stderr(Stdio::from(file))
            .spawn()
            .unwrap(),
    )
}

#[test]
fn invalid_discovery_cli_options_fail_before_state_loading() {
    let directory = directory();
    let path = directory.path().join("missing.json");
    for args in [
        vec!["run", "--stun-server", "127.0.0.1:3478"],
        vec![
            "run",
            "--apply-connections",
            "--stun-server",
            "stun.example:3478",
        ],
        vec![
            "run",
            "--apply-connections",
            "--stun-server",
            "0.0.0.0:3478",
        ],
        vec![
            "run",
            "--apply-connections",
            "--stun-server",
            "127.0.0.1:3478",
            "--stun-server",
            "127.0.0.1:3478",
        ],
        vec![
            "run",
            "--apply-connections",
            "--stun-server",
            "127.0.0.1:1",
            "--stun-server",
            "127.0.0.1:2",
            "--stun-server",
            "127.0.0.1:3",
            "--stun-server",
            "127.0.0.1:4",
            "--stun-server",
            "127.0.0.1:5",
        ],
    ] {
        let output = command(&path, &args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert!(!path.exists());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_udp_cli_reports_carrier_ports_and_watchdog_expires_during_http_stall() {
    let directory = directory();
    let state = AppState::open(directory.path().join("controller")).unwrap();
    let admin = admin_token(directory.path().join("controller")).unwrap();
    let stall = Arc::new(AtomicBool::new(false));
    let reports: Arc<Mutex<Vec<ReportEndpointDiscoveryRequest>>> = Arc::default();
    let watched = reports.clone();
    let stalled = stall.clone();
    let server = serve(router(state).layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let reports = watched.clone();
            let stall = stalled.clone();
            async move {
                if request.uri().path() == "/api/v1/agents/heartbeat"
                    && stall.load(Ordering::SeqCst)
                {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
                if request.uri().path() != "/api/v1/agent/connections/discovery" {
                    return next.run(request).await;
                }
                let (parts, body) = request.into_parts();
                let bytes = axum::body::to_bytes(body, 256 * 1024).await.unwrap();
                let report: ReportEndpointDiscoveryRequest =
                    serde_json::from_slice(&bytes).unwrap();
                let response = next
                    .run(axum::extract::Request::from_parts(
                        parts,
                        axum::body::Body::from(bytes),
                    ))
                    .await;
                if response.status() != 200 {
                    return response;
                }
                let (parts, body) = response.into_parts();
                let bytes = axum::body::to_bytes(body, 8192).await.unwrap();
                let receipt: ReportEndpointDiscoveryResponse =
                    serde_json::from_slice(&bytes).unwrap();
                assert_eq!(receipt.epoch, report.epoch);
                assert_eq!(receipt.sequence, report.sequence);
                reports.lock().unwrap().push(report);
                axum::response::Response::from_parts(parts, axum::body::Body::from(bytes))
            }
        },
    )))
    .await;
    let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let observer = udp.local_addr().unwrap();
    let sources: Arc<Mutex<Vec<std::net::SocketAddr>>> = Arc::default();
    let seen = sources.clone();
    let observer_task = tokio::spawn(async move {
        let mut bytes = [0_u8; 2048];
        loop {
            let (len, source) = udp.recv_from(&mut bytes).await.unwrap();
            if len != 20 || bytes[..2] != [0, 1] {
                continue;
            }
            seen.lock().unwrap().push(source);
            let mut response = bytes[..20].to_vec();
            response[..4].copy_from_slice(&[1, 1, 0, 12]);
            response.extend_from_slice(&[0, 0x20, 0, 8, 0, 1]);
            response.extend_from_slice(&(source.port() ^ 0x2112).to_be_bytes());
            let std::net::IpAddr::V4(ip) = source.ip() else {
                panic!("IPv4 observer")
            };
            response.extend(
                ip.octets()
                    .iter()
                    .zip([0x21, 0x12, 0xa4, 0x42])
                    .map(|(a, b)| a ^ b),
            );
            udp.send_to(&response, source).await.unwrap();
        }
    });
    let lax_path = directory.path().join("lax.json");
    let sha_path = directory.path().join("sha.json");
    let mut lax = enrolled(&server, &admin, &lax_path, "lax").await;
    let mut sha = enrolled(&server, &admin, &sha_path, "sha").await;
    let lax_identity = lax.publish_connection_identity().await.unwrap();
    let sha_identity = sha.publish_connection_identity().await.unwrap();
    let reserved = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let bound = reserved.local_addr().unwrap();
    drop(reserved);
    let endpoint = ConnectionEndpoint {
        host: "127.0.0.1".into(),
        port: bound.port(),
    };
    let _: ConnectionsSnapshot = admin_post(
        &server.base,
        &admin,
        "/api/v1/connections",
        SaveConnectionsRequest {
            expected_revision: sha_identity.revision,
            configuration: ConnectionPlan {
                listeners: vec![ConnectionListener {
                    id: "lax-udp".into(),
                    node_id: lax_identity.node_id,
                    transport: ConnectionTransport::NativeUdp,
                    bind: bound.to_string(),
                    advertised_endpoints: vec![endpoint.clone()],
                    credential_ref: None,
                }],
                candidates: vec![ConnectionCandidate {
                    id: "sha-to-lax".into(),
                    dialer_node_id: sha_identity.node_id,
                    listener_id: "lax-udp".into(),
                    endpoint,
                    priority: 0,
                    credential_ref: None,
                }],
            },
        },
    )
    .await;
    drop(lax);
    drop(sha);
    let lax_log = directory.path().join("lax.log");
    let sha_log = directory.path().join("sha.log");
    let _lax = run_discovery(&lax_path, &lax_log, observer);
    let _sha = run_discovery(&sha_path, &sha_log, observer);
    wait_neighbors(&server, &admin, true).await;
    let observed = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let sockets: Vec<_> = reports
                .lock()
                .unwrap()
                .iter()
                .filter(|report| report.state == EndpointDiscoveryState::Observed)
                .flat_map(|report| report.sockets.clone())
                .collect();
            if sockets
                .iter()
                .any(|s| s.resource_kind == UdpDiscoveryResourceKind::Listener)
                && sockets
                    .iter()
                    .any(|s| s.resource_kind == UdpDiscoveryResourceKind::OutgoingCandidate)
            {
                return sockets;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    for socket in &observed {
        assert!(socket.generation > 0 && socket.socket_id > 0);
        let address: std::net::SocketAddr = socket.bound_address.parse().unwrap();
        assert!(address.port() > 0);
        for observation in &socket.observations {
            let mapped: std::net::SocketAddr = observation
                .mapped_endpoint
                .as_ref()
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(mapped.port(), address.port());
            assert!(sources.lock().unwrap().contains(&mapped));
            assert!(observation.expires_at > observation.observed_at);
            assert!(
                observation.expires_at - observation.observed_at <= chrono::Duration::seconds(10)
            );
        }
    }
    assert!(
        observed
            .iter()
            .any(|s| s.resource_id == "lax-udp" && s.bound_address == bound.to_string())
    );
    assert!(observed.iter().any(|s| s.resource_id == "sha-to-lax"));
    stall.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(14), async {
        loop {
            if tokio::net::UdpSocket::bind(bound).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("watchdog must close actual carrier during stalled HTTP");
    let at_expiry = sources.lock().unwrap().len();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        sources.lock().unwrap().len(),
        at_expiry,
        "expired generation still probing"
    );
    observer_task.abort();
}
