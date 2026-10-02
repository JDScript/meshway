use super::*;
use crate::{ListenerConfig, MAX_FRAME_SIZE, OutgoingCandidate, PeerConfig, PrivateKey};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::UdpSocket;

mod control_recovery;
mod discovery;
mod managed;
mod observability;
mod packet_io;
mod security;
mod socket_options;

const PATH: [&str; 3] = ["nyc", "lax", "sha"];

fn path() -> Vec<String> {
    PATH.iter().map(|id| (*id).to_owned()).collect()
}

fn configs() -> Vec<LabConfig> {
    let identities: Vec<_> = (0..3).map(|_| Identity::generate().unwrap()).collect();
    PATH.iter()
        .enumerate()
        .map(|(index, id)| LabConfig {
            node_id: (*id).into(),
            private_key: PrivateKey::from_hex(&identities[index].private_key_hex()).unwrap(),
            public_key: identities[index].public_key_hex(),
            trusted_peers: PATH
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .map(|(other, id)| PeerConfig {
                    node_id: (*id).into(),
                    public_key: identities[other].public_key_hex(),
                    transports: vec![
                        TransportKind::NativeUdp,
                        TransportKind::NativeTcp,
                        TransportKind::Vless,
                    ],
                })
                .collect(),
            allowed_paths: vec![path(), path().into_iter().rev().collect()],
            listeners: Vec::new(),
            outgoing: Vec::new(),
            limits: crate::Limits {
                probe_timeout_ms: 1_500,
                handshake_timeout_ms: 500,
                reconnect_delay_ms: 100,
                ..Default::default()
            },
        })
        .collect()
}

struct Fixture {
    nyc: Runtime,
    lax: Runtime,
    sha: Runtime,
}

impl Fixture {
    async fn native(transport: TransportKind) -> Self {
        let mut configs = configs();
        configs[1].listeners.push(ListenerConfig {
            transport,
            bind: "127.0.0.1:0".parse().unwrap(),
            id: None,
            allowed_peers: None,
        });
        let lax = Runtime::start(configs[1].clone()).await.unwrap();
        let endpoint = lax.bound_listeners()[0].address.to_string();
        for index in [0, 2] {
            configs[index].outgoing.push(OutgoingCandidate {
                peer: "lax".into(),
                transport,
                endpoint: endpoint.clone(),
                priority: 0,
                id: None,
            });
        }
        let nyc = Runtime::start(configs[0].clone()).await.unwrap();
        let sha = Runtime::start(configs[2].clone()).await.unwrap();
        for runtime in [&nyc, &sha] {
            runtime
                .wait_for_peer("lax", Duration::from_secs(3))
                .await
                .unwrap();
        }
        for peer in ["nyc", "sha"] {
            lax.wait_for_peer(peer, Duration::from_secs(3))
                .await
                .unwrap();
        }
        Self { nyc, lax, sha }
    }

    async fn shutdown(self) {
        tokio::join!(
            self.nyc.shutdown(),
            self.lax.shutdown(),
            self.sha.shutdown()
        );
        assert!(lock(&self.nyc.handle.inner.handles).is_empty());
        assert!(lock(&self.lax.handle.inner.handles).is_empty());
        assert!(lock(&self.sha.handle.inner.handles).is_empty());
    }

    async fn exercise(&self, transport: TransportKind) {
        let secret = b"plaintext-must-not-exist-at-lax-0123456789".to_vec();
        assert_eq!(
            self.nyc
                .probe(path(), secret.clone())
                .await
                .unwrap()
                .payload,
            secret
        );
        assert_eq!(
            self.sha
                .probe(path().into_iter().rev().collect(), vec![42; 1024])
                .await
                .unwrap()
                .payload,
            vec![42; 1024]
        );
        assert!(
            self.nyc
                .probe(path(), Vec::new())
                .await
                .unwrap()
                .payload
                .is_empty()
        );
        let observed = lock(&self.lax.handle.inner.observed_transit);
        assert!(observed.len() >= 12);
        assert!(
            observed
                .iter()
                .all(|body| !body.windows(secret.len()).any(|window| window == secret))
        );
        assert_eq!(self.lax.peer_transport("sha"), Some(transport));
        assert_eq!(self.nyc.peer_transport("sha"), None);
    }
}

#[tokio::test]
async fn real_udp_three_nodes_reverse_accepted_channel_and_ciphertext() {
    let fixture = Fixture::native(TransportKind::NativeUdp).await;
    fixture.exercise(TransportKind::NativeUdp).await;
    assert!(fixture.lax.statistics().native_udp.forwarded >= 12);
    // Native UDP is datagram-sized, not tunneled through the stream adapter.
    assert_eq!(fixture.lax.statistics().native_tcp.received, 0);
    fixture.shutdown().await;
}

#[tokio::test]
async fn real_tcp_three_nodes_reverse_accepted_channel_and_ciphertext() {
    let fixture = Fixture::native(TransportKind::NativeTcp).await;
    fixture.exercise(TransportKind::NativeTcp).await;
    assert!(fixture.lax.statistics().native_tcp.forwarded >= 12);
    fixture.shutdown().await;
}

struct Tap<S> {
    stream: S,
    writes: Arc<Mutex<Vec<u8>>>,
}

impl<S: AsyncRead + Unpin> AsyncRead for Tap<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Tap<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match Pin::new(&mut self.stream).poll_write(cx, bytes) {
            Poll::Ready(Ok(size)) => {
                lock(&self.writes).extend_from_slice(&bytes[..size]);
                Poll::Ready(Ok(size))
            }
            result => result,
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

async fn attach_pair(dialer: &Runtime, acceptor: &Runtime, priority: u16) -> Arc<Mutex<Vec<u8>>> {
    // Tiny duplex capacity also exercises length/body partial reads under concurrent writes.
    let (a, b) = tokio::io::duplex(7);
    let writes = Arc::new(Mutex::new(Vec::new()));
    let a = Tap {
        stream: a,
        writes: writes.clone(),
    };
    let b = Tap {
        stream: b,
        writes: writes.clone(),
    };
    let (a, b) = tokio::join!(
        dialer.attach_stream_with_priority(
            a,
            StreamRole::Dialer {
                peer: acceptor.node_id().to_owned()
            },
            TransportKind::NativeTcp,
            priority
        ),
        acceptor.attach_stream(b, StreamRole::Acceptor, TransportKind::NativeTcp),
    );
    a.unwrap();
    b.unwrap();
    writes
}

#[tokio::test]
async fn duplex_stream_attachment_is_bidirectional_and_wire_has_no_plaintext() {
    let configs = configs();
    let nyc = Runtime::start(configs[0].clone()).await.unwrap();
    let lax = Runtime::start(configs[1].clone()).await.unwrap();
    let sha = Runtime::start(configs[2].clone()).await.unwrap();
    let capture_a = attach_pair(&nyc, &lax, 0).await;
    let capture_b = attach_pair(&sha, &lax, 0).await;
    let fixture = Fixture { nyc, lax, sha };
    fixture.exercise(TransportKind::NativeTcp).await;
    let secret = b"plaintext-must-not-exist-at-lax-0123456789";
    for capture in [capture_a, capture_b] {
        let bytes = lock(&capture);
        assert!(!bytes.is_empty());
        assert!(!bytes.windows(secret.len()).any(|window| window == secret));
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn wrong_hop_unauthorized_paths_and_closed_must_hop_do_not_fallback() {
    let fixture = Fixture::native(TransportKind::NativeTcp).await;
    let invalid = vec!["nyc".into(), "sha".into()];
    assert_eq!(
        fixture.nyc.probe(invalid, b"secret".to_vec()).await.err(),
        Some(Error::UnauthorizedPath)
    );
    assert_eq!(
        fixture.nyc.probe(path(), vec![0; 8193]).await.err(),
        Some(Error::Oversize)
    );
    // SHA is authenticated, but cannot claim to be NYC's predecessor at LAX.
    fixture
        .sha
        .handle
        .inner
        .send(
            "lax",
            Packet {
                phase: Phase::Init,
                id: [9; 16],
                path: path(),
                body: vec![0; 100],
            },
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while fixture.lax.statistics().native_tcp.dropped == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.lax.statistics().native_tcp.forwarded, 0);
    // A direct authenticated NYC–SHA channel still cannot substitute for required LAX.
    attach_pair(&fixture.nyc, &fixture.sha, 0).await;
    fixture.lax.shutdown().await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while fixture.nyc.connected_peers().contains(&"lax".to_owned()) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        fixture.nyc.probe(path(), b"secret".to_vec()).await.err(),
        Some(Error::Unavailable)
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn unknown_identity_wrong_pinned_key_and_stream_oversize_rejected() {
    let mut configs = configs();
    configs[1]
        .trusted_peers
        .retain(|peer| peer.node_id != "nyc");
    configs[1].allowed_paths.clear();
    let nyc = Runtime::start(configs[0].clone()).await.unwrap();
    let lax = Runtime::start(configs[1].clone()).await.unwrap();
    let (a, b) = tokio::io::duplex(1024);
    let (dial, accept) = tokio::join!(
        nyc.attach_stream(
            a,
            StreamRole::Dialer { peer: "lax".into() },
            TransportKind::NativeTcp
        ),
        lax.attach_stream(b, StreamRole::Acceptor, TransportKind::NativeTcp),
    );
    assert!(dial.is_err());
    assert_eq!(accept, Err(Error::Authentication));
    assert!(lax.connected_peers().is_empty());
    nyc.shutdown().await;
    lax.shutdown().await;

    let mut configs = self::configs();
    configs[0]
        .trusted_peers
        .iter_mut()
        .find(|peer| peer.node_id == "lax")
        .unwrap()
        .public_key = Identity::generate().unwrap().public_key_hex();
    let nyc = Runtime::start(configs[0].clone()).await.unwrap();
    let lax = Runtime::start(configs[1].clone()).await.unwrap();
    let (a, b) = tokio::io::duplex(1024);
    let (dial, accept) = tokio::join!(
        nyc.attach_stream(
            a,
            StreamRole::Dialer { peer: "lax".into() },
            TransportKind::NativeTcp
        ),
        lax.attach_stream(b, StreamRole::Acceptor, TransportKind::NativeTcp),
    );
    assert!(dial.is_err());
    assert_eq!(accept, Err(Error::Authentication));
    let (mut malicious, stream) = tokio::io::duplex(16);
    malicious.write_u32(u32::MAX).await.unwrap();
    assert_eq!(
        lax.attach_stream(stream, StreamRole::Acceptor, TransportKind::NativeTcp)
            .await,
        Err(Error::Oversize)
    );
    let (_stall, stream) = tokio::io::duplex(16);
    assert_eq!(
        lax.attach_stream(stream, StreamRole::Acceptor, TransportKind::NativeTcp)
            .await,
        Err(Error::Timeout)
    );
    nyc.shutdown().await;
    lax.shutdown().await;
}

#[test]
fn strict_config_identity_limits_and_debug_redaction() {
    let config = configs().remove(0);
    config.validate().unwrap();
    let private = config.private_key.to_hex();
    assert!(!format!("{config:?}").contains(&private));
    let mut value = serde_json::to_value(&config).unwrap();
    value["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<LabConfig>(value).is_err());
    let mut invalid = config.clone();
    invalid.trusted_peers.push(invalid.trusted_peers[0].clone());
    assert!(invalid.validate().is_err());
    let mut invalid = config.clone();
    invalid.public_key = Identity::generate().unwrap().public_key_hex();
    assert_eq!(
        invalid.validate(),
        Err(Error::InvalidConfig("private/public key mismatch"))
    );
    let mut invalid = config;
    invalid.allowed_paths = vec![vec!["nyc".into(), "lax".into(), "nyc".into()]];
    assert!(invalid.validate().is_err());
    assert_eq!(Batch::new(vec![vec![1]; 17]).err(), Some(Error::Oversize));
    assert_eq!(
        Batch::new(vec![vec![1; MAX_FRAME_SIZE + 1]]).err(),
        Some(Error::Oversize)
    );
}

#[tokio::test]
async fn cached_packet_delivery_batch_ipv6_mtu_and_no_automatic_echo() {
    let fixture = Fixture::native(TransportKind::NativeUdp).await;
    let mut at_sha = fixture.sha.take_packet_receiver().unwrap();
    let mut at_nyc = fixture.nyc.take_packet_receiver().unwrap();
    assert!(fixture.sha.take_packet_receiver().is_err());
    let payload = vec![0x60; 1280];
    assert_eq!(
        native_udp_datagram_len(&path(), payload.len()).unwrap(),
        1393
    );
    fixture
        .nyc
        .send_packet(path(), payload.clone())
        .await
        .unwrap();
    let received = tokio::time::timeout(Duration::from_secs(1), at_sha.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.source, "nyc");
    assert_eq!(received.path, path());
    assert_eq!(received.payload, payload);
    assert!(at_nyc.try_recv().is_err());
    let id = fixture
        .nyc
        .handle
        .inner
        .sending
        .lock()
        .await
        .get(&path())
        .unwrap()
        .id;
    let batch = vec![vec![7; 1280], vec![8; 1280], vec![9; 1280]];
    fixture
        .nyc
        .send_packet_batch(path(), batch.clone())
        .await
        .unwrap();
    for expected in batch {
        let received = tokio::time::timeout(Duration::from_secs(1), at_sha.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.payload, expected);
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
        id
    );
    let reversed: Vec<_> = path().into_iter().rev().collect();
    fixture
        .sha
        .send_packet(reversed.clone(), vec![4; 1280])
        .await
        .unwrap();
    let received = tokio::time::timeout(Duration::from_secs(1), at_nyc.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.source, "sha");
    assert_eq!(received.path, reversed);
    assert_eq!(received.payload, vec![4; 1280]);
    {
        let observed = lock(&fixture.lax.handle.inner.observed_transit);
        assert!(
            observed
                .iter()
                .all(|body| !body.windows(32).any(|window| window == [0x60; 32]))
        );
    }
    // Oversized batches are rejected before unbounded allocations/tasks are created.
    assert_eq!(
        fixture
            .nyc
            .send_packet_batch(path(), vec![vec![1]; 17])
            .await,
        Err(Error::Oversize)
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn accepted_channel_does_not_grant_reverse_initiation() {
    let mut configs = configs();
    for config in &mut configs {
        config.allowed_paths = vec![path()];
    }
    let nyc = Runtime::start(configs[0].clone()).await.unwrap();
    let lax = Runtime::start(configs[1].clone()).await.unwrap();
    let sha = Runtime::start(configs[2].clone()).await.unwrap();
    attach_pair(&nyc, &lax, 0).await;
    attach_pair(&sha, &lax, 0).await;
    assert_eq!(
        nyc.probe(path(), b"authorized".to_vec())
            .await
            .unwrap()
            .payload,
        b"authorized"
    );
    let reverse: Vec<_> = path().into_iter().rev().collect();
    assert_eq!(
        sha.probe(reverse.clone(), vec![1]).await.err(),
        Some(Error::UnauthorizedPath)
    );
    assert_eq!(
        sha.send_packet(reverse, vec![1]).await,
        Err(Error::UnauthorizedPath)
    );
    Fixture { nyc, lax, sha }.shutdown().await;
}

#[tokio::test]
async fn native_udp_replay_tamper_oversize_and_unknown_host_are_counted_drops() {
    use crate::crypto::{Cipher, read_handshake, write_handshake};
    let mut configs = configs();
    configs[1].listeners.push(ListenerConfig {
        transport: TransportKind::NativeUdp,
        bind: "127.0.0.1:0".parse().unwrap(),
        id: None,
        allowed_peers: None,
    });
    let lax = Runtime::start(configs[1].clone()).await.unwrap();
    let address = lax.bound_listeners()[0].address;
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let identity = Identity::from_private_key(configs[0].private_key.clone());
    let remote = public_key(&configs[1].public_key).unwrap();
    let mut state = identity
        .handshake(Some(&remote), &wire::hop_prologue(TransportKind::NativeUdp))
        .unwrap();
    let mut first = vec![1];
    first.extend(write_handshake(&mut state, &[1, 0, 0]).unwrap());
    socket.send_to(&first, address).await.unwrap();
    let mut buffer = [0; MAX_FRAME_SIZE];
    let (size, _) = tokio::time::timeout(Duration::from_secs(1), socket.recv_from(&mut buffer))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(buffer[0], 2);
    read_handshake(&mut state, &buffer[1..size]).unwrap();
    let mut cipher = Cipher::finish(state).unwrap();
    security::confirm_udp(&socket, address, &mut cipher).await;
    let mut plaintext = vec![1];
    plaintext.extend(
        Packet {
            phase: Phase::Init,
            id: [5; 16],
            path: path(),
            body: vec![0; 96],
        }
        .encode()
        .unwrap(),
    );
    let make_datagram = |cipher: &mut Cipher| {
        let mut datagram = vec![3];
        datagram.extend_from_slice(&cipher.id);
        datagram.extend(cipher.seal(&plaintext).unwrap());
        datagram
    };
    let one = make_datagram(&mut cipher);
    socket.send_to(&one, address).await.unwrap();
    socket.send_to(&one, address).await.unwrap(); // authenticated replay
    let two = make_datagram(&mut cipher);
    let mut forged = two.clone();
    *forged.last_mut().unwrap() ^= 1;
    socket.send_to(&forged, address).await.unwrap();
    let three = make_datagram(&mut cipher);
    socket.send_to(&three, address).await.unwrap(); // out of order is accepted
    socket.send_to(&two, address).await.unwrap();
    let mut unknown = Identity::generate()
        .unwrap()
        .handshake(Some(&remote), &wire::hop_prologue(TransportKind::NativeUdp))
        .unwrap();
    let mut hello = vec![1];
    hello.extend(write_handshake(&mut unknown, &[1, 0, 0]).unwrap());
    socket.send_to(&hello, address).await.unwrap();
    socket
        .send_to(&vec![0; MAX_NATIVE_UDP_DATAGRAM + 1], address)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while lax.statistics().native_udp.dropped < 7 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(lax.statistics().native_udp.received, 3);
    assert_eq!(lax.connected_peers(), vec!["nyc"]);
    lax.shutdown().await;
}

#[tokio::test]
async fn deterministic_duplicate_channels_prefer_lower_priority() {
    let configs = configs();
    let nyc = Runtime::start(configs[0].clone()).await.unwrap();
    let lax = Runtime::start(configs[1].clone()).await.unwrap();
    attach_pair(&nyc, &lax, 10).await;
    let old = lock(&nyc.handle.inner.peers).get("lax").unwrap().token;
    attach_pair(&lax, &nyc, 1).await;
    let a = lock(&nyc.handle.inner.peers).get("lax").unwrap().clone();
    let b = lock(&lax.handle.inner.peers).get("nyc").unwrap().clone();
    assert_ne!(old, a.token);
    assert_eq!(a.token, b.token);
    assert_eq!(a.priority, 1);
    assert_eq!(b.priority, 1);
    tokio::join!(nyc.shutdown(), lax.shutdown());
}

#[tokio::test]
async fn endpoint_rejects_relay_origin_impersonation_and_e2e_replay() {
    use crate::crypto::write_handshake;
    let fixture = Fixture::native(TransportKind::NativeTcp).await;
    let mut packets = fixture.sha.take_packet_receiver().unwrap();
    let relay = &fixture.lax.handle.inner;
    let mut forged = Packet {
        phase: Phase::Init,
        id: [81; 16],
        path: path(),
        body: Vec::new(),
    };
    let mut forged_state = relay
        .identity
        .handshake(relay.keys.get("sha"), &forged.e2e_prologue())
        .unwrap();
    forged.body = write_handshake(&mut forged_state, b"packet-v1").unwrap();
    relay.send("sha", forged).unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while fixture.sha.statistics().native_tcp.dropped == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(lock(&fixture.sha.handle.inner.endpoints).is_empty());
    assert!(packets.try_recv().is_err());

    fixture
        .nyc
        .send_packet(path(), b"establish".to_vec())
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), packets.recv())
            .await
            .unwrap()
            .unwrap()
            .payload,
        b"establish"
    );
    let (id, ciphertext) = {
        let mut sessions = fixture.nyc.handle.inner.sending.lock().await;
        let session = sessions.get_mut(&path()).unwrap();
        (session.id, session.cipher.seal(b"deliver once").unwrap())
    };
    let drops = fixture.sha.statistics().native_tcp.dropped;
    for _ in 0..2 {
        fixture
            .nyc
            .handle
            .inner
            .send(
                "lax",
                Packet {
                    phase: Phase::Packet,
                    id,
                    path: path(),
                    body: ciphertext.clone(),
                },
            )
            .unwrap();
    }
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), packets.recv())
            .await
            .unwrap()
            .unwrap()
            .payload,
        b"deliver once"
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while fixture.sha.statistics().native_tcp.dropped == drops {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(packets.try_recv().is_err());
    fixture.shutdown().await;
}

#[tokio::test]
async fn carrier_grants_reject_wrong_transport_and_endpoint_only_hops() {
    use crate::crypto::write_handshake;
    let mut configs = configs();
    for config in &mut configs {
        for peer in &mut config.trusted_peers {
            peer.transports = match (config.node_id.as_str(), peer.node_id.as_str()) {
                ("nyc", "lax") | ("lax", "nyc") => vec![TransportKind::NativeUdp],
                ("sha", "lax") | ("lax", "sha") => vec![TransportKind::Vless],
                _ => Vec::new(),
            };
        }
    }
    configs[1].listeners.push(ListenerConfig {
        transport: TransportKind::NativeUdp,
        bind: "127.0.0.1:0".parse().unwrap(),
        id: None,
        allowed_peers: None,
    });
    let lax = Runtime::start(configs[1].clone()).await.unwrap();
    let address = lax.bound_listeners()[0].address;
    configs[0].outgoing.push(OutgoingCandidate {
        peer: "lax".into(),
        transport: TransportKind::NativeUdp,
        endpoint: address.to_string(),
        priority: 0,
        id: None,
    });
    let nyc = Runtime::start(configs[0].clone()).await.unwrap();
    let sha = Runtime::start(configs[2].clone()).await.unwrap();
    nyc.wait_for_peer("lax", Duration::from_secs(1))
        .await
        .unwrap();
    let (a, b) = tokio::io::duplex(1024);
    let (a, b) = tokio::join!(
        sha.attach_stream(
            a,
            StreamRole::Dialer { peer: "lax".into() },
            TransportKind::Vless
        ),
        lax.attach_stream(b, StreamRole::Acceptor, TransportKind::Vless),
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(
        nyc.probe(path(), b"pinned carrier path".to_vec())
            .await
            .unwrap()
            .payload,
        b"pinned carrier path"
    );

    let sha_identity = Identity::from_private_key(configs[2].private_key.clone());
    let lax_key = public_key(&configs[1].public_key).unwrap();
    let mut state = sha_identity
        .handshake(
            Some(&lax_key),
            &wire::hop_prologue(TransportKind::NativeUdp),
        )
        .unwrap();
    let hello = write_handshake(&mut state, &[1, 0, 0]).unwrap();
    let mut datagram = vec![1];
    datagram.extend_from_slice(&hello);
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.send_to(&datagram, address).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while lax.statistics().native_udp.dropped == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(lax.peer_transport("sha"), Some(TransportKind::Vless));

    let mut state = sha_identity
        .handshake(
            Some(&lax_key),
            &wire::hop_prologue(TransportKind::NativeTcp),
        )
        .unwrap();
    let hello = write_handshake(&mut state, &[1, 0, 0]).unwrap();
    let (mut malicious, stream) = tokio::io::duplex(1024);
    crate::framing::write_record(&mut malicious, &hello)
        .await
        .unwrap();
    assert_eq!(
        lax.attach_stream(stream, StreamRole::Acceptor, TransportKind::NativeTcp)
            .await,
        Err(Error::UnauthorizedTransport)
    );
    let nyc_identity = Identity::from_private_key(configs[0].private_key.clone());
    let sha_key = public_key(&configs[2].public_key).unwrap();
    let mut state = nyc_identity
        .handshake(Some(&sha_key), &wire::hop_prologue(TransportKind::Vless))
        .unwrap();
    let hello = write_handshake(&mut state, &[1, 0, 0]).unwrap();
    let (mut malicious, stream) = tokio::io::duplex(1024);
    crate::framing::write_record(&mut malicious, &hello)
        .await
        .unwrap();
    assert_eq!(
        sha.attach_stream(stream, StreamRole::Acceptor, TransportKind::Vless)
            .await,
        Err(Error::UnauthorizedTransport)
    );
    assert_eq!(sha.connected_peers(), vec!["lax"]);
    Fixture { nyc, lax, sha }.shutdown().await;
}

#[tokio::test]
async fn shutdown_and_immediate_drop_close_owned_tasks_and_delivery() {
    let config = configs().remove(0);
    let runtime = Runtime::start(config.clone()).await.unwrap();
    let mut received = runtime.take_packet_receiver().unwrap();
    runtime.shutdown().await;
    assert!(
        tokio::time::timeout(Duration::from_secs(1), received.recv())
            .await
            .unwrap()
            .is_none()
    );
    assert!(lock(&runtime.handle.inner.handles).is_empty());
    let runtime = Runtime::start(config).await.unwrap();
    let weak = Arc::downgrade(&runtime.handle.inner);
    // No await between spawning runtime tasks and dropping the last owner.
    drop(runtime);
    tokio::time::timeout(Duration::from_secs(1), async {
        while weak.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
