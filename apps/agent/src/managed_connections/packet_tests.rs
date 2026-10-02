use super::*;
use crate::managed_dataplane::{
    PacketPlan, TunOptions,
    tests::{MockDevice, fixtures, packet, snapshot},
    tun::Tun,
};
use std::sync::Arc;
use tokio::sync::mpsc;

struct Mesh {
    managers: Vec<Manager>,
    devices: Vec<Arc<MockDevice>>,
    sends: Vec<mpsc::Sender<Vec<u8>>>,
    receives: Vec<mpsc::Receiver<Vec<u8>>>,
    payloads: Vec<meshway_protocol::NodeDataplane>,
    keys: Vec<PrivateKey>,
}

#[tokio::test]
async fn packet_capacity_rejection_remains_retryable_and_recovers_after_correction() {
    let (payloads, keys) = fixtures(2);
    let good = payloads[1].clone();
    let mut bad = good.clone();
    let candidate = bad.connections.outgoing[0].clone();
    bad.connections.outgoing = (0..33)
        .map(|index| {
            let mut candidate = candidate.clone();
            candidate.id = format!("candidate-{index:02}");
            candidate.transport = ConnectionTransport::NativeTcp;
            candidate.endpoint.port = 43100 + index;
            candidate
        })
        .collect();
    let plan = PacketPlan::compile(&bad, 1000).unwrap();
    let (device, _send, _receive) = MockDevice::new();
    let mut manager = Manager::new_dataplane(
        None,
        Duration::from_secs(30),
        TunOptions {
            name: "mock0".into(),
            mtu: 1000,
        },
        "http://127.0.0.1".into(),
        false,
    );
    manager.mock_tun(Tun::mock(device, &plan));
    manager
        .reconcile_dataplane(snapshot(bad), keys[1].clone(), Instant::now())
        .await
        .unwrap();
    assert_eq!(manager.observation().state, Phase::Error);
    assert_eq!(manager.observation().reason, Reason::InvalidConfiguration);
    assert!(manager.running.is_none());
    manager
        .reconcile_dataplane(snapshot(good), keys[1].clone(), Instant::now())
        .await
        .unwrap();
    assert_eq!(manager.observation().state, Phase::Applied);
    manager.withdraw(Reason::ShuttingDown).await;
}

impl Mesh {
    async fn start() -> Self {
        let (payloads, keys) = fixtures(3);
        let mut mesh = Self {
            managers: vec![],
            devices: vec![],
            sends: vec![],
            receives: vec![],
            payloads,
            keys,
        };
        for index in 0..3 {
            let plan = PacketPlan::compile(&mesh.payloads[index], 1000).unwrap();
            let (device, send, receive) = MockDevice::new();
            let mut manager = Manager::new_dataplane(
                None,
                Duration::from_secs(30),
                TunOptions {
                    name: "mock0".into(),
                    mtu: 1000,
                },
                "http://127.0.0.1".into(),
                false,
            );
            manager.mock_tun(Tun::mock(device.clone(), &plan));
            manager
                .reconcile_dataplane(
                    snapshot(mesh.payloads[index].clone()),
                    mesh.keys[index].clone(),
                    Instant::now(),
                )
                .await
                .unwrap();
            assert_eq!(manager.observation().state, Phase::Applied);
            assert_eq!(
                manager.snapshot.as_ref().unwrap().digest,
                meshway_protocol::connection_digest(&mesh.payloads[index].connections).unwrap()
            );
            mesh.managers.push(manager);
            mesh.devices.push(device);
            mesh.sends.push(send);
            mesh.receives.push(receive);
        }
        mesh
    }

    async fn attempt(&self, source: usize, target: usize) -> Vec<u8> {
        let bytes = packet(
            &self.payloads[source].configuration.node.mesh_ipv4,
            &self.payloads[target].configuration.node.mesh_ipv4,
            1,
        );
        time::timeout(Duration::from_secs(8), async {
            loop {
                self.sends[source].send(bytes.clone()).await.unwrap();
                tokio::select! {
                    _ = self.devices[target].attempted.notified() => break,
                    _ = time::sleep(Duration::from_millis(100)) => {},
                }
            }
        })
        .await
        .expect("real three-hop E2E IPv4 delivery attempt");
        bytes
    }

    async fn stop(&mut self) {
        for manager in &mut self.managers {
            manager.withdraw(Reason::ShuttingDown).await;
        }
    }
}

#[tokio::test]
async fn actual_udp_full_id_paths_only_plaintext_at_final_endpoint_and_explicit_reverse() {
    let mut mesh = Mesh::start().await;
    for device in &mesh.devices {
        device.allow_writes();
    }
    let bytes = mesh.attempt(0, 2).await;
    assert_eq!(
        time::timeout(Duration::from_secs(1), mesh.receives[2].recv())
            .await
            .unwrap(),
        Some(bytes)
    );
    assert!(
        mesh.receives[1].try_recv().is_err(),
        "relay never receives plaintext IP"
    );
    assert!(
        mesh.receives[0].try_recv().is_err(),
        "packet mode never auto echoes"
    );
    let reverse = mesh.attempt(2, 0).await;
    assert_eq!(
        time::timeout(Duration::from_secs(1), mesh.receives[0].recv())
            .await
            .unwrap(),
        Some(reverse)
    );
    let plan = PacketPlan::compile(&mesh.payloads[0], 1000).unwrap();
    let mut config = core_config(
        &mesh.payloads[0].connections,
        mesh.keys[0].clone(),
        &BTreeMap::new(),
    )
    .unwrap_or_else(|_| panic!("config"));
    plan.augment(&mut config).unwrap();
    assert!(
        config
            .trusted_peers
            .iter()
            .find(|peer| peer.node_id == mesh.payloads[2].node_id)
            .unwrap()
            .transports
            .is_empty()
    );
    assert_eq!(
        config.listeners[0].allowed_peers.as_ref().unwrap(),
        &vec![mesh.payloads[1].node_id.clone()]
    );
    mesh.stop().await;
}

#[tokio::test]
async fn withdrawal_and_reconfigure_cancel_inflight_tun_injection_and_destroy_e2e_cache() {
    let mut mesh = Mesh::start().await;
    mesh.attempt(0, 2).await; // Decrypted but blocked inside the mock device write.
    let generation = mesh.managers[2].running.as_ref().unwrap().task.id();
    let mut changed = mesh.payloads[2].clone();
    changed.revision += 1;
    changed.configuration.revision += 1;
    changed.connections.revision += 1;
    // Routing reject changes no physical adjacency. Early invalidation must still cancel.
    changed.configuration.routing_policy = meshway_protocol::RoutingPolicy::default();
    let changed = snapshot(changed);
    mesh.managers[2].invalidate_dataplane(&changed).await;
    assert_eq!(mesh.managers[2].observation().state, Phase::Withdrawn);
    mesh.devices[2].allow_writes();
    assert!(
        time::timeout(Duration::from_millis(100), mesh.receives[2].recv())
            .await
            .is_err()
    );
    mesh.managers[2]
        .reconcile_dataplane(changed, mesh.keys[2].clone(), Instant::now())
        .await
        .unwrap();
    assert_ne!(
        mesh.managers[2].running.as_ref().unwrap().task.id(),
        generation
    );
    for _ in 0..3 {
        mesh.sends[0]
            .send(packet("100.64.0.1", "100.64.0.3", 1))
            .await
            .unwrap();
        time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        time::timeout(Duration::from_millis(100), mesh.receives[2].recv())
            .await
            .is_err(),
        "old source E2E cache cannot survive receiver runtime shutdown"
    );
    mesh.stop().await;
}

#[tokio::test]
async fn independent_deadline_stops_packet_delivery_while_http_is_stalled() {
    let mut mesh = Mesh::start().await;
    mesh.attempt(0, 2).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let notify = entered.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/stall",
                axum::routing::get(move || {
                    let entered = notify.clone();
                    async move {
                        entered.notify_one();
                        std::future::pending::<axum::http::StatusCode>().await
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });
    let http = tokio::spawn(async move { reqwest::get(format!("http://{address}/stall")).await });
    entered.notified().await;
    let manager = &mut mesh.managers[2];
    manager
        .running
        .as_ref()
        .unwrap()
        .deadline
        .send(Some(Instant::now() + Duration::from_millis(100)))
        .unwrap();
    time::timeout(Duration::from_secs(2), async {
        while !manager.running.as_ref().unwrap().task.is_finished() {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!http.is_finished());
    assert_eq!(manager.observation().reason, Reason::LeaseExpired);
    mesh.devices[2].allow_writes();
    mesh.sends[0]
        .send(packet("100.64.0.1", "100.64.0.3", 1))
        .await
        .unwrap();
    assert!(
        time::timeout(Duration::from_millis(150), mesh.receives[2].recv())
            .await
            .is_err()
    );
    http.abort();
    server.abort();
    mesh.stop().await;
}

#[tokio::test]
async fn packet_device_failure_withdraws_adjacencies_and_keeps_route_sink_owned() {
    let mut mesh = Mesh::start().await;
    mesh.devices[0]
        .failed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    mesh.sends[0]
        .send(packet("100.64.0.1", "100.64.0.3", 1))
        .await
        .unwrap();
    time::timeout(Duration::from_secs(2), async {
        while mesh.managers[0].observation().state == Phase::Applied {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(mesh.managers[0].observation().reason, Reason::InternalError);
    assert!(mesh.managers[0].tun.is_some());
    mesh.stop().await;
}

#[tokio::test]
async fn a_saved_ack_cannot_turn_an_expired_fetch_into_a_new_packet_lease() {
    let (payloads, keys) = fixtures(2);
    let plan = PacketPlan::compile(&payloads[0], 1000).unwrap();
    let (device, _send, _receive) = MockDevice::new();
    let mut manager = Manager::new_dataplane(
        None,
        Duration::from_secs(1),
        TunOptions {
            name: "mock0".into(),
            mtu: 1000,
        },
        "http://127.0.0.1".into(),
        false,
    );
    manager.mock_tun(Tun::mock(device, &plan));
    manager
        .reconcile_dataplane(
            snapshot(payloads[0].clone()),
            keys[0].clone(),
            Instant::now() - Duration::from_secs(2),
        )
        .await
        .unwrap();
    assert!(manager.running.is_none());
    assert_eq!(manager.observation().state, Phase::Withdrawn);
    assert_eq!(manager.observation().reason, Reason::LeaseExpired);
    assert!(
        tokio::net::UdpSocket::bind(&payloads[0].connections.listeners[0].bind)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn failed_dns_preparation_withdraws_packets_retains_sink_and_requires_fresh_recovery() {
    let mut mesh = Mesh::start().await;
    mesh.attempt(0, 2).await;
    let device = mesh.managers[2].tun.as_ref().unwrap().device.clone();
    let mut unavailable = mesh.payloads[2].clone();
    unavailable.connections.outgoing[0].endpoint.host = "meshway-peer-not-started.invalid".into();
    // Exercise preparation itself, rather than relying on digest-change invalidation.
    let error = mesh.managers[2]
        .prepare_dataplane_sink(&snapshot(unavailable))
        .await
        .unwrap_err();
    assert!(matches!(error, crate::AgentError::DataplaneResolveFailed));
    assert!(error.retryable());
    assert!(
        mesh.managers[2].running.is_none(),
        "a failed preparation cannot renew the old lease"
    );
    assert_eq!(mesh.managers[2].observation().reason, Reason::ResolveFailed);
    assert!(Arc::ptr_eq(
        &device,
        &mesh.managers[2].tun.as_ref().unwrap().device
    ));
    mesh.devices[2].allow_writes();
    assert!(
        time::timeout(Duration::from_millis(100), mesh.receives[2].recv())
            .await
            .is_err()
    );
    let original = snapshot(mesh.payloads[2].clone());
    // Even successful DNS must not revive a previously fetched expired bundle.
    mesh.managers[2]
        .reconcile_dataplane(
            original.clone(),
            mesh.keys[2].clone(),
            Instant::now() - Duration::from_secs(60),
        )
        .await
        .unwrap();
    assert!(mesh.managers[2].running.is_none());
    assert_eq!(mesh.managers[2].observation().reason, Reason::LeaseExpired);
    mesh.managers[2]
        .reconcile_dataplane(original, mesh.keys[2].clone(), Instant::now())
        .await
        .unwrap();
    assert_eq!(mesh.managers[2].observation().state, Phase::Applied);
    assert!(Arc::ptr_eq(
        &device,
        &mesh.managers[2].tun.as_ref().unwrap().device
    ));
    mesh.stop().await;
}

#[tokio::test]
async fn resolved_mesh_endpoint_overlap_stays_fatal_and_withdraws_existing_packets() {
    let mut mesh = Mesh::start().await;
    mesh.attempt(0, 2).await;
    let mut unsafe_endpoint = mesh.payloads[2].clone();
    unsafe_endpoint.connections.outgoing[0].endpoint.host = "100.64.0.2".into();
    let error = mesh.managers[2]
        .prepare_dataplane_sink(&snapshot(unsafe_endpoint))
        .await
        .unwrap_err();
    assert!(matches!(error, crate::AgentError::Invalid(_)));
    assert!(!error.retryable());
    assert!(mesh.managers[2].running.is_none());
    assert_eq!(
        mesh.managers[2].observation().reason,
        Reason::InvalidConfiguration
    );
    assert!(mesh.managers[2].tun.is_some());
    mesh.devices[2].allow_writes();
    assert!(
        time::timeout(Duration::from_millis(100), mesh.receives[2].recv())
            .await
            .is_err()
    );
    mesh.stop().await;
}
