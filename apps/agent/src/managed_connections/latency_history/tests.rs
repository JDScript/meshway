use super::super::tests::{fixture, listener_port, session as start_session, snapshot};
use super::*;
use meshway_protocol::*;
use serde::{Serialize, de::DeserializeOwned};
use std::{
    os::unix::fs::PermissionsExt,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::{
    net::{TcpListener, UdpSocket},
    time::Instant,
};

fn wire<T: DeserializeOwned>(value: &impl Serialize) -> T {
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

async fn post<T: DeserializeOwned>(
    base: &str,
    path: &str,
    token: &str,
    body: &impl Serialize,
) -> T {
    let response = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body(serde_json::to_vec(body).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "{path}");
    serde_json::from_slice(&response.bytes().await.unwrap()).unwrap()
}

async fn controller(dir: &std::path::Path) -> (String, JoinHandle<()>) {
    let state = meshway_controller::AppState::open(dir).unwrap();
    let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", socket.local_addr().unwrap());
    (
        url,
        tokio::spawn(async move {
            axum::serve(socket, meshway_controller::router(state))
                .await
                .unwrap();
        }),
    )
}

async fn enrolled(
    base: &str,
    admin: &str,
    network: &str,
    key: &meshway_dataplane::Identity,
    name: &str,
) -> (String, String, HttpClient) {
    let enrollment: CreateEnrollmentTokenResponse = post(
        base,
        &format!("/api/v1/networks/{network}/enrollment-tokens"),
        admin,
        &CreateEnrollmentTokenRequest {
            expires_in_seconds: 3600,
        },
    )
    .await;
    let credential = local::credential().unwrap();
    let response = HttpClient::new()
        .with_base_url(base)
        .with_api_key(enrollment.token)
        .enroll_agent(wire(&EnrollAgentRequest {
            node_credential: credential.clone(),
            name: name.into(),
            agent_version: "history-test".into(),
        }))
        .await
        .unwrap();
    let response: AgentNodeResponse = wire(&response);
    let client = HttpClient::new()
        .with_base_url(base)
        .with_api_key(credential.clone());
    client
        .publish_connection_identity(wire(&PublishConnectionIdentityRequest {
            public_key: key.public_key_hex(),
            capabilities: crate::compiled_connection_capabilities(),
        }))
        .await
        .unwrap();
    (response.node.id, credential, client)
}

async fn history(
    base: &str,
    admin: &str,
    network: &str,
    source: &str,
    peer: &str,
) -> LatencyHistoryResponse {
    post(
        base,
        &format!("/api/v1/networks/{network}/latency/history/query"),
        admin,
        &QueryLatencyHistoryRequest {
            source_node_id: source.into(),
            peer_node_id: peer.into(),
            from: Utc::now() - chrono::Duration::hours(1),
            until: Utc::now() + chrono::Duration::seconds(3),
            limit: 100,
            after: None,
        },
    )
    .await
}

#[tokio::test]
async fn real_agent_sampler_generated_http_persistence_restart_and_withdrawal() {
    let dir = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let (base, server_task) = controller(dir.path()).await;
    let admin = meshway_controller::admin_token(dir.path()).unwrap();
    let networks: NetworkList = serde_json::from_slice(
        &reqwest::Client::new()
            .get(format!("{base}/api/v1/networks"))
            .bearer_auth(&admin)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
    )
    .unwrap();
    let network = &networks.networks[0].network_id;
    let a = meshway_dataplane::Identity::generate().unwrap();
    let b = meshway_dataplane::Identity::generate().unwrap();
    let (a_id, credential, a_http) = enrolled(&base, &admin, network, &a, "source").await;
    let (b_id, _, b_http) = enrolled(&base, &admin, network, &b, "peer").await;
    let port = listener_port(ConnectionTransport::NativeUdp);
    let config: ConnectionsSnapshot = serde_json::from_slice(
        &reqwest::Client::new()
            .get(format!("{base}/api/v1/networks/{network}/connections"))
            .bearer_auth(&admin)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
    )
    .unwrap();
    let _: ConnectionsSnapshot = post(
        &base,
        &format!("/api/v1/networks/{network}/connections"),
        &admin,
        &SaveConnectionsRequest {
            expected_revision: config.revision,
            configuration: ConnectionPlan {
                listeners: vec![ConnectionListener {
                    id: "history-listener".into(),
                    node_id: a_id.clone(),
                    transport: ConnectionTransport::NativeUdp,
                    bind: format!("127.0.0.1:{port}"),
                    advertised_endpoints: vec![ConnectionEndpoint {
                        host: "127.0.0.1".into(),
                        port,
                    }],
                    credential_ref: None,
                }],
                candidates: vec![ConnectionCandidate {
                    id: "history-dial".into(),
                    dialer_node_id: b_id.clone(),
                    listener_id: "history-listener".into(),
                    endpoint: ConnectionEndpoint {
                        host: "127.0.0.1".into(),
                        port,
                    },
                    priority: 0,
                    credential_ref: None,
                }],
            },
        },
    )
    .await;
    let a_snapshot: AgentConnectionsSnapshot = wire(&a_http.get_agent_connections().await.unwrap());
    let b_snapshot: AgentConnectionsSnapshot = wire(&b_http.get_agent_connections().await.unwrap());
    let epoch = a_http
        .begin_connection_runtime(wire(&BeginConnectionRuntimeRequest {
            expected_epoch: 0,
            instance_id: local::credential().unwrap(),
        }))
        .await
        .unwrap()
        .epoch;
    let epoch = u32::try_from(epoch).unwrap();
    let mut manager = Manager::new(None, Duration::from_secs(30));
    manager
        .reconcile(
            a_snapshot.clone(),
            meshway_dataplane::PrivateKey::from_hex(&a.private_key_hex()).unwrap(),
            Instant::now(),
        )
        .await;
    let mut remote = start_session(
        &b_snapshot.payload,
        meshway_dataplane::PrivateKey::from_hex(&b.private_key_hex()).unwrap(),
        None,
    )
    .await;
    remote
        .runtime
        .wait_for_peer(&a_id, Duration::from_secs(5))
        .await
        .unwrap();
    // A diagnostic nonce probe has happened, and the runtime report is fresh.
    // Neither operation may produce a history row by itself.
    manager
        .prober()
        .probe_neighbor_observed(crate::diagnostics::ProbeRequest {
            peer: b_id.clone(),
            timeout_ms: 3000,
        })
        .await
        .unwrap();
    a_http
        .report_connection_runtime(wire(&ReportConnectionRuntimeRequest {
            epoch,
            sequence: 1,
            revision: a_snapshot.payload.revision,
            digest: a_snapshot.digest.clone(),
            state: ConnectionRuntimeState::Applied,
            reason: ConnectionRuntimeReason::None,
            listeners: a_snapshot
                .payload
                .listeners
                .iter()
                .map(|l| ConnectionListenerObservation {
                    listener_id: l.id.clone(),
                    state: ConnectionListenerState::Bound,
                    reason: ConnectionRuntimeReason::None,
                })
                .collect(),
            peers: vec![],
        }))
        .await
        .unwrap();
    assert!(
        history(&base, &admin, network, &a_id, &b_id)
            .await
            .records
            .is_empty()
    );
    let sampler = manager
        .start_latency_history(a_http.clone(), epoch)
        .unwrap();
    let first = time::timeout(Duration::from_secs(35), async {
        loop {
            let h = history(&base, &admin, network, &a_id, &b_id).await;
            if !h.records.is_empty() {
                break h.records[0].clone();
            }
            time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(first.source_node_id, a_id);
    assert_eq!(first.sample.peer_node_id, b_id);
    assert_eq!(first.sample.outcome, LatencyOutcome::Success);
    assert!(first.sample.rtt_us.unwrap() > 0);
    assert_eq!(
        first.sample.channel_associations[0].transport,
        ConnectionTransport::NativeUdp
    );
    assert_eq!(first.sample.channel_associations[0].initiator_node_id, b_id);
    assert_eq!(
        first.sample.channel_associations[0].listener_id,
        "history-listener"
    );
    // The controller persists receipt timestamps as integer milliseconds;
    // sample timestamps retain sub-ms precision. Compare at the stored precision.
    assert!(
        first.historical
            && first.received_at.timestamp_millis() >= first.sample.measured_at.timestamp_millis()
    );
    let response = a_http
        .report_latency_samples(wire(&ReportLatencySamplesRequest {
            samples: vec![first.sample.clone()],
        }))
        .await
        .unwrap();
    assert_eq!(response.duplicates, 1);
    let mut forged = serde_json::to_value(ReportLatencySamplesRequest {
        samples: vec![first.sample.clone()],
    })
    .unwrap();
    forged["source_node_id"] = serde_json::json!(b_id);
    let reject = reqwest::Client::new()
        .post(format!("{base}/api/v1/agent/latency/samples"))
        .bearer_auth(&credential)
        .header("content-type", "application/json")
        .body(serde_json::to_vec(&forged).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(reject.status(), 400);
    manager.withdraw(super::super::Reason::ShuttingDown).await;
    time::timeout(Duration::from_secs(2), sampler)
        .await
        .unwrap()
        .unwrap();
    remote.shutdown().await;
    server_task.abort();
    let _ = server_task.await;
    let (base, restarted) = controller(dir.path()).await;
    let retained = history(&base, &admin, network, &a_id, &b_id).await;
    assert_eq!(retained.records.len(), 1);
    assert_eq!(retained.records[0].sample, first.sample);
    assert_eq!(retained.records[0].received_at, first.received_at);
    assert!(
        history(&base, &admin, network, &b_id, &a_id)
            .await
            .records
            .is_empty()
    );
    restarted.abort();
    let _ = restarted.await;
}

#[tokio::test]
async fn real_authenticated_udp_timeout_and_missing_channel_are_distinct() {
    let (server, skey, mut client, ckey) = fixture(ConnectionTransport::NativeUdp);
    let server_address: std::net::SocketAddr = server.listeners[0].bind.parse().unwrap();
    let proxy = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client.outgoing[0].endpoint.host = "127.0.0.1".into();
    client.outgoing[0].endpoint.port = proxy.local_addr().unwrap().port();
    let blocked = Arc::new(AtomicBool::new(false));
    let drop_packets = blocked.clone();
    let relay = tokio::spawn(async move {
        let mut remote = None;
        let mut buf = vec![0; 65536];
        loop {
            let (size, address) = proxy.recv_from(&mut buf).await.unwrap();
            if drop_packets.load(Ordering::SeqCst) {
                continue;
            }
            let to = if address == server_address {
                let Some(remote) = remote else {
                    continue;
                };
                remote
            } else {
                remote = Some(address);
                server_address
            };
            proxy.send_to(&buf[..size], to).await.unwrap();
        }
    });
    let mut a = start_session(&server, skey, None).await;
    let mut b = start_session(&client, ckey, None).await;
    a.runtime
        .wait_for_peer(&client.node_id, Duration::from_secs(4))
        .await
        .unwrap();
    let config = snapshot(server);
    let generation = local::credential().unwrap();
    let good = measure(
        &a.runtime,
        &config,
        1,
        &generation,
        &client.node_id,
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert_eq!(good.outcome, LatencyOutcome::Success);
    blocked.store(true, Ordering::SeqCst);
    {
        // Reserve a real pending nonce slot first. Local capacity pressure must
        // expire inside the original budget without inventing a timeout sample.
        let pending = a
            .runtime
            .probe_neighbor_observed(&client.node_id, Duration::from_secs(1));
        tokio::pin!(pending);
        std::future::poll_fn(|cx| {
            assert!(pending.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        let began = Instant::now();
        assert!(
            measure(
                &a.runtime,
                &config,
                1,
                &generation,
                &client.node_id,
                Duration::from_millis(100),
            )
            .await
            .is_none()
        );
        assert!(began.elapsed() >= Duration::from_millis(100));
        assert!(began.elapsed() < Duration::from_secs(1));
    }
    let timeout = measure(
        &a.runtime,
        &config,
        1,
        &generation,
        &client.node_id,
        Duration::from_millis(100),
    )
    .await
    .unwrap();
    assert_eq!(timeout.outcome, LatencyOutcome::Timeout);
    assert!(timeout.rtt_us.is_none() && timeout.channel_associations.is_empty());
    assert!(
        measure(
            &a.runtime,
            &config,
            1,
            &generation,
            &client.node_id,
            Duration::from_millis(100)
        )
        .await
        .is_none()
    );
    relay.abort();
    let _ = relay.await;
    a.shutdown().await;
    b.shutdown().await;
}

#[tokio::test]
async fn lease_expiry_cancels_runner_without_reporting_a_missing_sample() {
    let (server, key, _, _) = fixture(ConnectionTransport::NativeUdp);
    let mut manager = Manager::new(None, Duration::from_secs(30));
    manager
        .reconcile(snapshot(server), key, Instant::now())
        .await;
    let task = manager
        .start_latency_history(HttpClient::new().with_base_url("http://127.0.0.1:1"), 1)
        .unwrap();
    // Expire an already-published lease, rather than racing a 100 ms startup
    // budget against other workspace tests and filesystem/crypto scheduling.
    manager
        .running
        .as_ref()
        .unwrap()
        .deadline
        .send_replace(Some(Instant::now()));
    let summary = time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.sampled, 0);
    assert!(manager.prober().live_status(true, 0, 1).is_err());
    manager.withdraw(super::super::Reason::ShuttingDown).await;
}
