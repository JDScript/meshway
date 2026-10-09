use super::*;

pub(super) async fn passive_restart_challenges_dial_only_peer_without_retiring_it_until_fresh_ack_and_ready()
 {
    // C: wait for the real signed health probe (PROBE_INTERVAL = 2 s) and
    // observed ACK. Sending a synthetic trigger would bypass passive recovery.
    let mut fixture = fixture::Fixture::builder().build().await;
    let ids = fixture.ids();
    assert!(
        fixture.identities[0]
            .snapshot()
            .configuration()
            .unwrap()
            .listeners
            .is_empty()
    );
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
    let old = a.shared.links.lock().unwrap()[&ids[1]].fact.clone();
    let old_scope = a.shared.links.lock().unwrap()[&ids[1]].scope.clone();
    let acks = || {
        faults
            .controls
            .lock()
            .unwrap()
            .iter()
            .filter(|(kind, _, _)| *kind == Kind::Ack as u8)
            .count()
    };
    let before_ack = acks();
    faults.held_controls.lock().unwrap().insert(Kind::Ack as u8);
    b.shutdown().await.unwrap();
    b.reconcile(fixture.identities[1].clone()).await.unwrap();
    // No listener/dial candidate was added to A. Its existing real signed Ping
    // solicits a new server challenge; the actual ACK is held at its raw sender.
    tokio::time::timeout(RECOVERY, async {
        while acks() == before_ack {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("signed recovery challenge must reach the existing dial-only socket");
    old_scope.check().unwrap();
    assert_eq!(a.shared.links.lock().unwrap()[&ids[1]].fact.id, old.id);
    assert!(!b.shared.links.lock().unwrap().contains_key(&ids[0]));
    faults
        .held_controls
        .lock()
        .unwrap()
        .remove(&(Kind::Ack as u8));
    converged(&a, &b, ids[0], ids[1], Some(old.id)).await;
    assert!(old_scope.cancellation.is_cancelled());
    assert_eq!(a.snapshot().unwrap().links[0].socket_id, old.socket_id);
    sa.set_carriers(a.carriers().unwrap()).unwrap();
    sb.set_carriers(b.carriers().unwrap()).unwrap();
    exchange(&b, &mut a, &mut sb, &mut sa, cid, packet(&fixture, true)).await;
    assert_eq!(sa.ready_circuits().unwrap()[0].id, cid);
    assert_eq!(sb.ready_circuits().unwrap()[0].id, cid);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn recovery_probe_and_challenge_bind_registered_peer_channel_role_and_outstanding_nonce() {
    let fixture = fixture::Fixture::builder().build().await;
    let ids = fixture.ids();
    let carrier = fixture::carriers(&fixture.identities[0])[&ids[1]].clone();
    let (pa, pb) = (
        Plan::new(fixture.identities[0].snapshot()).unwrap(),
        Plan::new(fixture.identities[1].snapshot()).unwrap(),
    );
    let (a, b) = (&*pa, &*pb);
    let (_, intro) = Initiation::new(&fixture.identities[0], a, &carrier).unwrap();
    let (_, _, init) = adjacency::incoming(b, &[(ids[0], carrier.clone())], &intro).unwrap();
    let old = init.respond().unwrap();
    let probe = adjacency::nonce().unwrap();
    let ping = old
        .health(&fixture.identities[0], Kind::Ping, &probe)
        .unwrap();
    let (old_id, recovery) = Initiation::from_probe(b, ids[0], &carrier, &ping).unwrap();
    assert_eq!(old_id, old.id());
    let fresh = recovery.respond().unwrap();
    let challenge = fresh
        .message(&fixture.identities[1], Kind::Challenge)
        .unwrap();
    let checked = old.probe_challenge(a, ids[1], probe, &challenge).unwrap();
    assert_eq!(checked.id(), fresh.id());
    assert!(
        old.probe_challenge(a, ids[1], adjacency::nonce().unwrap(), &challenge)
            .is_err()
    );
    assert!(old.probe_challenge(a, ids[2], probe, &challenge).is_err());
    let mut tampered = ping.clone();
    tampered[64] ^= 1; // The predecessor channel ID is part of the signed proof.
    assert!(Initiation::from_probe(b, ids[0], &carrier, &tampered).is_err());
    let pong = old
        .health(&fixture.identities[0], Kind::Pong, &probe)
        .unwrap();
    assert!(Initiation::from_probe(b, ids[0], &carrier, &pong).is_err());
    assert!(Initiation::from_probe(b, ids[2], &carrier, &ping).is_err());
    let mut reversed = carrier.clone();
    std::mem::swap(&mut reversed.dialer, &mut reversed.listener);
    assert!(Initiation::from_probe(b, ids[0], &reversed, &ping).is_err());
    let stale_ack = old.message(&fixture.identities[0], Kind::Ack).unwrap();
    assert!(fresh.verify(b, ids[0], Kind::Ack, &stale_ack).is_err());
    let ack = checked.message(&fixture.identities[0], Kind::Ack).unwrap();
    fresh.verify(b, ids[0], Kind::Ack, &ack).unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn restart_reattaches_across_opposite_udp_carrier_and_fences_unpolled_old_output() {
    let mut fixture = fixture::Fixture::builder().reverse().build().await;
    let ids = fixture.ids();
    let faults = Arc::new(Faults::default());
    let factory = Factory::new(&mut fixture, faults.clone());
    // Initial physical orientation is B -> A, independently of ID ordering.
    factory.blocked_dialers.lock().unwrap().insert(ids[0]);
    let mut a = fixture.manager(0, factory.clone()).start().await;
    let mut b = fixture.manager(1, factory.clone()).start().await;
    converged(&a, &b, ids[0], ids[1], None).await;
    assert_eq!(b.snapshot().unwrap().links[0].carrier.dialer, ids[1]);
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
    old_scope.check().unwrap();
    faults.block_data.store(true, Ordering::SeqCst);
    let pending = sb.send(cid, packet(&fixture, true)).unwrap();
    let before = faults.writes.load(Ordering::SeqCst);
    let mut sending = Box::pin(b.send(pending));
    assert!(futures_util::poll!(sending.as_mut()).is_pending());
    assert!(faults.blocked.load(Ordering::SeqCst));
    factory.blocked_dialers.lock().unwrap().insert(ids[1]);
    factory.blocked_dialers.lock().unwrap().remove(&ids[0]);
    a.shutdown().await.unwrap();
    a.reconcile(fixture.identities[0].clone()).await.unwrap();
    converged(&a, &b, ids[0], ids[1], Some(old.id)).await;
    assert_eq!(b.snapshot().unwrap().links[0].carrier.dialer, ids[0]);
    assert!(old_scope.cancellation.is_cancelled());
    faults.block_data.store(false, Ordering::SeqCst);
    faults.waker.wake();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(faults.writes.load(Ordering::SeqCst), before);
    drop(sending); // It was deliberately not polled while replacement committed.
    sa.set_carriers(a.carriers().unwrap()).unwrap();
    sb.set_carriers(b.carriers().unwrap()).unwrap();
    exchange(&b, &mut a, &mut sb, &mut sa, cid, packet(&fixture, true)).await;
    exchange(&a, &mut b, &mut sa, &mut sb, cid, packet(&fixture, false)).await;
    assert_eq!(sa.ready_circuits().unwrap()[0].id, cid);
    assert_eq!(sb.ready_circuits().unwrap()[0].id, cid);
    assert_eq!(sa.retained_flows(), 1);
    assert_eq!(sb.retained_flows(), 1);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}
