use super::*;

#[tokio::test]
async fn receipt_uses_replacement_channel_and_rejects_same_carrier_old_scope_waiting_for_sender() {
    let mut fixture = fixture::Fixture::builder().build().await;
    let ids = fixture.ids();
    let faults = Arc::new(Faults::default());
    let factory = Factory::new(&mut fixture, faults.clone());
    let mut b = fixture.manager(1, factory.clone()).start().await;
    let mut a = fixture.manager(0, factory.clone()).start().await;
    converged(&a, &b, ids[0], ids[1], None).await;
    let (mut sa, mut sb) = (
        secure(&fixture.identities[0], &a),
        secure(&fixture.identities[1], &b),
    );
    let cid = handshake(
        &mut a,
        &mut b,
        &mut sa,
        &mut sb,
        fixture.demands[0],
        vec![ids[0], ids[1]],
    )
    .await;
    exchange(&a, &mut b, &mut sa, &mut sb, cid, packet(&fixture, false)).await;
    let old = b.snapshot().unwrap().links[0].clone();
    let (sender, old_scope) = {
        let links = b.shared.links.lock().unwrap();
        (links[&ids[0]].sender.clone(), links[&ids[0]].scope.clone())
    };
    let pending_before_replacement = sb.send(cid, packet(&fixture, true)).unwrap();
    // Capture another real slot now, but do not poll until after replacement.
    let delayed = b.send_with_receipt(sb.send(cid, packet(&fixture, true)).unwrap());
    a.shutdown().await.unwrap();
    let rebound = UdpSocket::bind(old.remote).await.unwrap();
    let (transcript, _, ack) = challenge(&fixture, &rebound, old.local, "udp").await;

    faults.block_data.store(true, Ordering::SeqCst);
    let mut admitted = Box::pin(b.send_with_receipt(sb.send(cid, packet(&fixture, true)).unwrap()));
    assert!(futures_util::poll!(admitted.as_mut()).is_pending());
    assert!(faults.blocked.load(Ordering::SeqCst));
    // Admission is waiting on the actual shared sender, after selecting the old
    // slot. A fresh authenticated ACK replaces that slot with the same carrier
    // and socket, but a new channel ID and lifetime.
    let locked = sender.lock().await;
    let writes = faults.writes.load(Ordering::SeqCst);
    let mut stale = Box::pin(b.send_with_receipt(sb.send(cid, packet(&fixture, true)).unwrap()));
    assert!(futures_util::poll!(stale.as_mut()).is_pending());
    rebound.send_to(&ack, old.local).await.unwrap();
    selected(&b, ids[0], transcript.id()).await;
    assert!(old_scope.check().is_err());
    assert!(
        tokio::time::timeout(RECOVERY, stale)
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(faults.writes.load(Ordering::SeqCst), writes);
    drop(locked);
    faults.block_data.store(false, Ordering::SeqCst);
    faults.waker.wake();
    // The already-admitted old output stays alive without its caller polling.
    // Its retained channel lifetime must still fence the actual raw writer.
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(faults.writes.load(Ordering::SeqCst), writes);
    assert!(admitted.await.is_err());
    control(&rebound, Kind::Ready).await;
    let mut sends = JoinSet::new();
    sends.spawn(delayed);
    assert!(sends.join_next().await.unwrap().unwrap().is_err());
    assert_eq!(faults.writes.load(Ordering::SeqCst), writes);

    let current = b.snapshot().unwrap().links[0].clone();
    assert_ne!(current.id, old.id);
    assert_eq!(current.carrier, old.carrier);
    assert_eq!(current.socket_id, old.socket_id);
    assert_eq!(current.acquisition, LinkAcquisition::AcceptedInbound);
    let before = std::time::Instant::now();
    let receipt = b
        .send_with_receipt(pending_before_replacement)
        .await
        .unwrap();
    assert_eq!(receipt.link, current);
    assert!(receipt.admitted_at >= before);
    assert!(receipt.admitted_at <= std::time::Instant::now());
    assert_eq!(faults.writes.load(Ordering::SeqCst), writes + 1);
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn copied_init_and_wrong_source_or_old_ack_do_not_replace_same_tuple_until_fresh_ack() {
    let mut fixture = fixture::Fixture::builder().build().await;
    let ids = fixture.ids();
    let faults = Arc::new(Faults::default());
    let factory = Factory::new(&mut fixture, faults.clone());
    let mut b = fixture.manager(1, factory.clone()).start().await;
    let mut a = fixture.manager(0, factory.clone()).start().await;
    converged(&a, &b, ids[0], ids[1], None).await;
    let old = b
        .shared
        .links
        .lock()
        .unwrap()
        .get(&ids[0])
        .unwrap()
        .fact
        .clone();
    let old_scope = b
        .shared
        .links
        .lock()
        .unwrap()
        .get(&ids[0])
        .unwrap()
        .scope
        .clone();
    let old_ack = faults
        .controls
        .lock()
        .unwrap()
        .iter()
        .find(|(kind, _, _)| *kind == 3)
        .unwrap()
        .2
        .clone();
    let mut c = fixture.manager(2, factory.clone()).start().await;
    linked(&b, 2, TransportProtocol::NativeUdp).await;
    let sibling = b
        .shared
        .links
        .lock()
        .unwrap()
        .get(&ids[2])
        .unwrap()
        .fact
        .clone();
    a.shutdown().await.unwrap();
    // Actually rebind the old initiating tuple after its physical owner closed.
    let rebound = UdpSocket::bind(old.remote).await.unwrap();
    let (transcript, init, ack) = challenge(&fixture, &rebound, old.local, "udp").await;
    old_scope.check().unwrap();
    assert_eq!(b.shared.links.lock().unwrap()[&ids[0]].fact.id, old.id);
    let other_source = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    other_source.send_to(&init, old.local).await.unwrap(); // Copy cannot acquire its own source challenge.
    other_source.send_to(&ack, old.local).await.unwrap(); // Valid signature, wrong source.
    rebound.send_to(&old_ack, old.local).await.unwrap(); // Correct source, old transcript.
    tokio::time::sleep(Duration::from_millis(50)).await;
    old_scope.check().unwrap();
    assert_eq!(b.shared.links.lock().unwrap()[&ids[0]].fact.id, old.id);
    rebound.send_to(&ack, old.local).await.unwrap();
    selected(&b, ids[0], transcript.id()).await;
    assert!(old_scope.cancellation.is_cancelled());
    assert_eq!(
        b.shared.links.lock().unwrap()[&ids[0]].fact.socket_id,
        old.socket_id
    );
    assert_eq!(b.shared.links.lock().unwrap()[&ids[2]].fact, sibling);
    b.shared.links.lock().unwrap()[&ids[2]]
        .scope
        .check()
        .unwrap();
    rebound.send_to(&init, old.local).await.unwrap(); // Consumed nonce cannot restart a channel.
    rebound.send_to(&old_ack, old.local).await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        b.shared.links.lock().unwrap()[&ids[0]].fact.id,
        transcript.id()
    );
    b.shutdown().await.unwrap();
    c.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn pending_supersession_cannot_replace_a_newer_authenticated_channel() {
    let mut fixture = fixture::Fixture::builder()
        .multiple_listeners()
        .build()
        .await;
    let ids = fixture.ids();
    let factory = Factory::new(&mut fixture, Arc::new(Faults::default()));
    let mut b = fixture.manager(1, factory.clone()).start().await;
    let mut a = fixture.manager(0, factory.clone()).start().await;
    converged(&a, &b, ids[0], ids[1], None).await;
    let old = b.shared.links.lock().unwrap()[&ids[0]].fact.id;
    let sender = b.shared.links.lock().unwrap()[&ids[0]].sender.clone();
    assert_eq!(
        b.shared.links.lock().unwrap()[&ids[0]]
            .fact
            .carrier
            .listener_key,
        "udp"
    );
    let listeners = b.snapshot().unwrap().listeners;
    let first = listeners
        .iter()
        .find(|listener| listener.key == "udp")
        .unwrap()
        .local;
    let second = listeners
        .iter()
        .find(|listener| listener.key == "udp2")
        .unwrap()
        .local;
    let stale_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let winner_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let (stale, _, stale_ack) = challenge(&fixture, &stale_socket, first, "udp").await;
    let (winner, _, winner_ack) = challenge(&fixture, &winner_socket, second, "udp2").await;
    assert_eq!(b.shared.links.lock().unwrap()[&ids[0]].fact.id, old);
    winner_socket.send_to(&winner_ack, second).await.unwrap();
    selected(&b, ids[0], winner.id()).await;
    stale_socket.send_to(&stale_ack, first).await.unwrap();
    // A subsequent challenge on the same ordered loopback source establishes
    // that the old pending ACK was consumed, without ACKing another replacement.
    let _ = challenge(&fixture, &stale_socket, first, "udp").await;
    assert_eq!(b.shared.links.lock().unwrap()[&ids[0]].fact.id, winner.id());
    assert_ne!(stale.id(), winner.id());
    // A listener that captured an empty slot during probe recovery must also
    // reject a later winner, even if ordinary ranking prefers its candidate.
    let current = b.shared.links.lock().unwrap()[&ids[0]].fact.clone();
    let mut candidate = current.clone();
    candidate.id = stale.id();
    candidate.carrier.listener_key = "udp".into();
    candidate.remote = stale_socket.local_addr().unwrap();
    candidate.local = first;
    candidate.socket_id = listeners
        .iter()
        .find(|listener| listener.key == "udp")
        .unwrap()
        .socket_id;
    assert!(!preferred_existing(&current, &candidate));
    let ready = || ReadyPeer {
        fact: candidate.clone(),
        sender: sender.clone(),
        scope: b
            .shared
            .scope(Some((ids[0], candidate.carrier.clone())), false),
        supersedes: Replacement::Vacant(None),
        attempt: None,
    };
    assert!(!install_ready(&b.shared, ready()).unwrap());
    assert_eq!(b.shared.links.lock().unwrap()[&ids[0]].fact, current);
    retire(&b.shared, &current).unwrap();
    assert!(!b.shared.links.lock().unwrap().contains_key(&ids[0]));
    assert!(!install_ready(&b.shared, ready()).unwrap()); // Empty-slot ABA is also rejected.
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn copied_signed_init_without_source_ack_keeps_live_channel_and_business_output() {
    let mut fixture = fixture::Fixture::builder().build().await;
    let ids = fixture.ids();
    let factory = Factory::new(&mut fixture, Arc::new(Faults::default()));
    let mut b = fixture.manager(1, factory.clone()).start().await;
    let mut a = fixture.manager(0, factory.clone()).start().await;
    converged(&a, &b, ids[0], ids[1], None).await;
    let (mut sa, mut sb) = (
        secure(&fixture.identities[0], &a),
        secure(&fixture.identities[1], &b),
    );
    let cid = handshake(
        &mut a,
        &mut b,
        &mut sa,
        &mut sb,
        fixture.demands[0],
        vec![ids[0], ids[1]],
    )
    .await;
    exchange(&a, &mut b, &mut sa, &mut sb, cid, packet(&fixture, false)).await;
    let old = b.shared.links.lock().unwrap()[&ids[0]].fact.clone();
    let scope = b.shared.links.lock().unwrap()[&ids[0]].scope.clone();
    let copied_source = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let _ = challenge(&fixture, &copied_source, old.local, "udp").await;
    assert_eq!(b.shared.links.lock().unwrap()[&ids[0]].fact, old);
    scope.check().unwrap();
    exchange(&b, &mut a, &mut sb, &mut sa, cid, packet(&fixture, true)).await;
    assert_eq!(b.shared.links.lock().unwrap()[&ids[0]].fact, old);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}
