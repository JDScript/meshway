//! Receive backpressure: a full Manager event queue leaves business frames in
//! the socket, and control behind them waits at most the stall bound.
use super::*;

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
/// Beyond the queue: one read batch waits in the owner and the rest in the
/// kernel buffer, few enough for an unprivileged default `rmem_max`, where
/// further datagrams would be the kernel's (counted) drop.
const BEYOND_QUEUE: usize = 64;

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
    let sent = QUEUE_SLOTS + BEYOND_QUEUE;
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
    business(&client, local, QUEUE_SLOTS + BEYOND_QUEUE).await;
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
