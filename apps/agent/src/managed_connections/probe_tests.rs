use std::{
    os::unix::fs::PermissionsExt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use super::tests::{fixture, session, snapshot, tls_profiles};
use super::*;
use crate::diagnostics::{self, ProbeError};
use meshway_agent_connections::observations::{ProbeError as ConnectionError, ProbeRequest};

fn inventory(own: &str, peer: &str) -> meshway_protocol::NodeConfiguration {
    let node = |id: &str, name: &str, index: u8| meshway_protocol::MeshNode {
        id: id.into(),
        name: name.into(),
        country: None,
        mesh_ipv4: format!("10.0.0.{index}"),
        mesh_ipv6: format!("fd00::{index}"),
    };
    meshway_protocol::NodeConfiguration {
        schema_version: 1,
        revision: 1,
        network: meshway_protocol::MeshNetwork {
            ipv4_prefix: "10.0.0.0/24".into(),
            ipv6_prefix: "fd00::/64".into(),
        },
        node: node(own, "LAX", 1),
        peers: vec![node(peer, "SHA", 2), node(&"c".repeat(64), "NYC", 3)],
        routing_policy: Default::default(),
        policy_valid: true,
        diagnostics: vec![],
    }
}

async fn ready(manager: &Manager) {
    time::timeout(Duration::from_secs(8), async {
        while !manager
            .observation()
            .peers
            .iter()
            .any(|peer| peer.probe_fresh)
        {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("real authenticated channel must become ready");
}

#[tokio::test]
async fn real_ipc_native_udp_and_reverse_tls_vless_only_direct_authenticated_neighbors() {
    for kind in [ConnectionTransport::NativeUdp, ConnectionTransport::Vless] {
        let profiles = (kind == ConnectionTransport::Vless).then(tls_profiles);
        let profile_path = profiles
            .as_ref()
            .map(|dir| dir.path().join("profiles.json"));
        let (server, skey, client, ckey) = fixture(kind);
        let mut manager = Manager::new(profile_path.clone(), Duration::from_secs(30));
        let mut names = inventory(&server.node_id, &client.node_id);
        manager.set_probe_inventory(&names);
        manager
            .reconcile(snapshot(server.clone()), skey, Instant::now())
            .await;
        let mut remote = session(&client, ckey, profile_path.as_deref()).await;
        ready(&manager).await;
        let dir = tempfile::Builder::new()
            .prefix("mw")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let state = dir.path().join("a");
        let lock_path = dir.path().join("a.lock");
        let lock = meshway_protocol::local::lock(&lock_path).unwrap();
        let socket = diagnostics::Server::bind(&state, &lock).await.unwrap();
        let prober = manager.prober();
        let task = tokio::spawn(async move { socket.serve(prober).await });
        let proof = diagnostics::probe(&state, "SHA", Duration::from_secs(3))
            .await
            .unwrap();
        assert_eq!(proof.peer_node_id, client.node_id);
        assert_eq!(proof.local_node_id, server.node_id);
        assert_eq!(
            proof.initiator_node_id, client.node_id,
            "accepted reverse-dial connection must remain attributed to SHA"
        );
        assert_eq!(proof.transport, kind);
        assert!(proof.rtt_ns > 0);
        assert!(
            meshway_protocol::local::lock(&lock_path).is_err(),
            "probe client must not take/drop the daemon state lock"
        );
        for peer in ["NYC", "localhost", "https://SHA", "bbbb", &server.node_id] {
            assert_eq!(
                diagnostics::probe(&state, peer, Duration::from_secs(1))
                    .await
                    .unwrap_err(),
                ProbeError::UnconfiguredPeer
            );
        }
        // Own inventory name participates in ambiguity; adjacency does not permit guessing.
        names.node.name = "SHA".into();
        manager.set_probe_inventory(&names);
        assert_eq!(
            diagnostics::probe(&state, "SHA", Duration::from_secs(1))
                .await
                .unwrap_err(),
            ProbeError::AmbiguousPeer
        );
        time::sleep(Duration::from_secs(1)).await;
        let proof = diagnostics::probe(&state, &client.node_id, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(proof.peer_node_id, client.node_id);
        manager.withdraw(Reason::ShuttingDown).await;
        assert_eq!(
            diagnostics::probe(&state, "SHA", Duration::from_secs(1))
                .await
                .unwrap_err(),
            ProbeError::Unavailable
        );
        task.abort();
        let _ = task.await;
        assert!(!diagnostics::socket_path(&state).exists());
        remote.shutdown().await;
    }
}

// A real UDP relay used only to delay authenticated probe frames after Noise
// establishment. It never knows/decrypts the node identity or transport keys.
async fn relay(
    target: SocketAddr,
    paused: Arc<AtomicBool>,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut client = None;
        let mut buffer = vec![0; 65536];
        loop {
            let (size, from) = socket.recv_from(&mut buffer).await.unwrap();
            let to = if from == target {
                client
            } else {
                client = Some(from);
                Some(target)
            };
            if !paused.load(Ordering::SeqCst)
                && let Some(to) = to
            {
                socket.send_to(&buffer[..size], to).await.unwrap();
            }
        }
    });
    (address, task)
}

#[tokio::test]
async fn pending_probe_cancellation_lease_expiry_and_generation_replacement_are_fenced() {
    for mode in ["replace", "expire", "timeout"] {
        let (server, skey, mut client, ckey) = fixture(ConnectionTransport::NativeUdp);
        let paused = Arc::new(AtomicBool::new(false));
        let (relay_address, relay_task) =
            relay(server.listeners[0].bind.parse().unwrap(), paused.clone()).await;
        client.outgoing[0].endpoint.host = relay_address.ip().to_string();
        client.outgoing[0].endpoint.port = relay_address.port();
        let mut manager = Manager::new(None, Duration::from_secs(30));
        manager.set_probe_inventory(&inventory(&server.node_id, &client.node_id));
        manager
            .reconcile(snapshot(server.clone()), skey.clone(), Instant::now())
            .await;
        let mut remote = session(&client, ckey, None).await;
        ready(&manager).await;
        let prober = manager.prober();
        let dir = tempfile::Builder::new()
            .prefix("mw")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let state = dir.path().join("a");
        let lock = meshway_protocol::local::lock(&dir.path().join("a.lock")).unwrap();
        let socket = diagnostics::Server::bind(&state, &lock).await.unwrap();
        let socket_prober = prober.clone();
        let socket_task = tokio::spawn(async move { socket.serve(socket_prober).await });
        let request = ProbeRequest {
            peer: "SHA".into(),
            timeout: Duration::from_secs(1),
        };
        paused.store(true, Ordering::SeqCst);
        let pending = {
            let state = state.clone();
            tokio::spawn(
                async move { diagnostics::probe(&state, "SHA", Duration::from_secs(1)).await },
            )
        };
        time::sleep(Duration::from_millis(80)).await;
        assert!(!pending.is_finished());
        pending.abort();
        let _ = pending.await;
        time::sleep(Duration::from_millis(40)).await;
        paused.store(false, Ordering::SeqCst);
        assert!(
            prober
                .probe_neighbor_observed(request.clone())
                .await
                .is_ok(),
            "IPC EOF cancellation must release the core pending slot"
        );

        paused.store(true, Ordering::SeqCst);
        let pending = {
            let prober = prober.clone();
            let request = request.clone();
            tokio::spawn(async move { prober.probe_neighbor_observed(request).await })
        };
        time::sleep(Duration::from_millis(80)).await;
        assert!(!pending.is_finished());
        if mode == "expire" {
            manager
                .running
                .as_ref()
                .unwrap()
                .deadline
                .send(Some(Instant::now() + Duration::from_millis(30)))
                .unwrap();
        } else if mode == "replace" {
            let mut changed = server.clone();
            changed.incoming.clear();
            changed.peers.clear();
            manager.invalidate_if_changed(&snapshot(changed.clone()));
            manager
                .reconcile(snapshot(changed), skey, Instant::now())
                .await;
        }
        assert_eq!(
            time::timeout(Duration::from_secs(2), pending)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err(),
            if mode == "timeout" {
                ConnectionError::Timeout
            } else {
                ConnectionError::Unavailable
            }
        );
        assert_eq!(
            prober.probe_neighbor_observed(request).await.unwrap_err(),
            match mode {
                "replace" => ConnectionError::UnconfiguredPeer,
                "expire" => ConnectionError::Unavailable,
                _ => ConnectionError::ChannelUnavailable,
            }
        );
        socket_task.abort();
        let _ = socket_task.await;
        manager.withdraw(Reason::ShuttingDown).await;
        remote.shutdown().await;
        relay_task.abort();
        let _ = relay_task.await;
    }
}
