use super::*;
use crate::{
    crypto::{Cipher, read_handshake, write_handshake},
    framing::{read_record, write_record},
};
use tokio::net::TcpStream;

async fn udp_receive(socket: &UdpSocket) -> Vec<u8> {
    let mut bytes = vec![0; MAX_NATIVE_UDP_DATAGRAM];
    let (size, _) = tokio::time::timeout(Duration::from_secs(1), socket.recv_from(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    bytes.truncate(size);
    bytes
}

fn udp_encrypted(cipher: &mut Cipher, kind: u8, plaintext: &[u8]) -> Vec<u8> {
    let mut bytes = vec![kind];
    bytes.extend_from_slice(&cipher.id);
    bytes.extend(cipher.seal(plaintext).unwrap());
    bytes
}

pub(super) async fn confirm_udp(socket: &UdpSocket, address: SocketAddr, cipher: &mut Cipher) {
    socket
        .send_to(&udp_encrypted(cipher, 4, wire::HOP_CONFIRM), address)
        .await
        .unwrap();
    let ack = udp_receive(socket).await;
    assert_eq!(ack[0], 5);
    assert_eq!(&ack[1..33], &cipher.id);
    assert_eq!(cipher.open(&ack[33..]).unwrap(), wire::HOP_ACK);
}

async fn raw_udp_connect(
    config: &LabConfig,
    responder: &LabConfig,
    socket: &UdpSocket,
    address: SocketAddr,
    priority: u16,
) -> (Vec<u8>, Cipher) {
    let identity = Identity::from_private_key(config.private_key.clone());
    let key = public_key(&responder.public_key).unwrap();
    let mut state = identity
        .handshake(Some(&key), &wire::hop_prologue(TransportKind::NativeUdp))
        .unwrap();
    let mut first = vec![1];
    first.extend(write_handshake(&mut state, &[1, (priority >> 8) as u8, priority as u8]).unwrap());
    socket.send_to(&first, address).await.unwrap();
    let response = udp_receive(socket).await;
    assert_eq!(response[0], 2);
    read_handshake(&mut state, &response[1..]).unwrap();
    let mut cipher = Cipher::finish(state).unwrap();
    confirm_udp(socket, address, &mut cipher).await;
    (first, cipher)
}

async fn wait_absent(runtime: &Runtime, peer: &str) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while runtime.peer_transport(peer).is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

fn token(runtime: &Runtime, peer: &str) -> [u8; 32] {
    lock(&runtime.handle.inner.peers).get(peer).unwrap().token
}

async fn dial_tcp(runtime: &Runtime, address: SocketAddr, priority: u16) -> Arc<Mutex<Vec<u8>>> {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let stream = Tap {
        stream: TcpStream::connect(address).await.unwrap(),
        writes: writes.clone(),
    };
    runtime
        .attach_stream_with_priority(
            stream,
            StreamRole::Dialer { peer: "lax".into() },
            TransportKind::NativeTcp,
            priority,
        )
        .await
        .unwrap();
    writes
}

#[tokio::test]
async fn captured_stream_first_flight_cannot_replace_live_channel_or_survive_restart() {
    let mut configs = configs();
    configs[1].listeners.push(ListenerConfig {
        transport: TransportKind::NativeTcp,
        bind: "127.0.0.1:0".parse().unwrap(),
        id: None,
        allowed_peers: None,
    });
    let lax = Runtime::start(configs[1].clone()).await.unwrap();
    let nyc = Runtime::start(configs[0].clone()).await.unwrap();
    let sha = Runtime::start(configs[2].clone()).await.unwrap();
    let address = lax.bound_listeners()[0].address;
    let capture = dial_tcp(&nyc, address, 0).await;
    let first = {
        let capture = lock(&capture);
        let size = u32::from_be_bytes(capture[..4].try_into().unwrap()) as usize;
        capture[4..4 + size].to_vec()
    };
    // Capture from a successfully confirmed connection, then replace it with a
    // lower-priority working connection. Replay priority 0 would deterministically
    // displace priority 10 in the old implementation, independent of hash ordering.
    lock(&nyc.handle.inner.peers)
        .get("lax")
        .unwrap()
        .close
        .send_replace(true);
    wait_absent(&nyc, "lax").await;
    wait_absent(&lax, "nyc").await;
    dial_tcp(&nyc, address, 10).await;
    attach_pair(&sha, &lax, 0).await;
    let original = token(&lax, "nyc");
    let mut attacker = TcpStream::connect(address).await.unwrap();
    write_record(&mut attacker, &first).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), read_record(&mut attacker))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(token(&lax, "nyc"), original);
    assert_eq!(
        nyc.probe(path(), b"still live".to_vec())
            .await
            .unwrap()
            .payload,
        b"still live"
    );
    // Holding the replay stream open without confirmation expires its permit.
    assert!(
        tokio::time::timeout(Duration::from_secs(1), read_record(&mut attacker))
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(token(&lax, "nyc"), original);
    lax.shutdown().await;
    wait_absent(&nyc, "lax").await;
    wait_absent(&sha, "lax").await;

    let lax = Runtime::start(configs[1].clone()).await.unwrap();
    let address = lax.bound_listeners()[0].address;
    let mut attacker = TcpStream::connect(address).await.unwrap();
    write_record(&mut attacker, &first).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), read_record(&mut attacker))
        .await
        .unwrap()
        .unwrap();
    assert!(lax.connected_peers().is_empty());
    dial_tcp(&nyc, address, 10).await;
    attach_pair(&sha, &lax, 0).await;
    assert_eq!(
        nyc.probe(path(), b"reconnected".to_vec())
            .await
            .unwrap()
            .payload,
        b"reconnected"
    );
    Fixture { nyc, lax, sha }.shutdown().await;
}

#[tokio::test]
async fn captured_udp_first_flight_and_old_confirmation_cannot_replace_live_channel() {
    let mut configs = configs();
    configs[1].listeners.push(ListenerConfig {
        transport: TransportKind::NativeUdp,
        bind: "127.0.0.1:0".parse().unwrap(),
        id: None,
        allowed_peers: None,
    });
    let lax = Runtime::start(configs[1].clone()).await.unwrap();
    let address = lax.bound_listeners()[0].address;
    let legitimate = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let (first, mut old_cipher) =
        raw_udp_connect(&configs[0], &configs[1], &legitimate, address, 0).await;
    let old_confirmation = udp_encrypted(&mut old_cipher, 4, wire::HOP_CONFIRM);
    // Simulate a lost ACK with a fresh-nonce confirmation, then keep using the cipher.
    confirm_udp(&legitimate, address, &mut old_cipher).await;
    assert_eq!(token(&lax, "nyc"), old_cipher.id);
    lax.handle.inner.unregister("nyc", &old_cipher.id);
    configs[0].outgoing.push(OutgoingCandidate {
        peer: "lax".into(),
        transport: TransportKind::NativeUdp,
        endpoint: address.to_string(),
        priority: 10,
        id: None,
    });
    let nyc = Runtime::start(configs[0].clone()).await.unwrap();
    let sha = Runtime::start(configs[2].clone()).await.unwrap();
    attach_pair(&sha, &lax, 0).await;
    nyc.wait_for_peer("lax", Duration::from_secs(1))
        .await
        .unwrap();
    let original = token(&lax, "nyc");
    let attacker = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    attacker.send_to(&first, address).await.unwrap();
    assert_eq!(udp_receive(&attacker).await[0], 2);
    attacker.send_to(&old_confirmation, address).await.unwrap();
    assert_eq!(token(&lax, "nyc"), original);
    assert_eq!(
        nyc.probe(path(), b"UDP still live".to_vec())
            .await
            .unwrap()
            .payload,
        b"UDP still live"
    );
    nyc.shutdown().await;
    lax.shutdown().await;
    wait_absent(&sha, "lax").await;

    let lax = Runtime::start(configs[1].clone()).await.unwrap();
    let address = lax.bound_listeners()[0].address;
    attacker.send_to(&first, address).await.unwrap();
    assert_eq!(udp_receive(&attacker).await[0], 2);
    let drops = lax.statistics().native_udp.dropped;
    attacker.send_to(&old_confirmation, address).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while lax.statistics().native_udp.dropped == drops {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(lax.connected_peers().is_empty());
    configs[0].outgoing[0].endpoint = address.to_string();
    let nyc = Runtime::start(configs[0].clone()).await.unwrap();
    attach_pair(&sha, &lax, 0).await;
    nyc.wait_for_peer("lax", Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(
        nyc.probe(path(), b"UDP reconnected".to_vec())
            .await
            .unwrap()
            .payload,
        b"UDP reconnected"
    );
    Fixture { nyc, lax, sha }.shutdown().await;
}

#[tokio::test]
async fn udp_initiator_waits_for_ack_and_retries_with_fresh_nonces() {
    let mut configs = configs();
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    configs[0].outgoing.push(OutgoingCandidate {
        peer: "lax".into(),
        transport: TransportKind::NativeUdp,
        endpoint: server.local_addr().unwrap().to_string(),
        priority: 0,
        id: None,
    });
    let nyc = Runtime::start(configs[0].clone()).await.unwrap();
    let mut buffer = [0; MAX_NATIVE_UDP_DATAGRAM];
    let (size, source) =
        tokio::time::timeout(Duration::from_secs(1), server.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
    let identity = Identity::from_private_key(configs[1].private_key.clone());
    let mut state = identity
        .handshake(None, &wire::hop_prologue(TransportKind::NativeUdp))
        .unwrap();
    let setup = read_handshake(&mut state, &buffer[1..size]).unwrap();
    let mut response = vec![2];
    response.extend(write_handshake(&mut state, &setup).unwrap());
    server.send_to(&response, source).await.unwrap();
    let mut cipher = Cipher::finish(state).unwrap();
    let first = udp_receive(&server).await;
    assert_eq!(first[0], 4);
    assert_eq!(cipher.open(&first[33..]).unwrap(), wire::HOP_CONFIRM);
    assert!(nyc.connected_peers().is_empty());
    // Discard an ACK after generating it, like a dropped network datagram.
    let _lost_ack = udp_encrypted(&mut cipher, 5, wire::HOP_ACK);
    let retry = udp_receive(&server).await;
    assert_ne!(&first[33..41], &retry[33..41]);
    assert_eq!(cipher.open(&retry[33..]).unwrap(), wire::HOP_CONFIRM);
    assert!(nyc.connected_peers().is_empty());
    server
        .send_to(&udp_encrypted(&mut cipher, 5, wire::HOP_ACK), source)
        .await
        .unwrap();
    nyc.wait_for_peer("lax", Duration::from_secs(1))
        .await
        .unwrap();
    nyc.shutdown().await;
}

fn endpoint_init(origin: &Runtime, route: Vec<String>, id: [u8; 16], mode: &[u8]) -> Packet {
    let inner = &origin.handle.inner;
    let key = inner.keys.get(route.last().unwrap()).unwrap();
    let mut packet = Packet {
        phase: Phase::Init,
        id,
        path: route,
        body: Vec::new(),
    };
    let mut state = inner
        .identity
        .handshake(Some(key), &packet.e2e_prologue())
        .unwrap();
    packet.body = write_handshake(&mut state, mode).unwrap();
    packet
}

async fn receive_packet(receiver: &mut mpsc::Receiver<ReceivedPacket>) -> ReceivedPacket {
    tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn wait_endpoint_count(runtime: &Runtime, count: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while lock(&runtime.handle.inner.endpoints).len() != count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn capacity_one_rollover_preserves_live_key_until_fresh_key_proof_and_has_reserved_bounds() {
    let mut configs = configs();
    for config in &mut configs {
        config.limits.max_pending_probes = 1;
    }
    // LAX is independently authorized as an origin for a different exact context.
    for index in [1, 2] {
        configs[index]
            .allowed_paths
            .push(vec!["lax".into(), "sha".into()]);
    }
    let nyc = Runtime::start(configs[0].clone()).await.unwrap();
    let lax = Runtime::start(configs[1].clone()).await.unwrap();
    let sha = Runtime::start(configs[2].clone()).await.unwrap();
    attach_pair(&nyc, &lax, 0).await;
    attach_pair(&sha, &lax, 0).await;
    let mut delivered = sha.take_packet_receiver().unwrap();
    nyc.send_packet(path(), b"old key".to_vec()).await.unwrap();
    assert_eq!(receive_packet(&mut delivered).await.payload, b"old key");
    let captured_init = lock(&lax.handle.inner.observed_transit)[0].clone();
    let (old_id, old_ciphertext) = {
        let mut sessions = nyc.handle.inner.sending.lock().await;
        let session = sessions.get_mut(&path()).unwrap();
        let ciphertext = session.cipher.seal(b"captured old packet").unwrap();
        // Expire ONLY the sender. The recipient remains actively valid for 60s.
        session.expires = Instant::now() - Duration::from_secs(1);
        (session.id, ciphertext)
    };
    assert!(
        lock(&sha.handle.inner.endpoints)
            .get(&("nyc".into(), old_id))
            .unwrap()
            .expires
            > Instant::now()
    );
    tokio::time::timeout(
        Duration::from_secs(1),
        nyc.send_packet(path(), b"new key".to_vec()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(receive_packet(&mut delivered).await.payload, b"new key");
    let new_id = nyc
        .handle
        .inner
        .sending
        .lock()
        .await
        .get(&path())
        .unwrap()
        .id;
    assert_ne!(new_id, old_id);
    {
        let sessions = lock(&sha.handle.inner.endpoints);
        assert_eq!(sessions.len(), 1);
        assert!(sessions.get(&("nyc".into(), new_id)).unwrap().confirmed);
        assert!(!sessions.contains_key(&("nyc".into(), old_id)));
    }

    // A captured Init from the retired session can create one pending challenge,
    // but cannot evict the new key. The captured old transport packet cannot confirm it.
    nyc.handle
        .inner
        .send(
            "lax",
            Packet {
                phase: Phase::Init,
                id: old_id,
                path: path(),
                body: captured_init,
            },
        )
        .unwrap();
    wait_endpoint_count(&sha, 2).await;
    let drops = sha.statistics().native_tcp.dropped;
    nyc.handle
        .inner
        .send(
            "lax",
            Packet {
                phase: Phase::Packet,
                id: old_id,
                path: path(),
                body: old_ciphertext,
            },
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while sha.statistics().native_tcp.dropped == drops {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(delivered.try_recv().is_err());
    assert!(
        lock(&sha.handle.inner.endpoints)
            .get(&("nyc".into(), new_id))
            .unwrap()
            .confirmed
    );
    nyc.send_packet(path(), b"active survives replay".to_vec())
        .await
        .unwrap();
    assert_eq!(
        receive_packet(&mut delivered).await.payload,
        b"active survives replay"
    );

    // A genuinely authenticated but unrelated origin/path cannot borrow the
    // occupied context's rollover slot or displace either state.
    let drops = sha.statistics().native_tcp.dropped;
    lax.handle
        .inner
        .send(
            "sha",
            endpoint_init(
                &lax,
                vec!["lax".into(), "sha".into()],
                [31; 16],
                b"packet-v1",
            ),
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while sha.statistics().native_tcp.dropped == drops {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(lock(&sha.handle.inner.endpoints).len(), 2);
    assert!(
        lock(&sha.handle.inner.endpoints)
            .keys()
            .all(|(origin, _)| origin == "nyc")
    );

    // Echo setup has an independently bounded pool, so probes work while the
    // one packet context has both an active and unconfirmed replacement key.
    assert_eq!(
        nyc.probe(path(), b"probe headroom".to_vec())
            .await
            .unwrap()
            .payload,
        b"probe headroom"
    );
    nyc.handle
        .inner
        .send("lax", endpoint_init(&nyc, path(), [32; 16], b"echo-v1"))
        .unwrap();
    wait_endpoint_count(&sha, 3).await;
    assert_eq!(
        lock(&sha.handle.inner.endpoints).len(),
        3 * configs[2].limits.max_pending_probes
    );

    // Purging expired pending state allows another capacity-one rollover, without
    // expiring the active recipient key or relying on a 60-second sleep.
    {
        let mut sessions = lock(&sha.handle.inner.endpoints);
        for session in sessions.values_mut().filter(|session| !session.confirmed) {
            session.expires = Instant::now() - Duration::from_secs(1);
        }
    }
    nyc.handle
        .inner
        .sending
        .lock()
        .await
        .get_mut(&path())
        .unwrap()
        .expires = Instant::now() - Duration::from_secs(1);
    tokio::time::timeout(
        Duration::from_secs(1),
        nyc.send_packet(path(), b"next rollover".to_vec()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        receive_packet(&mut delivered).await.payload,
        b"next rollover"
    );
    assert_eq!(lock(&sha.handle.inner.endpoints).len(), 1);
    Fixture { nyc, lax, sha }.shutdown().await;
}

// Test-only interception at the authenticated relay's outbound queue. Frames still
// traverse the real duplex carriers and endpoint crypto; no runtime fault hook.
fn intercept_peer(
    runtime: &Runtime,
    peer: &str,
) -> (mpsc::Receiver<Outbound>, mpsc::Sender<Outbound>) {
    let (tx, rx) = mpsc::channel(16);
    let mut peers = lock(&runtime.handle.inner.peers);
    let original = std::mem::replace(&mut peers.get_mut(peer).unwrap().tx, tx);
    (rx, original)
}

async fn capacity_one_fixture(probe_timeout_ms: u64) -> Fixture {
    let mut configs = configs();
    for config in &mut configs {
        config.limits.max_pending_probes = 1;
        config.limits.handshake_timeout_ms = 3_000;
        config.limits.probe_timeout_ms = probe_timeout_ms;
    }
    let nyc = Runtime::start(configs[0].clone()).await.unwrap();
    let lax = Runtime::start(configs[1].clone()).await.unwrap();
    let sha = Runtime::start(configs[2].clone()).await.unwrap();
    attach_pair(&nyc, &lax, 0).await;
    attach_pair(&sha, &lax, 0).await;
    Fixture { nyc, lax, sha }
}

async fn first_packet_loss(rollover: bool) {
    let fixture = capacity_one_fixture(3_000).await;
    let mut received = fixture.sha.take_packet_receiver().unwrap();
    let mut old_id = None;
    if rollover {
        fixture
            .nyc
            .send_packet(path(), b"old working key".to_vec())
            .await
            .unwrap();
        assert_eq!(
            receive_packet(&mut received).await.payload,
            b"old working key"
        );
        let mut sessions = fixture.nyc.handle.inner.sending.lock().await;
        let session = sessions.get_mut(&path()).unwrap();
        old_id = Some(session.id);
        session.expires = Instant::now() - Duration::from_secs(1);
    }
    let (mut frames, original) = intercept_peer(&fixture.lax, "sha");
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let forwarder = tokio::spawn(async move {
        let mut dropped_tx = Some(dropped_tx);
        while let Some(outbound) = frames.recv().await {
            let mut retained = Vec::new();
            for frame in outbound.batch.into_frames() {
                let packet = Packet::decode(&frame).unwrap();
                if packet.phase == Phase::Packet
                    && let Some(notify) = dropped_tx.take()
                {
                    notify.send(packet.id).unwrap();
                } else {
                    retained.push(frame);
                }
            }
            if !retained.is_empty() {
                original
                    .send(Outbound {
                        token: outbound.token,
                        batch: Batch::new(retained).unwrap(),
                        generation: outbound.generation,
                    })
                    .await
                    .unwrap();
            }
        }
    });
    fixture
        .nyc
        .send_packet(path(), b"lost first packet".to_vec())
        .await
        .unwrap();
    let pending_id = dropped_rx.await.unwrap();
    assert!(received.try_recv().is_err());
    {
        let sessions = lock(&fixture.sha.handle.inner.endpoints);
        assert_eq!(sessions.len(), if rollover { 2 } else { 1 });
        assert!(!sessions.get(&("nyc".into(), pending_id)).unwrap().confirmed);
        if let Some(old_id) = old_id {
            assert!(sessions.get(&("nyc".into(), old_id)).unwrap().confirmed);
        }
    }
    // The independent echo pool keeps its short configured timeout, even while
    // packet pending state has the longer cache-compatible retention budget.
    fixture
        .nyc
        .handle
        .inner
        .send(
            "lax",
            endpoint_init(&fixture.nyc, path(), [47; 16], b"echo-v1"),
        )
        .unwrap();
    wait_endpoint_count(&fixture.sha, if rollover { 3 } else { 2 }).await;
    assert!(lock(&fixture.sha.handle.inner.endpoints).len() <= 3);
    tokio::time::advance(Duration::from_secs(5)).await;
    {
        let sessions = lock(&fixture.sha.handle.inner.endpoints);
        let pending = sessions.get(&("nyc".into(), pending_id)).unwrap();
        assert!(!pending.confirmed);
        assert!(pending.expires > Instant::now());
        if let Some(echo) = sessions.get(&("nyc".into(), [47; 16])) {
            assert!(echo.expires <= Instant::now());
        }
    }
    assert_eq!(
        fixture
            .nyc
            .handle
            .inner
            .sending
            .lock()
            .await
            .get(&path())
            .unwrap()
            .id,
        pending_id
    );
    fixture
        .nyc
        .send_packet(path(), b"later packet recovers".to_vec())
        .await
        .unwrap();
    assert_eq!(
        receive_packet(&mut received).await.payload,
        b"later packet recovers"
    );
    {
        let sessions = lock(&fixture.sha.handle.inner.endpoints);
        assert!(sessions.get(&("nyc".into(), pending_id)).unwrap().confirmed);
        assert_eq!(
            sessions.values().filter(|session| session.packets).count(),
            1
        );
        assert!(sessions.len() <= 3);
        if let Some(old_id) = old_id {
            assert!(!sessions.contains_key(&("nyc".into(), old_id)));
        }
    }
    fixture.shutdown().await;
    tokio::time::timeout(Duration::from_secs(1), forwarder)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn initial_packet_loss_recovers_after_hop_timeout_with_cached_endpoint_key() {
    first_packet_loss(false).await;
}

#[tokio::test(start_paused = true)]
async fn rollover_packet_loss_recovers_after_hop_timeout_at_capacity_one() {
    first_packet_loss(true).await;
}

async fn receive_ready(frames: &mut mpsc::Receiver<Outbound>) -> Outbound {
    let outbound = tokio::time::timeout(Duration::from_secs(1), frames.recv())
        .await
        .unwrap()
        .unwrap();
    let frames = outbound.batch.into_frames();
    assert_eq!(frames.len(), 1);
    assert!(Packet::decode(&frames[0]).unwrap().phase == Phase::Ready);
    Outbound {
        token: outbound.token,
        batch: Batch::new(frames).unwrap(),
        generation: outbound.generation,
    }
}

#[tokio::test]
async fn admission_fence_discards_queued_outbound_and_ingress_before_reactivation() {
    let fixture = capacity_one_fixture(3_000).await;
    let mut delivered = fixture.sha.take_packet_receiver().unwrap();
    fixture.nyc.send_packet(path(), vec![1]).await.unwrap();
    assert_eq!(receive_packet(&mut delivered).await.payload, vec![1]);
    let (mut frames, original) = intercept_peer(&fixture.lax, "sha");
    fixture.nyc.send_packet(path(), vec![2]).await.unwrap();
    let old = tokio::time::timeout(Duration::from_secs(1), frames.recv())
        .await
        .unwrap()
        .unwrap();
    fixture.lax.retire_packet_generation();
    let _next = fixture.lax.begin_packet_generation();
    original.send(old).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), delivered.recv())
            .await
            .is_err()
    );
    assert!(
        fixture.lax.peer_transport("sha").is_some(),
        "unstarted queue discard preserves carrier"
    );

    fixture.nyc.send_packet(path(), vec![3]).await.unwrap();
    let queued = tokio::time::timeout(Duration::from_secs(1), frames.recv())
        .await
        .unwrap()
        .unwrap();
    let body = queued.batch.into_frames().pop().unwrap();
    let old_generation = fixture.sha.handle.inner.packet_generation();
    fixture.sha.retire_packet_generation();
    let _next = fixture.sha.begin_packet_generation();
    let token = lock(&fixture.sha.handle.inner.peers)["lax"].token;
    fixture.sha.handle.inner.deliver(Incoming {
        peer: "lax".into(),
        token,
        transport: TransportKind::Vless,
        body: body.clone(),
        generation: old_generation,
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), delivered.recv())
            .await
            .is_err()
    );
    // The discarded old ingress did not consume the ciphertext nonce. This
    // deliberate fresh ingress injection proves the captured packet was valid.
    fixture.sha.handle.inner.deliver(Incoming {
        peer: "lax".into(),
        token,
        transport: TransportKind::Vless,
        body,
        generation: fixture.sha.handle.inner.packet_generation(),
    });
    assert_eq!(receive_packet(&mut delivered).await.payload, vec![3]);
    fixture.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn slow_packet_setup_does_not_block_established_path() {
    let mut configs = configs();
    let direct = vec!["nyc".to_owned(), "lax".to_owned()];
    for config in &mut configs[..2] {
        config.allowed_paths.push(direct.clone());
    }
    let nyc = Runtime::start(configs[0].clone()).await.unwrap();
    let lax = Runtime::start(configs[1].clone()).await.unwrap();
    let sha = Runtime::start(configs[2].clone()).await.unwrap();
    attach_pair(&nyc, &lax, 0).await;
    attach_pair(&sha, &lax, 0).await;
    let fixture = Fixture { nyc, lax, sha };
    let mut received = fixture.lax.take_packet_receiver().unwrap();
    fixture
        .nyc
        .send_packet(direct.clone(), vec![1])
        .await
        .unwrap();
    assert_eq!(receive_packet(&mut received).await.payload, vec![1]);
    let (mut frames, original) = intercept_peer(&fixture.lax, "nyc");
    let source = fixture.nyc.clone();
    let slow = tokio::spawn(async move { source.send_packet(path(), vec![2]).await });
    let ready = receive_ready(&mut frames).await;
    // The slow setup is deterministically waiting on this withheld Ready. A
    // separate cached path must finish before its 1500ms setup timeout.
    tokio::time::timeout(
        Duration::from_millis(100),
        fixture.nyc.send_packet(direct, vec![3]),
    )
    .await
    .expect("unrelated cached path blocked by setup")
    .unwrap();
    assert_eq!(receive_packet(&mut received).await.payload, vec![3]);
    original.send(ready).await.unwrap();
    slow.await.unwrap().unwrap();
    fixture.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn concurrent_packet_setup_has_one_key_and_distinct_nonces() {
    let fixture = capacity_one_fixture(3_000).await;
    let mut received = fixture.sha.take_packet_receiver().unwrap();
    let (mut frames, original) = intercept_peer(&fixture.lax, "nyc");
    let source = fixture.nyc.clone();
    let first = tokio::spawn(async move { source.send_packet(path(), vec![1]).await });
    let ready = receive_ready(&mut frames).await;
    let source = fixture.nyc.clone();
    let second = tokio::spawn(async move { source.send_packet(path(), vec![2]).await });
    tokio::task::yield_now().await;
    assert_eq!(lock(&fixture.nyc.handle.inner.pending).len(), 1);
    assert!(frames.try_recv().is_err());
    original.send(ready).await.unwrap();
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    // Real decryption/replay checks accept both packets in order on one key.
    assert_eq!(receive_packet(&mut received).await.payload, vec![1]);
    assert_eq!(receive_packet(&mut received).await.payload, vec![2]);
    assert_eq!(fixture.nyc.handle.inner.sending.lock().await.len(), 1);
    assert_eq!(lock(&fixture.sha.handle.inner.endpoints).len(), 1);
    assert!(lock(&fixture.nyc.handle.inner.pending).is_empty());
    fixture.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn cancelled_packet_setup_releases_permit_and_shutdown_never_recaches() {
    let fixture = capacity_one_fixture(3_000).await;
    let (mut frames, original) = intercept_peer(&fixture.lax, "nyc");
    let source = fixture.nyc.clone();
    let first = tokio::spawn(async move { source.send_packet(path(), vec![1]).await });
    let old_ready = receive_ready(&mut frames).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    assert!(lock(&fixture.nyc.handle.inner.pending).is_empty());
    // Cancellation keeps only bounded setup state; an immediate retry uses the
    // same Init rather than blackholing behind the remote pending slot for 60s.
    let source = fixture.nyc.clone();
    let second = tokio::spawn(async move { source.send_packet(path(), vec![2]).await });
    let ready = receive_ready(&mut frames).await;
    assert_eq!(lock(&fixture.sha.handle.inner.endpoints).len(), 1);
    fixture.nyc.shutdown().await;
    assert_eq!(second.await.unwrap(), Err(Error::Closed));
    let _ = original.send(old_ready).await;
    let _ = original.send(ready).await;
    assert!(fixture.nyc.handle.inner.sending.lock().await.is_empty());
    assert!(lock(&fixture.nyc.handle.inner.pending).is_empty());
    assert_eq!(
        fixture.nyc.send_packet(path(), vec![3]).await,
        Err(Error::Closed)
    );
    fixture.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn delayed_ready_consumes_init_time_budget_and_expired_ready_is_not_cached() {
    let fixture = capacity_one_fixture(60_000).await;
    let mut received = fixture.sha.take_packet_receiver().unwrap();
    let (mut frames, original) = intercept_peer(&fixture.lax, "nyc");
    let started = Instant::now();
    let source = fixture.nyc.clone();
    let send =
        tokio::spawn(async move { source.send_packet(path(), b"delayed ready".to_vec()).await });
    let ready = receive_ready(&mut frames).await;
    let ready_at = Instant::now();
    tokio::time::advance(Duration::from_secs(5)).await;
    original.send(ready).await.unwrap();
    send.await.unwrap().unwrap();
    assert_eq!(
        receive_packet(&mut received).await.payload,
        b"delayed ready"
    );
    {
        let mut sessions = fixture.nyc.handle.inner.sending.lock().await;
        let session = sessions.get_mut(&path()).unwrap();
        assert!(session.expires >= started + Duration::from_secs(59));
        assert!(session.expires <= ready_at + Duration::from_secs(59));
        assert!(session.expires <= Instant::now() + Duration::from_secs(54));
        session.expires = Instant::now() - Duration::from_secs(1);
    }
    let source = fixture.nyc.clone();
    let send = tokio::spawn(async move {
        source
            .send_packet(path(), b"expired ready must not send".to_vec())
            .await
    });
    let ready = receive_ready(&mut frames).await;
    // Past the sender's 59s cache budget but before the caller's 60s timeout.
    tokio::time::advance(Duration::from_millis(59_500)).await;
    original.send(ready).await.unwrap();
    assert_eq!(send.await.unwrap(), Err(Error::Timeout));
    assert!(fixture.nyc.handle.inner.sending.lock().await.is_empty());
    assert!(received.try_recv().is_err());
    assert!(lock(&fixture.sha.handle.inner.endpoints).len() <= 2);
    fixture.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn cancellation_after_ready_before_cache_insert_reuses_finished_key_without_nonce_reuse() {
    let fixture = capacity_one_fixture(3_000).await;
    let mut received = fixture.sha.take_packet_receiver().unwrap();
    let (mut frames, original) = intercept_peer(&fixture.lax, "nyc");
    let source = fixture.nyc.clone();
    let first = tokio::spawn(async move { source.send_packet(path(), vec![1]).await });
    let ready = receive_ready(&mut frames).await;
    let cache = fixture.nyc.handle.inner.sending.lock().await;
    original.send(ready).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !lock(&fixture.nyc.handle.inner.pending).is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    assert!(cache.is_empty());
    drop(cache);
    fixture.nyc.send_packet(path(), vec![2]).await.unwrap();
    fixture.nyc.send_packet(path(), vec![3]).await.unwrap();
    assert_eq!(receive_packet(&mut received).await.payload, vec![2]);
    assert_eq!(receive_packet(&mut received).await.payload, vec![3]);
    assert!(
        frames.try_recv().is_err(),
        "finished setup needs no second Init"
    );
    assert!(lock(&fixture.nyc.handle.inner.packet_retries).is_empty());
    fixture.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn late_ready_retry_replays_only_identical_challenge_without_extending_expiry() {
    let fixture = capacity_one_fixture(3_000).await;
    let mut received = fixture.sha.take_packet_receiver().unwrap();
    let (mut frames, original) = intercept_peer(&fixture.lax, "nyc");
    let source = fixture.nyc.clone();
    let first = tokio::spawn(async move { source.send_packet(path(), vec![1]).await });
    let ready = receive_ready(&mut frames).await;
    let expiry = lock(&fixture.sha.handle.inner.endpoints)
        .values()
        .next()
        .unwrap()
        .expires;
    tokio::time::advance(Duration::from_secs(4)).await;
    assert_eq!(first.await.unwrap(), Err(Error::Timeout));
    original.send(ready).await.unwrap(); // Late response has no pending caller.
    tokio::task::yield_now().await;
    let source = fixture.nyc.clone();
    let retry = tokio::spawn(async move { source.send_packet(path(), vec![2]).await });
    let ready = receive_ready(&mut frames).await;
    assert_eq!(
        lock(&fixture.sha.handle.inner.endpoints)
            .values()
            .next()
            .unwrap()
            .expires,
        expiry
    );
    original.send(ready).await.unwrap();
    retry.await.unwrap().unwrap();
    assert_eq!(receive_packet(&mut received).await.payload, vec![2]);
    assert_eq!(lock(&fixture.sha.handle.inner.endpoints).len(), 1);
    fixture.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn lost_ready_timeout_retries_same_setup_without_pending_blackhole() {
    let fixture = capacity_one_fixture(3_000).await;
    let mut received = fixture.sha.take_packet_receiver().unwrap();
    let (mut frames, original) = intercept_peer(&fixture.lax, "nyc");
    let source = fixture.nyc.clone();
    let send =
        tokio::spawn(async move { source.send_packet(path(), b"lost ready".to_vec()).await });
    let ready = receive_ready(&mut frames).await;
    let original_id = Packet::decode(&ready.batch.into_frames()[0]).unwrap().id;
    // Restore normal forwarding, but deliberately do not redeliver the lost Ready.
    lock(&fixture.lax.handle.inner.peers)
        .get_mut("nyc")
        .unwrap()
        .tx = original;
    tokio::time::advance(Duration::from_secs(4)).await;
    assert_eq!(send.await.unwrap(), Err(Error::Timeout));
    assert!(fixture.nyc.handle.inner.sending.lock().await.is_empty());
    assert!(received.try_recv().is_err());
    {
        let sessions = lock(&fixture.sha.handle.inner.endpoints);
        assert_eq!(sessions.len(), 1);
        assert!(
            sessions
                .values()
                .all(|session| !session.confirmed && session.expires > Instant::now())
        );
    }
    // Healthy-carrier retry succeeds immediately, on the original key; no
    // waiting for the receiver's 60-second challenge retention to expire.
    fixture
        .nyc
        .send_packet(path(), b"setup after expiry".to_vec())
        .await
        .unwrap();
    assert_eq!(
        receive_packet(&mut received).await.payload,
        b"setup after expiry"
    );
    assert_eq!(lock(&fixture.sha.handle.inner.endpoints).len(), 1);
    assert_eq!(
        fixture.nyc.handle.inner.sending.lock().await[&path()].id,
        original_id
    );
    fixture.shutdown().await;
}
