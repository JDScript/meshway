mod roam;

use super::super::tests::{netmap::support::Netmaps, support::carriers};
use super::*;

const DIALER_BOOT: crate::Boot = [0xd1; 16];
use meshway_transport::udp::UdpTransport;
use tokio::net::UdpSocket;

pub(in crate::manager) fn shared(
    desired: &Desired,
    keys: &DataPlaneKeys,
) -> (Arc<Shared>, super::super::events::Receiver) {
    let plan = Plan::new(desired).unwrap();
    let generation = meshway_model::Sequence::MIN.checked_next().unwrap();
    let (events, receiver) = super::super::events::channel(64, 8 * 1024 * 1024);
    let interests = Arc::new(super::super::interest::Interests::new());
    interests.allow(&plan);
    let shared = Arc::new(Shared {
        keys: keys.clone(),
        plans: tokio::sync::watch::channel(plan).0,
        stop: CancellationToken::new(),
        links: Arc::new(Mutex::new(LinkTable::default())),
        events: events.clone(),
        control_events: events,
        capacity: Arc::new(Semaphore::new(64)),
        limits: Limits::default(),
        control_budget: Arc::new(Mutex::new(ControlBudget::new())),
        sockets: Arc::new(Mutex::new(BTreeMap::new())),
        stun_confirmed: Arc::default(),
        stun_samples: Arc::default(),
        path_mtu: Arc::default(),
        size_probes: Arc::default(),
        observation_request: Arc::default(),
        generation,
        traversal_endpoints: Arc::new(Mutex::new(BTreeMap::new())),
        resource_changes: tokio::sync::watch::channel(()).0,
        interests,
        facts: Default::default(),
        instance: Default::default(),
        boot: crate::adjacency::boot().unwrap(),
        unbound: Arc::default(),
        roams: Arc::new(tokio::sync::watch::channel(0).0),
        roam_observation: Arc::default(),
    });
    (shared, receiver)
}

/// Bound on every control datagram these tests wait for.
const CONTROL_WAIT: Duration = Duration::from_secs(5);

/// An owner endpoint whose write readiness is already observed, so each
/// control send completes inline within the owner call that makes it. A
/// socket bound just now has none yet: its first send would wait for the
/// Session worker, and a runtime stalled beyond [`OUTPUT_WAIT`] drops that
/// best-effort datagram (as the network may). A real dialer retransmits;
/// these tests drive the owner directly and send each message once.
async fn endpoint(socket: UdpSocket) -> UdpTransport {
    socket.writable().await.unwrap();
    UdpTransport::adopt(socket, &Default::default()).unwrap()
}

/// Node 1 (`shared`) accepts node 0's real Native UDP handshake: a
/// selected channel toward node 0 is in its link table while the
/// returned owner lives.
pub(in crate::manager) async fn accepted_link(
    fixture: &mut Netmaps,
    shared: &Arc<Shared>,
) -> Box<dyn std::any::Any + Send> {
    let ids = fixture.ids();
    let socket = endpoint(fixture.take_udp(1)).await;
    let local = socket.local_addr().unwrap();
    let id = adjacency::nonce().unwrap();
    let mut owner = Owner::new(shared.clone(), Box::new(socket), id).unwrap();
    owner.listener = Some(ListenerFact {
        socket_id: id,
        key: "udp".into(),
        protocol: TransportProtocol::NativeUdp,
        local,
        buffers: None,
    });
    let dialer = Plan::new(&fixture.desired[0]).unwrap();
    let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
    let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let (initiation, intro) =
        Initiation::new(&fixture.keys[0], &dialer, &carrier, &DIALER_BOOT).unwrap();
    owner
        .init(peer.local_addr().unwrap(), &intro)
        .await
        .unwrap();
    let mut bytes = [0; adjacency::MAX_MESSAGE];
    let (len, _) = control(&peer, &mut bytes).await;
    let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
    let transcript = initiation.challenge(&dialer, ids[1], challenge).unwrap();
    let ack = transcript
        .message(&fixture.keys[0], &dialer, Kind::Ack)
        .unwrap();
    owner.ack(peer.local_addr().unwrap(), &ack).await.unwrap();
    assert!(shared.links.lock().unwrap().contains_key(&ids[0]));
    Box::new(owner)
}

/// The next datagram `client` receives, within [`CONTROL_WAIT`].
async fn control(client: &UdpSocket, bytes: &mut [u8]) -> (usize, SocketAddr) {
    tokio::time::timeout(CONTROL_WAIT, client.recv_from(bytes))
        .await
        .expect("a control datagram within CONTROL_WAIT")
        .unwrap()
}

#[tokio::test]
async fn replay_exhaustion_rebuilds_instead_of_forgetting_nonces_and_old_ack_cannot_install() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let (shared, _events) = shared(&fixture.desired[1], &fixture.keys[1]);
    let socket = endpoint(fixture.take_udp(1)).await;
    let local = socket.local_addr().unwrap();
    let id = adjacency::nonce().unwrap();
    let listener = ListenerFact {
        socket_id: id,
        key: "udp".into(),
        protocol: TransportProtocol::NativeUdp,
        local,
        buffers: None,
    };
    shared
        .register_socket(
            id,
            meshway_model::state::ResourceRef::Listener { key: "udp".into() },
            TransportProtocol::NativeUdp,
            local,
        )
        .unwrap();
    let mut owner = Owner::new(shared.clone(), Box::new(socket), id).unwrap();
    owner.listener = Some(listener.clone());
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let source = client.local_addr().unwrap();
    let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
    let (initiation, intro) = Initiation::new(
        &fixture.keys[0],
        &Plan::new(&fixture.desired[0]).unwrap(),
        &carrier,
        &DIALER_BOOT,
    )
    .unwrap();
    owner.init(source, &intro).await.unwrap();
    let mut bytes = [0; adjacency::MAX_MESSAGE];
    let (len, from) = control(&client, &mut bytes).await;
    assert_eq!(from, local);
    let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
    let old = initiation
        .challenge(&Plan::new(&fixture.desired[0]).unwrap(), ids[1], challenge)
        .unwrap();
    let old_ack = old
        .message(
            &fixture.keys[0],
            &Plan::new(&fixture.desired[0]).unwrap(),
            Kind::Ack,
        )
        .unwrap();
    owner.ack(source, &old_ack).await.unwrap();
    let (len, _) = control(&client, &mut bytes).await;
    assert_eq!(adjacency::decode(&bytes[..len]).unwrap().0, Kind::Ready);
    assert_eq!(owner.channels[&source].fact.id, old.id());

    // Seed only already-retired nonce metadata to exercise the production
    // capacity boundary without thousands of redundant crypto exchanges.
    // The triggering packets below still require real registered signatures.
    while owner.replay.len() < MAX_REPLAY {
        owner.replay.insert((ids[0], adjacency::nonce().unwrap()));
    }
    let history = owner.replay.clone();
    // New content keeping every identity and carrier: the Manager's apply
    // keeps this channel; the owner only drops in-flight acquisition.
    let next = Plan::new(&fixture.reserial(1).await).unwrap();
    let previous = shared.plan();
    shared.plans.send_replace(next.clone());
    shared.retain_table(&previous, &next).unwrap();
    owner.reconfigured().unwrap();
    let live = owner.channels[&source].scope.clone();
    assert_eq!(
        owner.replay, history,
        "renewal must not forget replay state"
    );
    let mut invalid = intro.clone();
    *invalid.last_mut().unwrap() ^= 1;
    owner.init(source, &invalid).await.unwrap();
    owner.init(source, &intro).await.unwrap();
    live.check().unwrap();
    assert!(
        !shared.stop.is_cancelled(),
        "invalid/replayed traffic cannot force rebuilding"
    );
    let (rebuild_init, fresh_intro) = Initiation::new(
        &fixture.keys[0],
        &Plan::new(&fixture.desired[0]).unwrap(),
        &carrier,
        &DIALER_BOOT,
    )
    .unwrap();
    owner.init(source, &fresh_intro).await.unwrap();
    live.check().unwrap();
    assert!(
        !shared.stop.is_cancelled(),
        "an INIT alone cannot retire channels"
    );
    let (len, _) = control(&client, &mut bytes).await;
    let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
    let rebuild = rebuild_init
        .challenge(&Plan::new(&fixture.desired[0]).unwrap(), ids[1], challenge)
        .unwrap();
    let expired_ack = rebuild
        .message(
            &fixture.keys[0],
            &Plan::new(&fixture.desired[0]).unwrap(),
            Kind::Ack,
        )
        .unwrap();
    owner
        .pending
        .remove(&source)
        .unwrap()
        .scope
        .cancellation
        .cancel();
    owner.init(source, &fresh_intro).await.unwrap();
    let (len, _) = control(&client, &mut bytes).await;
    let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
    let retried = rebuild_init
        .challenge(&Plan::new(&fixture.desired[0]).unwrap(), ids[1], challenge)
        .unwrap();
    assert_ne!(
        retried.id(),
        rebuild.id(),
        "retrying an unrecorded capacity slot must use a fresh server nonce"
    );
    owner.ack(source, &expired_ack).await.unwrap();
    assert!(!shared.stop.is_cancelled());
    owner.ack(source, &old_ack).await.unwrap();
    assert!(!shared.stop.is_cancelled());
    let ack = retried
        .message(
            &fixture.keys[0],
            &Plan::new(&fixture.desired[0]).unwrap(),
            Kind::Ack,
        )
        .unwrap();
    let impostor = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    owner
        .ack(impostor.local_addr().unwrap(), &ack)
        .await
        .unwrap();
    assert!(!shared.stop.is_cancelled());
    live.check().unwrap();
    assert!(matches!(
        owner.ack(source, &ack).await,
        Err(Error::Capacity)
    ));
    assert!(live.check().is_err());
    assert!(
        shared.stop.is_cancelled(),
        "the Manager recv boundary must surface Closed"
    );
    assert_eq!(owner.replay, history);
    owner.shutdown().await.unwrap();
    drop(owner);

    // This is a new physical owner, the same path as a host reinstall. An
    // old INIT is harmless only because a fresh source challenge is required.
    let (restarted, _events) = self::shared(&fixture.desired[1], &fixture.keys[1]);
    let socket = endpoint(UdpSocket::bind(local).await.unwrap()).await;
    let socket_id = adjacency::nonce().unwrap();
    let mut owner = Owner::new(restarted.clone(), Box::new(socket), socket_id).unwrap();
    owner.listener = Some(ListenerFact {
        socket_id,
        ..listener
    });
    owner.init(source, &intro).await.unwrap();
    let (len, from) = control(&client, &mut bytes).await;
    assert_eq!(from, local);
    let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
    let fresh = initiation
        .challenge(&Plan::new(&fixture.desired[0]).unwrap(), ids[1], challenge)
        .unwrap();
    assert_ne!(fresh.id(), old.id());
    owner.ack(source, &old_ack).await.unwrap();
    assert!(owner.channels.is_empty());
    let fresh_ack = fresh
        .message(
            &fixture.keys[0],
            &Plan::new(&fixture.desired[0]).unwrap(),
            Kind::Ack,
        )
        .unwrap();
    let impostor = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    owner
        .ack(impostor.local_addr().unwrap(), &fresh_ack)
        .await
        .unwrap();
    assert!(owner.channels.is_empty());
    owner.ack(source, &fresh_ack).await.unwrap();
    assert_eq!(restarted.links.lock().unwrap()[&ids[0]].fact.id, fresh.id());
    // The dialer's state survived; the listener restarted. Each side
    // reads the other's signed boot from the fresh transcript.
    assert_eq!(
        restarted.links.lock().unwrap()[&ids[0]].fact.peer_boot,
        DIALER_BOOT
    );
    assert_eq!(fresh.peer_boot(true), restarted.boot);
    assert_ne!(fresh.peer_boot(true), old.peer_boot(true));
    let fresh_ready = fresh
        .message(&restarted.keys, &restarted.plan(), Kind::Ready)
        .unwrap();
    assert!(
        old.verify(
            &Plan::new(&fixture.desired[0]).unwrap(),
            ids[1],
            Kind::Ready,
            &fresh_ready
        )
        .is_err()
    );
    owner.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn server_source_challenge_replay_and_confirmation_use_real_registered_keys() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let (shared, _receiver) = shared(&fixture.desired[1], &fixture.keys[1]);
    let socket = endpoint(fixture.take_udp(1)).await;
    let local = socket.local_addr().unwrap();
    let id = adjacency::nonce().unwrap();
    let mut owner = Owner::new(shared.clone(), Box::new(socket), id).unwrap();
    owner.listener = Some(ListenerFact {
        socket_id: id,
        key: "udp".into(),
        protocol: TransportProtocol::NativeUdp,
        local,
        buffers: None,
    });
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let source = client.local_addr().unwrap();
    let impostor = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let wrong_source = impostor.local_addr().unwrap();
    let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
    let (initiation, intro) = Initiation::new(
        &fixture.keys[0],
        &Plan::new(&fixture.desired[0]).unwrap(),
        &carrier,
        &DIALER_BOOT,
    )
    .unwrap();
    let mut invalid = intro.clone();
    *invalid.last_mut().unwrap() ^= 1;
    owner.init(wrong_source, &invalid).await.unwrap();
    assert!(owner.pending.is_empty());
    owner.init(source, &intro).await.unwrap();
    let expected_id = owner.pending[&source].transcript.id();
    owner.pending.get_mut(&source).unwrap().responses = Retry::due();
    owner.init(source, &intro).await.unwrap();
    assert_eq!(owner.pending[&source].transcript.id(), expected_id);
    owner.init(wrong_source, &intro).await.unwrap();
    assert_eq!(owner.pending.len(), 1);
    let mut bytes = [0; adjacency::MAX_MESSAGE];
    let (len, actual) = control(&client, &mut bytes).await;
    assert_eq!(actual, local);
    let (kind, body) = adjacency::decode(&bytes[..len]).unwrap();
    assert_eq!(kind, Kind::Challenge);
    let transcript = initiation
        .challenge(&Plan::new(&fixture.desired[0]).unwrap(), ids[1], body)
        .unwrap();
    let ack = transcript
        .message(
            &fixture.keys[0],
            &Plan::new(&fixture.desired[0]).unwrap(),
            Kind::Ack,
        )
        .unwrap();
    owner.ack(wrong_source, &ack).await.unwrap();
    assert!(owner.channels.is_empty());
    let wrong_peer = transcript
        .message(
            &fixture.keys[2],
            &Plan::new(&fixture.desired[2]).unwrap(),
            Kind::Ack,
        )
        .unwrap();
    owner.ack(source, &wrong_peer).await.unwrap();
    assert!(owner.channels.is_empty());
    owner.ack(source, &ack).await.unwrap();
    assert_eq!(owner.channels[&source].fact.id, transcript.id());
    assert_eq!(shared.links.lock().unwrap().len(), 1);
    let ready = transcript
        .message(
            &fixture.keys[1],
            &Plan::new(&fixture.desired[1]).unwrap(),
            Kind::Ready,
        )
        .unwrap();
    let plan = Plan::new(&fixture.desired[0]).unwrap();
    let cfg = &*plan;
    transcript.verify(cfg, ids[1], Kind::Ready, &ready).unwrap();
    assert!(
        transcript
            .verify(cfg, ids[1], Kind::Challenge, &ready)
            .is_err()
    );
    let nonce = adjacency::nonce().unwrap();
    let pong = transcript
        .health(
            &fixture.keys[1],
            &Plan::new(&fixture.desired[1]).unwrap(),
            Kind::Pong,
            &nonce,
        )
        .unwrap();
    assert_eq!(
        transcript
            .verify_health(cfg, ids[1], Kind::Pong, &pong)
            .unwrap(),
        nonce
    );
    assert!(
        transcript
            .verify_health(cfg, ids[1], Kind::Ping, &pong)
            .is_err()
    );
    // A fresh challenge changes channel identity; old ACK/health proofs fail.
    let (_, _, fresh) = adjacency::incoming(&shared.plan(), &[(ids[0], carrier)], &intro).unwrap();
    let fresh = fresh.respond(&DIALER_BOOT).unwrap();
    assert!(
        fresh
            .verify(&shared.plan(), ids[0], Kind::Ack, &ack)
            .is_err()
    );
    assert!(fresh.verify_health(cfg, ids[1], Kind::Pong, &pong).is_err());
    let peer_pong = transcript
        .health(
            &fixture.keys[0],
            &Plan::new(&fixture.desired[0]).unwrap(),
            Kind::Pong,
            &nonce,
        )
        .unwrap();
    let before = *owner.channels[&source].health.lock().unwrap();
    owner.health(source, Kind::Pong, &peer_pong).await.unwrap();
    assert_eq!(*owner.channels[&source].health.lock().unwrap(), before);
    owner.channels.get_mut(&source).unwrap().probe = Some(nonce);
    owner
        .health(wrong_source, Kind::Pong, &peer_pong)
        .await
        .unwrap();
    assert_eq!(*owner.channels[&source].health.lock().unwrap(), before);
    owner.health(source, Kind::Pong, &peer_pong).await.unwrap();
    let renewed = *owner.channels[&source].health.lock().unwrap();
    assert!(renewed >= before);
    assert!(owner.channels[&source].probe.is_none());
    owner.health(source, Kind::Pong, &peer_pong).await.unwrap();
    assert_eq!(*owner.channels[&source].health.lock().unwrap(), renewed);
    // Hold the consumer idle until its business event queue is saturated.
    // An authenticated Pong at the tail of a hot batch must still renew the
    // exact channel, rather than being dropped or waiting on that consumer.
    for _ in 0..8 {
        let nonce = adjacency::nonce().unwrap();
        owner.channels.get_mut(&source).unwrap().probe = Some(nonce);
        let body = transcript
            .health(
                &fixture.keys[0],
                &Plan::new(&fixture.desired[0]).unwrap(),
                Kind::Pong,
                &nonce,
            )
            .unwrap();
        let mut frames = (0..31)
            .map(|_| Frame::new(vec![7; 1200], Some(source)))
            .collect::<Vec<_>>();
        frames.push(Frame::new(
            adjacency::encode(Kind::Pong, &body).unwrap(),
            Some(source),
        ));
        owner
            .receive_batch(frames, std::time::Instant::now())
            .await
            .unwrap();
        assert!(owner.channels[&source].probe.is_none());
        assert_eq!(owner.channels[&source].fact.id, transcript.id());
        owner.channels[&source].scope.check().unwrap();
    }
    assert!(
        owner.backlog.blocked() && shared.events.capacity() < 31,
        "business queue really saturated"
    );
    owner.shutdown().await.unwrap();
    fixture.close().await;
}

/// Node 1's owner on a real UDP socket with a selected channel toward node
/// 0's `client`, accepted through the real handshake; the owner is not yet
/// running.
async fn established(
    fixture: &mut Netmaps,
    shared: &Arc<Shared>,
) -> (Owner, UdpSocket, SocketAddr, Transcript) {
    let ids = fixture.ids();
    let socket = endpoint(fixture.take_udp(1)).await;
    let local = socket.local_addr().unwrap();
    let id = adjacency::nonce().unwrap();
    let mut owner = Owner::new(shared.clone(), Box::new(socket), id).unwrap();
    owner.listener = Some(ListenerFact {
        socket_id: id,
        key: "udp".into(),
        protocol: TransportProtocol::NativeUdp,
        local,
        buffers: None,
    });
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let source = client.local_addr().unwrap();
    let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
    let dialer = Plan::new(&fixture.desired[0]).unwrap();
    let (initiation, intro) =
        Initiation::new(&fixture.keys[0], &dialer, &carrier, &DIALER_BOOT).unwrap();
    owner.init(source, &intro).await.unwrap();
    let mut bytes = [0; adjacency::MAX_MESSAGE];
    let (len, _) = control(&client, &mut bytes).await;
    let (_, challenge) = adjacency::decode(&bytes[..len]).unwrap();
    let transcript = initiation.challenge(&dialer, ids[1], challenge).unwrap();
    let ack = transcript
        .message(&fixture.keys[0], &dialer, Kind::Ack)
        .unwrap();
    owner.ack(source, &ack).await.unwrap();
    assert!(owner.selected(&owner.channels[&source]));
    (owner, client, local, transcript)
}

/// Business frames at most this many slots ahead are what the idle
/// consumer's queue holds; the rest have to wait somewhere.
const QUEUE_SLOTS: usize = 64;

async fn business(client: &UdpSocket, local: SocketAddr, count: usize) {
    for _ in 0..count {
        client.send_to(&[7; 1200], local).await.unwrap();
    }
}

/// While the Manager event queue has no room, the receive owner does not
/// read its socket: business frames wait in the kernel receive buffer (whose
/// overflow the kernel counts) instead of being read and dropped unseen.
/// Once the consumer makes room, every one arrives.
#[tokio::test]
async fn a_full_event_queue_keeps_business_frames_in_the_socket_until_the_consumer_makes_room() {
    let mut fixture = Netmaps::start(false, true).await;
    let (shared, mut consumer) = shared(&fixture.desired[1], &fixture.keys[1]);
    let (mut owner, client, local, _) = established(&mut fixture, &shared).await;
    let running = tokio::spawn(async move {
        let result = owner.run().await;
        (owner, result)
    });
    let sent = QUEUE_SLOTS * 3;
    business(&client, local, sent).await;
    tokio::time::timeout(CONTROL_WAIT, async {
        while shared.events.capacity() >= 32 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the idle consumer's queue fills");
    // Shorter than RECEIVE_STALL: an unread backlog, not a stalled consumer.
    tokio::time::sleep(RECEIVE_STALL / 2).await;
    let mut received = 0;
    let delivered = tokio::time::timeout(CONTROL_WAIT, async {
        while received < sent {
            let mut events = Vec::new();
            consumer.recv_many(&mut events, 32).await;
            received += events
                .iter()
                .filter(|event| matches!(event, Event::Frame { .. }))
                .count();
        }
    })
    .await;
    assert!(
        delivered.is_ok(),
        "{received} of {sent} business frames delivered"
    );
    running.abort();
    if let Ok((mut owner, _)) = running.await {
        owner.shutdown().await.unwrap();
    }
    fixture.close().await;
}

/// Behind business frames the full queue keeps in the socket, a peer's
/// health Ping is still answered within RECEIVE_STALL (plus scheduling)
/// although the consumer never makes room: past that bound the owner reads
/// again and drops the business frames that have no room.
#[tokio::test]
async fn control_behind_a_stalled_consumer_is_answered_within_the_stall_bound() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let (shared, _consumer) = shared(&fixture.desired[1], &fixture.keys[1]);
    let (mut owner, client, local, transcript) = established(&mut fixture, &shared).await;
    let running = tokio::spawn(async move {
        let result = owner.run().await;
        (owner, result)
    });
    // Past the owner's Ping answer pacing.
    tokio::time::sleep(RETRY * 2).await;
    business(&client, local, QUEUE_SLOTS * 3).await;
    let nonce = adjacency::nonce().unwrap();
    let dialer = Plan::new(&fixture.desired[0]).unwrap();
    let ping = adjacency::encode(
        Kind::Ping,
        &transcript
            .health(&fixture.keys[0], &dialer, Kind::Ping, &nonce)
            .unwrap(),
    )
    .unwrap();
    let sent = Instant::now();
    client.send_to(&ping, local).await.unwrap();
    let mut bytes = [0; adjacency::MAX_PROBE];
    loop {
        let (len, _) = control(&client, &mut bytes).await;
        let Ok((Kind::Pong, body)) = adjacency::decode(&bytes[..len]) else {
            continue;
        };
        if transcript
            .verify_health(&dialer, ids[1], Kind::Pong, body)
            .is_ok_and(|answered| answered == nonce)
        {
            break;
        }
    }
    let waited = sent.elapsed();
    assert!(
        waited < RECEIVE_STALL * 3,
        "Ping answered after {waited:?}, beyond the stall bound"
    );
    assert!(shared.events.capacity() < 32, "the consumer stayed idle");
    running.abort();
    if let Ok((mut owner, _)) = running.await {
        owner.shutdown().await.unwrap();
    }
    fixture.close().await;
}

/// A real UDP endpoint whose route claims jumbo frames.
struct Jumbo<T>(T);

impl<T: Datagram> Datagram for Jumbo<T> {
    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.0.local_addr()
    }
    fn poll_send_to(
        &self,
        cx: &mut std::task::Context<'_>,
        payload: &[u8],
        peer: SocketAddr,
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.0.poll_send_to(cx, payload, peer)
    }
    fn poll_recv_from(
        &self,
        cx: &mut std::task::Context<'_>,
        buffer: &mut [u8],
    ) -> std::task::Poll<std::io::Result<meshway_transport::raw::DatagramRead>> {
        self.0.poll_recv_from(cx, buffer)
    }
    fn path_mtu(&self, _peer: SocketAddr) -> Option<u16> {
        Some(9000)
    }
}

/// Route MTU 9000: the owner's size probe stays within the codec (the
/// search is capped at the largest encodable probe) and the owner keeps
/// running. A peer's probe is answered only at the length it was signed
/// for: a truncated copy gets no Pong.
#[tokio::test]
async fn size_probes_stay_encodable_on_a_jumbo_route_and_bind_their_length() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let (shared, _receiver) = shared(&fixture.desired[1], &fixture.keys[1]);
    let socket = endpoint(fixture.take_udp(1)).await;
    let local = socket.local_addr().unwrap();
    let id = adjacency::nonce().unwrap();
    let mut owner = Owner::new(shared.clone(), Box::new(Jumbo(socket)), id).unwrap();
    owner.listener = Some(ListenerFact {
        socket_id: id,
        key: "udp".into(),
        protocol: TransportProtocol::NativeUdp,
        local,
        buffers: None,
    });
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let source = client.local_addr().unwrap();
    let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
    let dialer = Plan::new(&fixture.desired[0]).unwrap();
    let (initiation, intro) =
        Initiation::new(&fixture.keys[0], &dialer, &carrier, &DIALER_BOOT).unwrap();
    owner.init(source, &intro).await.unwrap();
    let mut bytes = vec![0; 65_536];
    let (len, _) = control(&client, &mut bytes).await;
    let (kind, body) = adjacency::decode(&bytes[..len]).unwrap();
    assert_eq!(kind, Kind::Challenge);
    let transcript = initiation.challenge(&dialer, ids[1], body).unwrap();
    let ack = transcript
        .message(&fixture.keys[0], &dialer, Kind::Ack)
        .unwrap();
    owner.ack(source, &ack).await.unwrap();
    assert!(owner.channels.contains_key(&source));
    owner.tick().await.unwrap();
    let probe = loop {
        let (len, _) = control(&client, &mut bytes).await;
        if adjacency::decode(&bytes[..len]).unwrap().0 == Kind::Probe {
            break bytes[..len].to_vec();
        }
    };
    assert_eq!(probe.len(), adjacency::MAX_PROBE);
    let (_, body) = adjacency::decode(&probe).unwrap();
    let owner_plan = Plan::new(&fixture.desired[1]).unwrap();
    assert!(
        transcript
            .verify_probe(&dialer, ids[1], probe.len(), body)
            .is_ok()
    );
    assert!(
        transcript
            .verify_probe(&dialer, ids[1], 1300, body)
            .is_err()
    );
    // The peer's probe: truncated, then whole.
    let nonce = adjacency::nonce().unwrap();
    let signed = transcript
        .probe(&fixture.keys[0], &dialer, &nonce, 1300)
        .unwrap();
    assert!(
        transcript
            .probe(&fixture.keys[0], &dialer, &nonce, 9000 - 28)
            .is_err()
    );
    let (_, body) = adjacency::decode(&signed[..1200]).unwrap();
    let before = owner.channels[&source].last_probe_response;
    owner.answer_size_probe(source, 1200, body).await.unwrap();
    assert_eq!(
        owner.channels[&source].last_probe_response, before,
        "a truncated probe is not answered"
    );
    let (_, body) = adjacency::decode(&signed).unwrap();
    owner
        .answer_size_probe(source, signed.len(), body)
        .await
        .unwrap();
    let pong = loop {
        let (len, _) = control(&client, &mut bytes).await;
        let (kind, body) = adjacency::decode(&bytes[..len]).unwrap();
        if kind == Kind::Pong {
            break transcript
                .verify_health(&owner_plan, ids[1], Kind::Pong, body)
                .unwrap();
        }
    };
    assert_eq!(pong, nonce, "only the whole probe is answered");
    owner.shutdown().await.unwrap();
    fixture.close().await;
}

/// A real UDP endpoint whose sends stay pending while `stalled`.
struct Stalled<T> {
    inner: T,
    stalled: Arc<std::sync::atomic::AtomicBool>,
}

impl<T: Datagram> Datagram for Stalled<T> {
    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.inner.local_addr()
    }
    fn poll_send_to(
        &self,
        cx: &mut std::task::Context<'_>,
        payload: &[u8],
        peer: SocketAddr,
    ) -> std::task::Poll<std::io::Result<usize>> {
        if self.stalled.load(std::sync::atomic::Ordering::SeqCst) {
            return std::task::Poll::Pending;
        }
        self.inner.poll_send_to(cx, payload, peer)
    }
    fn poll_recv_from(
        &self,
        cx: &mut std::task::Context<'_>,
        buffer: &mut [u8],
    ) -> std::task::Poll<std::io::Result<meshway_transport::raw::DatagramRead>> {
        self.inner.poll_recv_from(cx, buffer)
    }
}

/// A control send not completed within [`OUTPUT_WAIT`] is dropped like a
/// lost datagram, and the owner never waits longer: the dialer's repeated
/// INIT is answered with the same stored challenge.
#[tokio::test]
async fn a_challenge_dropped_at_the_output_wait_is_resent_on_the_repeated_init() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let (shared, _receiver) = shared(&fixture.desired[1], &fixture.keys[1]);
    let socket = endpoint(fixture.take_udp(1)).await;
    let local = socket.local_addr().unwrap();
    let stalled = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let id = adjacency::nonce().unwrap();
    let endpoint = Stalled {
        inner: socket,
        stalled: stalled.clone(),
    };
    let mut owner = Owner::new(shared.clone(), Box::new(endpoint), id).unwrap();
    owner.listener = Some(ListenerFact {
        socket_id: id,
        key: "udp".into(),
        protocol: TransportProtocol::NativeUdp,
        local,
        buffers: None,
    });
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let source = client.local_addr().unwrap();
    let carrier = carriers(&fixture.desired[0])[&ids[1]].clone();
    let dialer = Plan::new(&fixture.desired[0]).unwrap();
    let (initiation, intro) =
        Initiation::new(&fixture.keys[0], &dialer, &carrier, &DIALER_BOOT).unwrap();
    let started = Instant::now();
    owner.init(source, &intro).await.unwrap();
    assert!(
        started.elapsed() >= OUTPUT_WAIT,
        "the send waited out its bound"
    );
    let challenge = owner.pending[&source].challenge.clone();
    let mut bytes = [0; adjacency::MAX_MESSAGE];
    assert!(
        tokio::time::timeout(RETRY, client.recv_from(&mut bytes))
            .await
            .is_err(),
        "the dropped challenge never leaves"
    );
    stalled.store(false, std::sync::atomic::Ordering::SeqCst);
    owner.init(source, &intro).await.unwrap();
    let (len, from) = control(&client, &mut bytes).await;
    assert_eq!(from, local);
    assert_eq!(&bytes[..len], challenge.as_slice());
    let (kind, body) = adjacency::decode(&bytes[..len]).unwrap();
    assert_eq!(kind, Kind::Challenge);
    initiation.challenge(&dialer, ids[1], body).unwrap();
    owner.shutdown().await.unwrap();
    fixture.close().await;
}
