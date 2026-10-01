use super::*;
use crate::{
    crypto::{Cipher, read_handshake, write_handshake},
    framing::{read_record, write_record},
};
use tokio::net::TcpStream;

fn managed_configs() -> Vec<LabConfig> {
    let mut configs = configs();
    for config in &mut configs {
        let index = PATH.iter().position(|id| *id == config.node_id).unwrap();
        config.node_id = format!("{:064x}", index + 1);
        config.allowed_paths.clear();
        for peer in &mut config.trusted_peers {
            let index = PATH.iter().position(|id| *id == peer.node_id).unwrap();
            peer.node_id = format!("{:064x}", index + 1);
        }
    }
    configs
}

fn listener(transport: TransportKind, id: &str, peers: Option<Vec<String>>) -> ListenerConfig {
    ListenerConfig {
        transport,
        bind: "127.0.0.1:0".parse().unwrap(),
        id: Some(id.into()),
        allowed_peers: peers,
    }
}

async fn until(condition: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

fn carrier_stats(runtime: &Runtime, transport: TransportKind) -> TransportStatistics {
    let statistics = runtime.statistics();
    match transport {
        TransportKind::NativeUdp => statistics.native_udp,
        TransportKind::NativeTcp => statistics.native_tcp,
        TransportKind::Vless => statistics.vless,
    }
}

#[test]
fn controller_ids_are_exact_and_bounded_in_config_wire_prologue_and_mtu() {
    let mut configs = managed_configs();
    for config in &configs {
        config.validate().unwrap();
    }
    let long_path: Vec<_> = configs
        .iter()
        .map(|config| config.node_id.clone())
        .collect();
    let packet = Packet {
        phase: Phase::Packet,
        id: [8; 16],
        path: long_path.clone(),
        body: vec![9],
    };
    let encoded = packet.encode().unwrap();
    let decoded = Packet::decode(&encoded).unwrap();
    assert_eq!(decoded.path, long_path);
    let prologue = packet.e2e_prologue();
    for id in &long_path {
        assert!(prologue.windows(64).any(|window| window == id.as_bytes()));
    }
    assert_eq!(native_udp_datagram_len(&path(), 1280).unwrap(), 1393);
    assert_eq!(native_udp_datagram_len(&long_path, 1280).unwrap(), 1576);
    configs[0].node_id.push('0');
    assert!(configs[0].validate().is_err());
    let mut too_long = long_path.clone();
    too_long[0].push('0');
    assert!(native_udp_datagram_len(&too_long, 1280).is_err());
    assert!(
        Packet {
            path: too_long,
            ..packet
        }
        .encode()
        .is_err()
    );
    let mut invalid = encoded;
    invalid[20] = b'/';
    assert!(Packet::decode(&invalid).is_err());
    assert!(wire::hop_prologue(TransportKind::NativeTcp).starts_with(b"meshway/lab/hop/3"));
}

#[test]
fn scoped_grants_and_context_ids_are_known_unique_and_serde_defaults_preserve_legacy() {
    let mut config = configs().remove(1);
    config.listeners.push(listener(
        TransportKind::NativeTcp,
        "listener-a",
        Some(vec!["nyc".into()]),
    ));
    config.validate().unwrap();
    for peers in [
        vec!["unknown".into()],
        vec!["nyc".into(), "nyc".into()],
        vec!["lax".into()],
    ] {
        config.listeners[0].allowed_peers = Some(peers);
        assert!(config.validate().is_err());
    }
    config.listeners[0].allowed_peers = Some(vec!["nyc".into()]);
    config
        .trusted_peers
        .iter_mut()
        .find(|peer| peer.node_id == "nyc")
        .unwrap()
        .transports
        .clear();
    assert!(config.validate().is_err());
    config.listeners[0].allowed_peers = Some(Vec::new());
    config.validate().unwrap();
    config.listeners.push(config.listeners[0].clone());
    assert!(config.validate().is_err());
    config.listeners.pop();
    config.listeners[0].id = Some("x".repeat(129));
    assert!(config.validate().is_err());
    config.listeners[0].id = Some("bad label".into());
    assert!(config.validate().is_err());

    let old: ListenerConfig =
        serde_json::from_str(r#"{"transport":"native_tcp","bind":"127.0.0.1:0"}"#).unwrap();
    assert!(old.allowed_peers.is_none());
    assert!(old.id.is_none());
    let old: OutgoingCandidate =
        serde_json::from_str(r#"{"peer":"nyc","transport":"native_tcp","endpoint":"127.0.0.1:1"}"#)
            .unwrap();
    assert!(old.id.is_none());
}

async fn denied_before_reply(
    config: &LabConfig,
    responder: &LabConfig,
    transport: TransportKind,
    address: SocketAddr,
) {
    let identity = Identity::from_private_key(config.private_key.clone());
    let mut state = identity
        .handshake(
            Some(&public_key(&responder.public_key).unwrap()),
            &wire::hop_prologue(transport),
        )
        .unwrap();
    let first = write_handshake(&mut state, &[1, 0, 0]).unwrap();
    match transport {
        TransportKind::NativeTcp => {
            let mut stream = TcpStream::connect(address).await.unwrap();
            write_record(&mut stream, &first).await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_secs(1), read_record(&mut stream))
                    .await
                    .unwrap()
                    .is_err()
            );
        }
        TransportKind::NativeUdp => {
            let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let mut datagram = vec![1];
            datagram.extend(first);
            socket.send_to(&datagram, address).await.unwrap();
            let mut bytes = [0; 512];
            assert!(
                tokio::time::timeout(Duration::from_millis(100), socket.recv_from(&mut bytes))
                    .await
                    .is_err()
            );
        }
        TransportKind::Vless => unreachable!(),
    }
}

async fn native_managed(transport: TransportKind) {
    let mut configs = managed_configs();
    let client_id = configs[0].node_id.clone();
    let server_id = configs[1].node_id.clone();
    configs[1].listeners = vec![
        listener(transport, "listener-a", Some(vec![client_id.clone()])),
        listener(transport, "listener-b", Some(Vec::new())),
    ];
    let server = Runtime::start(configs[1].clone()).await.unwrap();
    let a = server.bound_listeners()[0].address;
    let b = server.bound_listeners()[1].address;
    assert_eq!(
        server.bound_listeners()[0].id.as_deref(),
        Some("listener-a")
    );
    denied_before_reply(&configs[0], &configs[1], transport, b).await;
    until(|| {
        carrier_stats(&server, transport)
            .drop_reasons
            .unauthorized_carrier
            == 1
    })
    .await;
    assert!(server.peer_connections().is_empty());
    configs[0].outgoing.push(OutgoingCandidate {
        peer: server_id.clone(),
        transport,
        endpoint: a.to_string(),
        priority: 0,
        id: Some("candidate-a".into()),
    });
    let client = Runtime::start(configs[0].clone()).await.unwrap();
    client
        .wait_for_peer(&server_id, Duration::from_secs(2))
        .await
        .unwrap();
    server
        .wait_for_peer(&client_id, Duration::from_secs(2))
        .await
        .unwrap();
    let accepted = server.peer_connections().remove(0);
    assert_eq!(accepted.peer_node_id, client_id);
    assert!(!accepted.initiator);
    assert_eq!(accepted.listener_id.as_deref(), Some("listener-a"));
    assert!(accepted.candidate_id.is_none());
    assert!(accepted.remote_addr.is_some());
    let dialed = client.peer_connections().remove(0);
    assert_eq!(dialed.peer_node_id, server_id);
    assert!(dialed.initiator);
    assert_eq!(dialed.candidate_id.as_deref(), Some("candidate-a"));
    assert!(dialed.listener_id.is_none());
    assert_eq!(dialed.remote_addr, Some(a));
    assert_eq!(dialed.transport, transport);
    for (runtime, peer) in [(&client, &server_id), (&server, &client_id)] {
        assert!(
            runtime
                .probe_neighbor(peer, Duration::from_secs(1))
                .await
                .unwrap()
                < Duration::from_secs(1)
        );
        let path = vec![runtime.node_id().to_owned(), peer.clone()];
        assert_eq!(
            runtime.probe(path.clone(), vec![1]).await.err(),
            Some(Error::UnauthorizedPath)
        );
        assert_eq!(
            runtime.send_packet(path, vec![1]).await,
            Err(Error::UnauthorizedPath)
        );
        assert!(lock(&runtime.handle.inner.pending).is_empty());
        assert!(lock(&runtime.handle.inner.endpoints).is_empty());
        assert!(runtime.handle.inner.sending.lock().await.is_empty());
        assert_eq!(carrier_stats(runtime, transport).received, 0);
        assert_eq!(carrier_stats(runtime, transport).sent, 0);
    }
    // Even a malicious authenticated neighbor bypassing its own API cannot submit
    // an IP/application Packet without the receiver's explicit ordered path grant.
    let mut delivered = server.take_packet_receiver().unwrap();
    client
        .handle
        .inner
        .send(
            &server_id,
            Packet {
                phase: Phase::Packet,
                id: [1; 16],
                path: vec![client_id.clone(), server_id.clone()],
                body: vec![4; 32],
            },
        )
        .unwrap();
    until(|| {
        carrier_stats(&server, transport)
            .drop_reasons
            .unauthorized_path
            == 1
    })
    .await;
    assert!(delivered.try_recv().is_err());
    assert_eq!(server.peer_transport(&client_id), Some(transport));
    // A rejected second listener cannot evict the already confirmed connection.
    denied_before_reply(&configs[0], &configs[1], transport, b).await;
    assert_eq!(
        server.peer_connections()[0].listener_id.as_deref(),
        Some("listener-a")
    );
    server
        .probe_neighbor(&client_id, Duration::from_secs(1))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(client.shutdown(), server.shutdown());
    })
    .await
    .unwrap();
    assert!(server.peer_connections().is_empty());
    assert!(client.peer_connections().is_empty());
    assert!(delivered.recv().await.is_none());
}

#[tokio::test]
async fn real_native_tcp_controller_ids_scoped_admission_and_bidirectional_neighbor_only() {
    native_managed(TransportKind::NativeTcp).await;
}

#[tokio::test]
async fn real_native_udp_controller_ids_scoped_admission_and_bidirectional_neighbor_only() {
    native_managed(TransportKind::NativeUdp).await;
}

#[tokio::test]
async fn endpoint_only_identity_is_denied_on_native_listener_even_with_known_key() {
    for transport in [TransportKind::NativeTcp, TransportKind::NativeUdp] {
        let mut configs = managed_configs();
        let client_id = configs[0].node_id.clone();
        configs[1]
            .trusted_peers
            .iter_mut()
            .find(|p| p.node_id == client_id)
            .unwrap()
            .transports
            .clear();
        configs[1]
            .listeners
            .push(listener(transport, "legacy-explicit-carrier", None));
        let server = Runtime::start(configs[1].clone()).await.unwrap();
        denied_before_reply(
            &configs[0],
            &configs[1],
            transport,
            server.bound_listeners()[0].address,
        )
        .await;
        until(|| {
            carrier_stats(&server, transport)
                .drop_reasons
                .unauthorized_carrier
                == 1
        })
        .await;
        assert!(server.peer_connections().is_empty());
        server.shutdown().await;
    }
}

#[tokio::test]
async fn external_vless_stream_context_scopes_admission_and_supports_reverse_neighbor_ping() {
    let configs = managed_configs();
    let client = Runtime::start(configs[0].clone()).await.unwrap();
    let server = Runtime::start(configs[1].clone()).await.unwrap();
    let remote_addr: SocketAddr = "127.0.0.1:43210".parse().unwrap();
    for allowed in [false, true] {
        let (a, b) = tokio::io::duplex(64);
        let (dial, accept) = tokio::join!(
            client.attach_stream_with_context(
                a,
                StreamRole::Dialer {
                    peer: server.node_id().into()
                },
                TransportKind::Vless,
                0,
                StreamContext {
                    candidate_id: Some("vless-candidate".into()),
                    ..Default::default()
                }
            ),
            server.attach_stream_with_context(
                b,
                StreamRole::Acceptor,
                TransportKind::Vless,
                0,
                StreamContext {
                    listener_id: Some(if allowed { "vless-a" } else { "vless-b" }.into()),
                    allowed_peers: Some(if allowed {
                        vec![client.node_id().into()]
                    } else {
                        Vec::new()
                    }),
                    remote_addr: Some(remote_addr),
                    ..Default::default()
                }
            ),
        );
        if allowed {
            dial.unwrap();
            accept.unwrap();
        } else {
            assert!(dial.is_err());
            assert_eq!(accept, Err(Error::UnauthorizedTransport));
            assert!(server.peer_connections().is_empty());
        }
    }
    server
        .probe_neighbor(client.node_id(), Duration::from_secs(1))
        .await
        .unwrap();
    client
        .probe_neighbor(server.node_id(), Duration::from_secs(1))
        .await
        .unwrap();
    let observed = server.peer_connections().remove(0);
    assert_eq!(observed.listener_id.as_deref(), Some("vless-a"));
    assert_eq!(observed.remote_addr, Some(remote_addr));
    assert!(!observed.initiator);
    assert_eq!(
        client.peer_connections()[0].candidate_id.as_deref(),
        Some("vless-candidate")
    );
    let json = serde_json::to_value(observed).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 6);
    assert!(!json.to_string().contains("token"));
    let (a, _) = tokio::io::duplex(64);
    assert_eq!(
        server
            .attach_stream_with_context(
                a,
                StreamRole::Acceptor,
                TransportKind::Vless,
                0,
                StreamContext {
                    allowed_peers: Some(vec!["unknown".into()]),
                    ..Default::default()
                }
            )
            .await,
        Err(Error::InvalidConfig("invalid listener peer grant"))
    );
    tokio::join!(client.shutdown(), server.shutdown());
}

// Raw endpoint uses the existing Noise implementation, allowing deterministic
// withholding, replay and corruption of control replies after full confirmation.
async fn raw_stream(
    server: &Runtime,
    client: &LabConfig,
    priority: u16,
) -> (tokio::io::DuplexStream, Cipher) {
    let (mut a, b) = tokio::io::duplex(4096);
    let dial = async {
        let identity = Identity::from_private_key(client.private_key.clone());
        let mut state = identity
            .handshake(
                Some(&public_key(&server.handle.inner.config.public_key).unwrap()),
                &wire::hop_prologue(TransportKind::Vless),
            )
            .unwrap();
        let first =
            write_handshake(&mut state, &[1, (priority >> 8) as u8, priority as u8]).unwrap();
        write_record(&mut a, &first).await.unwrap();
        read_handshake(&mut state, &read_record(&mut a).await.unwrap()).unwrap();
        let mut cipher = Cipher::finish(state).unwrap();
        write_record(&mut a, &cipher.seal(wire::HOP_CONFIRM).unwrap())
            .await
            .unwrap();
        assert_eq!(
            cipher.open(&read_record(&mut a).await.unwrap()).unwrap(),
            wire::HOP_ACK
        );
        (a, cipher)
    };
    let (dial, accept) = tokio::join!(
        dial,
        server.attach_stream(b, StreamRole::Acceptor, TransportKind::Vless)
    );
    accept.unwrap();
    dial
}

async fn raw_read(stream: &mut tokio::io::DuplexStream, cipher: &mut Cipher) -> Vec<u8> {
    let bytes = tokio::time::timeout(Duration::from_secs(1), read_record(stream))
        .await
        .unwrap()
        .unwrap();
    cipher.open(&bytes).unwrap()
}

async fn raw_write(stream: &mut tokio::io::DuplexStream, cipher: &mut Cipher, body: &[u8]) {
    write_record(stream, &cipher.seal(body).unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn neighbor_nonce_size_unsolicited_replay_and_concurrency_are_bounded() {
    let configs = managed_configs();
    let server = Runtime::start(configs[1].clone()).await.unwrap();
    let peer = &configs[0].node_id;
    let (mut stream, mut cipher) = raw_stream(&server, &configs[0], 0).await;
    let operation = server.probe_neighbor(peer, Duration::from_secs(1));
    tokio::pin!(operation);
    let mut request = tokio::select! {
        result = &mut operation => panic!("probe completed without reply: {result:?}"),
        frame = raw_read(&mut stream, &mut cipher) => frame,
    };
    assert_eq!(request.len(), 18);
    assert_eq!(&request[..2], &[2, 1]);
    assert_eq!(
        server.probe_neighbor(peer, Duration::from_secs(1)).await,
        Err(Error::Capacity)
    );
    request[1] = 2;
    let mut wrong = request.clone();
    wrong[2] ^= 1;
    raw_write(&mut stream, &mut cipher, &wrong).await;
    until(|| server.statistics().vless.drop_reasons.authentication == 1).await;
    wrong.push(0);
    raw_write(&mut stream, &mut cipher, &wrong).await;
    until(|| server.statistics().vless.drop_reasons.protocol == 1).await;
    raw_write(&mut stream, &mut cipher, &request).await;
    operation.await.unwrap();
    assert!(lock(&server.handle.inner.neighbor_pending).is_empty());
    // Same request ID under a fresh cipher nonce is unsolicited, not health.
    raw_write(&mut stream, &mut cipher, &request).await;
    until(|| server.statistics().vless.drop_reasons.authentication == 2).await;
    let second = server.probe_neighbor(peer, Duration::from_secs(1));
    tokio::pin!(second);
    let mut second_request = tokio::select! {
        result = &mut second => panic!("probe completed without reply: {result:?}"),
        frame = raw_read(&mut stream, &mut cipher) => frame,
    };
    assert_ne!(&second_request[2..], &request[2..]);
    raw_write(&mut stream, &mut cipher, &request).await;
    until(|| server.statistics().vless.drop_reasons.authentication == 3).await;
    second_request[1] = 2;
    let ciphertext = cipher.seal(&second_request).unwrap();
    write_record(&mut stream, &ciphertext).await.unwrap();
    second.await.unwrap();
    // Exact encrypted replay is rejected by the uninterrupted hop cipher window.
    write_record(&mut stream, &ciphertext).await.unwrap();
    until(|| server.statistics().vless.drop_reasons.replay == 1).await;
    until(|| server.peer_connections().is_empty()).await;
    server.shutdown().await;
}

#[tokio::test]
async fn neighbor_timeout_closes_blackhole_and_reconnects_without_false_health() {
    let configs = managed_configs();
    let server = Runtime::start(configs[1].clone()).await.unwrap();
    let peer = &configs[0].node_id;
    let (mut stream, mut cipher) = raw_stream(&server, &configs[0], 0).await;
    let timeout = server.probe_neighbor(peer, Duration::from_millis(40));
    tokio::pin!(timeout);
    let request = tokio::select! {
        result = &mut timeout => panic!("probe completed without request: {result:?}"),
        frame = raw_read(&mut stream, &mut cipher) => frame,
    };
    assert_eq!(request.len(), 18);
    assert_eq!(timeout.await, Err(Error::Timeout));
    assert!(server.peer_connections().is_empty());
    assert!(lock(&server.handle.inner.neighbor_pending).is_empty());
    assert!(
        tokio::time::timeout(Duration::from_secs(1), read_record(&mut stream))
            .await
            .unwrap()
            .is_err()
    );
    let client = Runtime::start(configs[0].clone()).await.unwrap();
    attach_pair(&client, &server, 0).await;
    server
        .probe_neighbor(peer, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(
        server.probe_neighbor(peer, Duration::ZERO).await,
        Err(Error::InvalidConfig("invalid neighbor probe timeout"))
    );
    assert!(
        server
            .probe_neighbor(peer, Duration::from_secs(61))
            .await
            .is_err()
    );
    tokio::join!(client.shutdown(), server.shutdown());
}

#[tokio::test]
async fn observed_probe_after_replacement_returns_new_channel_metadata_and_proof() {
    let configs = managed_configs();
    let server = Runtime::start(configs[1].clone()).await.unwrap();
    let peer = &configs[0].node_id;
    let (_old_stream, old_cipher) = raw_stream(&server, &configs[0], 10).await;
    let old_snapshot = server.peer_connections().remove(0);
    assert_eq!(old_snapshot.transport, TransportKind::Vless);
    assert!(!old_snapshot.initiator);
    assert!(old_snapshot.candidate_id.is_none());

    // The future is deliberately not polled until B replaces A. A manager may
    // already have captured A's snapshot, but the proof must describe B entirely.
    let probe = server.probe_neighbor_observed(peer, Duration::from_secs(1));
    let client = outgoing_replacement(&server, &configs[0]).await;
    let new_token = lock(&server.handle.inner.peers).get(peer).unwrap().token;
    assert_ne!(old_cipher.id, new_token);
    let observed = probe.await.unwrap();
    assert_eq!(observed.connection.peer_node_id, *peer);
    assert_eq!(observed.connection.transport, TransportKind::NativeTcp);
    assert!(observed.connection.initiator);
    assert!(observed.connection.listener_id.is_none());
    assert_eq!(
        observed.connection.candidate_id.as_deref(),
        Some("replacement-b")
    );
    assert_eq!(
        observed.connection.remote_addr,
        Some(client.bound_listeners()[0].address)
    );
    assert!(observed.round_trip < Duration::from_secs(1));
    let json = serde_json::to_value(observed.clone()).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 2);
    assert_eq!(json["connection"].as_object().unwrap().len(), 6);
    assert!(!json.to_string().contains("token"));
    let debug = format!("{observed:?}");
    assert!(!debug.contains("token"));
    assert!(!debug.contains(&format!("{new_token:?}")));
    assert!(!debug.contains(&hex::encode(new_token)));
    tokio::join!(client.shutdown(), server.shutdown());
}

async fn outgoing_replacement(server: &Runtime, client_config: &LabConfig) -> Runtime {
    let mut config = client_config.clone();
    config.listeners.push(listener(
        TransportKind::NativeTcp,
        "replacement-listener",
        Some(vec![server.node_id().into()]),
    ));
    let client = Runtime::start(config).await.unwrap();
    let socket = TcpStream::connect(client.bound_listeners()[0].address)
        .await
        .unwrap();
    let remote_addr = socket.peer_addr().unwrap();
    server
        .attach_stream_with_context(
            socket,
            StreamRole::Dialer {
                peer: client.node_id().into(),
            },
            TransportKind::NativeTcp,
            0,
            StreamContext {
                candidate_id: Some("replacement-b".into()),
                remote_addr: Some(remote_addr),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    client
        .wait_for_peer(server.node_id(), Duration::from_secs(1))
        .await
        .unwrap();
    client
}

#[tokio::test]
async fn observed_probe_replacement_after_valid_reply_before_completion_is_unavailable() {
    let configs = managed_configs();
    let server = Runtime::start(configs[1].clone()).await.unwrap();
    let peer = &configs[0].node_id;
    let (mut stream, mut cipher) = raw_stream(&server, &configs[0], 10).await;
    let probe = server.probe_neighbor_observed(peer, Duration::from_secs(1));
    tokio::pin!(probe);
    let mut reply = tokio::select! {
        result = &mut probe => panic!("probe completed without request: {result:?}"),
        request = raw_read(&mut stream, &mut cipher) => request,
    };
    reply[1] = 2;
    raw_write(&mut stream, &mut cipher, &reply).await;
    // A's authenticated response is queued for the caller; keep that future
    // unpolled until B changes transport, direction and candidate metadata.
    until(|| lock(&server.handle.inner.neighbor_pending).is_empty()).await;
    let client = outgoing_replacement(&server, &configs[0]).await;
    assert_eq!(probe.await.err(), Some(Error::Unavailable));
    let observed = server
        .probe_neighbor_observed(peer, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(observed.connection.transport, TransportKind::NativeTcp);
    assert!(observed.connection.initiator);
    assert_eq!(
        observed.connection.candidate_id.as_deref(),
        Some("replacement-b")
    );
    tokio::join!(client.shutdown(), server.shutdown());
}

#[tokio::test]
async fn neighbor_replacement_and_old_timeout_token_cannot_remove_new_channel() {
    let configs = managed_configs();
    let server = Runtime::start(configs[1].clone()).await.unwrap();
    let peer = &configs[0].node_id;
    let (mut stream, mut cipher) = raw_stream(&server, &configs[0], 10).await;
    let old_token = cipher.id;
    let probe = server.probe_neighbor_observed(peer, Duration::from_millis(100));
    tokio::pin!(probe);
    let mut old_request = tokio::select! {
        result = &mut probe => panic!("probe completed without request: {result:?}"),
        request = raw_read(&mut stream, &mut cipher) => request,
    };
    let client = Runtime::start(configs[0].clone()).await.unwrap();
    attach_pair(&client, &server, 0).await;
    let new_token = lock(&server.handle.inner.peers).get(peer).unwrap().token;
    assert_ne!(new_token, old_token);
    // Same authenticated identity on a newer channel cannot satisfy an old
    // request even if it somehow knows that request's random ID.
    old_request[1] = 2;
    let selected = lock(&client.handle.inner.peers)
        .get(server.node_id())
        .unwrap()
        .clone();
    client
        .handle
        .inner
        .enqueue(&selected, Batch::new(vec![old_request]).unwrap())
        .map_err(|failure| failure.error)
        .unwrap();
    until(|| server.statistics().native_tcp.drop_reasons.authentication == 1).await;
    assert_eq!(probe.await.err(), Some(Error::Unavailable));
    // Exercise the exact removal primitive used by a racing timeout after replacement.
    server.handle.inner.unregister(peer, &old_token);
    assert!(server.handle.inner.active(peer, &new_token));
    server
        .probe_neighbor(peer, Duration::from_secs(1))
        .await
        .unwrap();
    tokio::join!(client.shutdown(), server.shutdown());
}

#[tokio::test(start_paused = true)]
async fn neighbor_late_authenticated_reply_is_not_health() {
    let configs = managed_configs();
    let server = Runtime::start(configs[1].clone()).await.unwrap();
    let peer = &configs[0].node_id;
    let (mut stream, mut cipher) = raw_stream(&server, &configs[0], 0).await;
    let probe = server.probe_neighbor(peer, Duration::from_secs(1));
    tokio::pin!(probe);
    let mut request = tokio::select! {
        result = &mut probe => panic!("probe completed without request: {result:?}"),
        request = raw_read(&mut stream, &mut cipher) => request,
    };
    // Leave the caller unpolled while dispatch receives a cryptographically valid
    // but late reply; dispatch itself must enforce the request's deadline.
    tokio::time::advance(Duration::from_secs(2)).await;
    request[1] = 2;
    raw_write(&mut stream, &mut cipher, &request).await;
    until(|| server.statistics().vless.drop_reasons.authentication == 1).await;
    assert_eq!(probe.await, Err(Error::Timeout));
    assert!(server.peer_connections().is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn neighbor_global_pending_limit_and_cancelled_call_cleanup() {
    let mut configs = managed_configs();
    configs[1].limits.max_pending_probes = 1;
    let server = Runtime::start(configs[1].clone()).await.unwrap();
    let (mut first, mut first_cipher) = raw_stream(&server, &configs[0], 0).await;
    let (mut second, mut second_cipher) = raw_stream(&server, &configs[2], 0).await;
    let mut probe = Box::pin(server.probe_neighbor(&configs[0].node_id, Duration::from_secs(1)));
    tokio::select! {
        result = &mut probe => panic!("probe completed without request: {result:?}"),
        _ = raw_read(&mut first, &mut first_cipher) => (),
    }
    assert_eq!(
        server
            .probe_neighbor(&configs[2].node_id, Duration::from_secs(1))
            .await,
        Err(Error::Capacity)
    );
    drop(probe);
    assert!(lock(&server.handle.inner.neighbor_pending).is_empty());
    assert_eq!(server.peer_connections().len(), 2);
    let probe = server.probe_neighbor(&configs[2].node_id, Duration::from_secs(1));
    tokio::pin!(probe);
    let mut request = tokio::select! {
        result = &mut probe => panic!("probe completed without request: {result:?}"),
        request = raw_read(&mut second, &mut second_cipher) => request,
    };
    request[1] = 2;
    raw_write(&mut second, &mut second_cipher, &request).await;
    probe.await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn neighbor_request_budget_and_shutdown_release_pending_calls_and_streams() {
    let configs = managed_configs();
    let server = Runtime::start(configs[1].clone()).await.unwrap();
    let peer = &configs[0].node_id;
    let (mut stream, mut cipher) = raw_stream(&server, &configs[0], 0).await;
    for id in 0..8 {
        let mut request = vec![2, 1];
        request.extend([id; 16]);
        raw_write(&mut stream, &mut cipher, &request).await;
        request[1] = 2;
        assert_eq!(raw_read(&mut stream, &mut cipher).await, request);
    }
    let mut request = vec![2, 1];
    request.extend([8; 16]);
    raw_write(&mut stream, &mut cipher, &request).await;
    until(|| server.statistics().vless.drop_reasons.capacity_other == 1).await;
    assert!(lock(&server.handle.inner.neighbor_pending).is_empty());
    let probe = server.probe_neighbor(peer, Duration::from_secs(60));
    tokio::pin!(probe);
    tokio::select! {
        result = &mut probe => panic!("probe completed without request: {result:?}"),
        _ = raw_read(&mut stream, &mut cipher) => (),
    }
    tokio::time::timeout(Duration::from_secs(1), server.shutdown())
        .await
        .unwrap();
    assert_eq!(probe.await, Err(Error::Closed));
    assert!(server.peer_connections().is_empty());
    assert!(lock(&server.handle.inner.neighbor_pending).is_empty());
    assert!(lock(&server.handle.inner.handles).is_empty());
    assert!(read_record(&mut stream).await.is_err());
}

#[tokio::test]
async fn native_udp_candidate_socket_is_dial_only_even_for_trusted_intended_peer() {
    let mut configs = managed_configs();
    let remote = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let peer = configs[1].node_id.clone();
    configs[0].outgoing.push(OutgoingCandidate {
        peer,
        transport: TransportKind::NativeUdp,
        endpoint: remote.local_addr().unwrap().to_string(),
        priority: 0,
        id: Some("dial-only".into()),
    });
    let client = Runtime::start(configs[0].clone()).await.unwrap();
    let mut bytes = [0; 1024];
    let (_, candidate_address) = remote.recv_from(&mut bytes).await.unwrap();
    // Another authorized peer AND the intended peer attempt to reverse-initiate.
    for index in [1, 2] {
        let identity = Identity::from_private_key(configs[index].private_key.clone());
        let mut state = identity
            .handshake(
                Some(&public_key(&configs[0].public_key).unwrap()),
                &wire::hop_prologue(TransportKind::NativeUdp),
            )
            .unwrap();
        let mut first = vec![1];
        first.extend(write_handshake(&mut state, &[1, 0, 0]).unwrap());
        remote.send_to(&first, candidate_address).await.unwrap();
    }
    until(|| {
        client
            .statistics()
            .native_udp
            .drop_reasons
            .unauthorized_carrier
            == 2
    })
    .await;
    assert!(client.peer_connections().is_empty());
    client.shutdown().await;
}
