use super::*;
use crate::adjacency::{Initiation, Kind, Transcript};
use meshway_dataplane::{
    Action, Packet,
    secure::{Received, SecureNode},
};

// A semantic recovery deadline below the unchanged six-second health window.
const RECOVERY: Duration = Duration::from_secs(3);

#[tokio::test]
async fn passive_restart_challenges_dial_only_peer_without_retiring_it_until_fresh_ack_and_ready() {
    let mut fixture = support::Fixture::new(false, false).await;
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
    let mut b = Manager::start(
        fixture.identities[1].clone(),
        factory.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    let mut a = Manager::start(fixture.identities[0].clone(), factory, Default::default())
        .await
        .unwrap();
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
    let fixture = support::Fixture::new(false, false).await;
    let ids = fixture.ids();
    let carrier = support::carriers(&fixture.identities[0])[&ids[1]].clone();
    let (_, intro) = Initiation::new(&fixture.identities[0], &carrier).unwrap();
    let a = fixture.identities[0].snapshot().configuration().unwrap();
    let b = fixture.identities[1].snapshot().configuration().unwrap();
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
async fn native_tcp_session_survives_staggered_control_publications_without_reopening() {
    let mut fixture = support::Fixture::new(true, false).await;
    let ids = fixture.ids();
    let faults = Arc::new(Faults::default());
    faults.blackhole.store(true, Ordering::SeqCst);
    let factory = Factory::new(&mut fixture, faults);
    let mut a = Manager::start(
        fixture.identities[0].clone(),
        factory.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    let mut b = Manager::start(fixture.identities[1].clone(), factory, Default::default())
        .await
        .unwrap();
    linked(&a, 1, TransportProtocol::NativeTcp).await;
    linked(&b, 1, TransportProtocol::NativeTcp).await;
    let af = a.snapshot().unwrap().links[0].clone();
    let bf = b.snapshot().unwrap().links[0].clone();
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
    for _ in 0..5 {
        let lease = fixture.controls[0].renew_instance().await.unwrap();
        fixture.controls[0]
            .renew_demand(&meshway_protocol::RenewDemandRequest {
                instance: meshway_protocol::InstanceRequest {
                    instance_id: lease.instance_id,
                    generation: lease.generation,
                },
                request_id: fixture.demands[0],
                expires_at_ms: lease.expires_at_ms,
            })
            .await
            .unwrap();
        for (index, manager, node) in [(0, &mut a, &mut sa), (1, &mut b, &mut sb)] {
            fixture.controls[index]
                .refresh_configuration()
                .await
                .unwrap();
            let next = fixture.controls[index]
                .dataplane_identity(fixture.controls[index].execution_snapshot().unwrap())
                .unwrap();
            assert!(manager.can_reconcile(&next));
            node.reconcile(next.clone()).unwrap();
            tokio::time::timeout(Duration::from_secs(2), manager.reconcile(next.clone()))
                .await
                .unwrap()
                .unwrap();
            node.set_carriers(manager.carriers().unwrap()).unwrap();
            fixture.identities[index] = next;
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert_eq!(a.snapshot().unwrap().links[0], af);
        assert_eq!(b.snapshot().unwrap().links[0], bf);
        exchange(&b, &mut a, &mut sb, &mut sa, cid, packet(&fixture, true)).await;
        assert!(
            sa.ready_circuits()
                .unwrap()
                .iter()
                .any(|info| info.id == cid)
        );
    }
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn direct_publication_retires_only_the_unadmitted_peer_on_a_shared_udp_descriptor() {
    let mut fixture = support::Fixture::new(false, false).await;
    let ids = fixture.ids();
    let factory = Factory::new(&mut fixture, Arc::new(Faults::default()));
    let mut a = Manager::start(
        fixture.identities[0].clone(),
        factory.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    let mut b = Manager::start(
        fixture.identities[1].clone(),
        factory.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    let mut c = Manager::start(fixture.identities[2].clone(), factory, Default::default())
        .await
        .unwrap();
    linked(&a, 1, TransportProtocol::NativeUdp).await;
    linked(&b, 2, TransportProtocol::NativeUdp).await;
    linked(&c, 1, TransportProtocol::NativeUdp).await;
    let before = b.shared.links.lock().unwrap()[&ids[0]].fact.clone();
    let retired = b.shared.links.lock().unwrap()[&ids[2]].scope.clone();
    assert!(b.shared.links.lock().unwrap().last.contains_key(&ids[2]));
    let socket = b.snapshot().unwrap().listeners[0].socket_id;
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
    let instance = fixture.controls[2].execution_snapshot().unwrap().instance();
    fixture.controls[2]
        .cancel_demand(&meshway_protocol::CancelDemandRequest {
            instance,
            request_id: fixture.demands[1],
        })
        .await
        .unwrap();
    for (index, manager, node) in [(0, &mut a, &mut sa), (1, &mut b, &mut sb)] {
        fixture.controls[index]
            .refresh_configuration()
            .await
            .unwrap();
        let next = fixture.controls[index]
            .dataplane_identity(fixture.controls[index].execution_snapshot().unwrap())
            .unwrap();
        assert!(
            manager.can_reconcile(&next),
            "index={index}, stopped={}, bounds={}, lineage={}, resources={:?}",
            manager.shared.stop.is_cancelled(),
            manager
                .shared
                .identity
                .snapshot()
                .retention_deadline()
                .is_ok(),
            next.snapshot()
                .continuation_from(manager.shared.identity.snapshot())
                .is_ok(),
            next.snapshot()
                .continuation_from(manager.shared.identity.snapshot())
                .ok()
                .map(|old| {
                    let cfg = next.snapshot().configuration().unwrap();
                    (
                        old.listeners == cfg.listeners,
                        old.dialers == cfg.dialers,
                        old.peers == cfg.peers,
                        old.mesh_address_pools == cfg.mesh_address_pools,
                    )
                })
        );
        node.reconcile(next.clone()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), manager.reconcile(next.clone()))
            .await
            .unwrap()
            .unwrap();
        node.set_carriers(manager.carriers().unwrap()).unwrap();
        fixture.identities[index] = next;
    }
    assert_eq!(b.snapshot().unwrap().links, vec![before.clone()]);
    assert_eq!(b.snapshot().unwrap().listeners[0].socket_id, socket);
    assert!(retired.check().is_err());
    assert!(!b.shared.links.lock().unwrap().last.contains_key(&ids[2]));
    assert_eq!(
        b.shared.links.lock().unwrap().last.get(&ids[0]),
        Some(&before.id)
    );
    exchange(&b, &mut a, &mut sb, &mut sa, cid, packet(&fixture, true)).await;
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    c.shutdown().await.unwrap();
    fixture.close().await;
}

async fn converged(a: &Manager, b: &Manager, a_id: NodeId, b_id: NodeId, old: Option<[u8; 32]>) {
    let result = tokio::time::timeout(RECOVERY, async {
        loop {
            let af = a.snapshot().unwrap();
            let bf = b.snapshot().unwrap();
            if let Some(left) = af.links.iter().find(|link| link.peer == b_id)
                && let Some(right) = bf.links.iter().find(|link| link.peer == a_id)
                && left.id == right.id
                && Some(left.id) != old
            {
                assert_eq!(left.carrier, right.carrier);
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "reattach before the healthy predecessor expires: {a_id} -> {b_id}; left={:?}; right={:?}",
        a.snapshot(),
        b.snapshot()
    );
}

fn secure(identity: &meshway_authority::DataPlaneIdentity, manager: &Manager) -> SecureNode {
    SecureNode::new(
        identity.clone(),
        Default::default(),
        manager.carriers().unwrap(),
    )
    .unwrap()
}

async fn handshake(
    a: &mut Manager,
    b: &mut Manager,
    sa: &mut SecureNode,
    sb: &mut SecureNode,
    demand: meshway_model::RequestId,
    path: Vec<NodeId>,
) -> [u8; 32] {
    let first = sa.start(demand, path).unwrap();
    let cid = first.circuit();
    a.send(first).await.unwrap();
    let (link, packet) = frame(b).await;
    let Received::Established {
        response: Some(response),
        ..
    } = sb.receive(&link.carrier, packet.payload).unwrap()
    else {
        panic!("terminal handshake");
    };
    b.send(response).await.unwrap();
    let (link, packet) = frame(a).await;
    assert!(
        matches!(sa.receive(&link.carrier, packet.payload).unwrap(), Received::Established { circuit, .. } if circuit == cid)
    );
    cid
}

pub(super) fn packet(fixture: &support::Fixture, reply: bool) -> Packet {
    packet_between(fixture, 0, 1, reply)
}

pub(super) fn packet_between(
    fixture: &support::Fixture,
    source: usize,
    target: usize,
    reply: bool,
) -> Packet {
    let address = |index: usize| {
        fixture.identities[index]
            .snapshot()
            .configuration()
            .unwrap()
            .addresses
            .ipv4
            .unwrap()
    };
    let (source, destination, sport, dport) = if reply {
        (address(target), address(source), 42424u16, 50000u16)
    } else {
        (address(source), address(target), 50000u16, 42424u16)
    };
    let payload = b"same protected circuit after UDP reattachment";
    let mut bytes = vec![0; 28];
    bytes[0] = 0x45;
    bytes[2..4].copy_from_slice(&u16::try_from(28 + payload.len()).unwrap().to_be_bytes());
    bytes[8] = 64;
    bytes[9] = 17;
    bytes[12..16].copy_from_slice(&source.octets());
    bytes[16..20].copy_from_slice(&destination.octets());
    let sum = bytes[..20]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u32::from(u16::from_be_bytes([pair[0], pair[1]])))
        .sum::<u32>();
    let folded = (sum & 65535) + (sum >> 16);
    let checksum = !(((folded & 65535) + (folded >> 16)) as u16);
    bytes[10..12].copy_from_slice(&checksum.to_be_bytes());
    bytes[20..22].copy_from_slice(&sport.to_be_bytes());
    bytes[22..24].copy_from_slice(&dport.to_be_bytes());
    bytes[24..26].copy_from_slice(&u16::try_from(8 + payload.len()).unwrap().to_be_bytes());
    // IPv4 permits an omitted UDP checksum; production Packet parses the wire.
    bytes.extend(payload);
    Packet::parse(bytes).unwrap()
}

pub(super) async fn exchange(
    from: &Manager,
    to: &mut Manager,
    source: &mut SecureNode,
    target: &mut SecureNode,
    cid: [u8; 32],
    packet: Packet,
) {
    let expected = packet.as_bytes().to_vec();
    from.send(source.send(cid, packet).unwrap()).await.unwrap();
    let (link, received) = frame(to).await;
    let Received::Packet(pending) = target.receive(&link.carrier, received.payload).unwrap() else {
        panic!("business packet");
    };
    pending
        .release(|action| match action {
            Action::Deliver { packet, .. } => assert_eq!(packet.as_bytes(), expected),
            _ => panic!("endpoint delivery"),
        })
        .unwrap();
}

#[tokio::test]
async fn restart_reattaches_across_opposite_udp_carrier_and_fences_unpolled_old_output() {
    let mut fixture = support::Fixture::reverse().await;
    let ids = fixture.ids();
    let faults = Arc::new(Faults::default());
    let factory = Factory::new(&mut fixture, faults.clone());
    // Initial physical orientation is B -> A, independently of ID ordering.
    factory.blocked_dialers.lock().unwrap().insert(ids[0]);
    let mut a = Manager::start(
        fixture.identities[0].clone(),
        factory.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    let mut b = Manager::start(
        fixture.identities[1].clone(),
        factory.clone(),
        Default::default(),
    )
    .await
    .unwrap();
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

async fn control(socket: &UdpSocket, kind: Kind) -> Vec<u8> {
    tokio::time::timeout(RECOVERY, async {
        let mut bytes = vec![0; adjacency::MAX_MESSAGE];
        loop {
            let (size, _) = socket.recv_from(&mut bytes).await.unwrap();
            if let Ok((actual, body)) = adjacency::decode(&bytes[..size])
                && actual == kind
            {
                return body.to_vec();
            }
        }
    })
    .await
    .unwrap()
}

async fn challenge(
    fixture: &support::Fixture,
    socket: &UdpSocket,
    listener: SocketAddr,
    key: &str,
) -> (Transcript, Vec<u8>, Vec<u8>) {
    let ids = fixture.ids();
    let carrier = Carrier {
        dialer: ids[0],
        listener: ids[1],
        listener_key: key.into(),
        protocol: TransportProtocol::NativeUdp,
    };
    let (initiation, intro) = Initiation::new(&fixture.identities[0], &carrier).unwrap();
    let init = adjacency::encode(Kind::Init, &intro).unwrap();
    socket.send_to(&init, listener).await.unwrap();
    let response = control(socket, Kind::Challenge).await;
    let transcript = initiation
        .challenge(
            fixture.identities[0].snapshot().configuration().unwrap(),
            ids[1],
            &response,
        )
        .unwrap();
    let ack = adjacency::encode(
        Kind::Ack,
        &transcript
            .message(&fixture.identities[0], Kind::Ack)
            .unwrap(),
    )
    .unwrap();
    (transcript, init, ack)
}

async fn selected(manager: &Manager, peer: NodeId, id: [u8; 32]) {
    tokio::time::timeout(RECOVERY, async {
        loop {
            if manager
                .snapshot()
                .unwrap()
                .links
                .iter()
                .any(|link| link.peer == peer && link.id == id)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn receipt_uses_replacement_channel_and_rejects_same_carrier_old_scope_waiting_for_sender() {
    let mut fixture = support::Fixture::new(false, false).await;
    let ids = fixture.ids();
    let faults = Arc::new(Faults::default());
    let factory = Factory::new(&mut fixture, faults.clone());
    let mut b = Manager::start(
        fixture.identities[1].clone(),
        factory.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    let mut a = Manager::start(fixture.identities[0].clone(), factory, Default::default())
        .await
        .unwrap();
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
    let mut fixture = support::Fixture::new(false, false).await;
    let ids = fixture.ids();
    let faults = Arc::new(Faults::default());
    let factory = Factory::new(&mut fixture, faults.clone());
    let mut b = Manager::start(
        fixture.identities[1].clone(),
        factory.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    let mut a = Manager::start(
        fixture.identities[0].clone(),
        factory.clone(),
        Default::default(),
    )
    .await
    .unwrap();
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
    let mut c = Manager::start(fixture.identities[2].clone(), factory, Default::default())
        .await
        .unwrap();
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
    let mut fixture = support::Fixture::multiple_listeners().await;
    let ids = fixture.ids();
    let factory = Factory::new(&mut fixture, Arc::new(Faults::default()));
    let mut b = Manager::start(
        fixture.identities[1].clone(),
        factory.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    let mut a = Manager::start(fixture.identities[0].clone(), factory, Default::default())
        .await
        .unwrap();
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
    let mut fixture = support::Fixture::new(false, false).await;
    let ids = fixture.ids();
    let factory = Factory::new(&mut fixture, Arc::new(Faults::default()));
    let mut b = Manager::start(
        fixture.identities[1].clone(),
        factory.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    let mut a = Manager::start(fixture.identities[0].clone(), factory, Default::default())
        .await
        .unwrap();
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

async fn route_packet(
    managers: &mut [Manager],
    nodes: &mut [SecureNode],
    cid: [u8; 32],
    reply: bool,
    packet: Packet,
) {
    let (from, to) = if reply { (1, 0) } else { (0, 1) };
    let expected = packet.as_bytes().to_vec();
    managers[from]
        .send(nodes[from].send(cid, packet).unwrap())
        .await
        .unwrap();
    let (link, data) = frame(&mut managers[2]).await;
    let Received::Forward(pending) = nodes[2].receive(&link.carrier, data.payload).unwrap() else {
        panic!("relay");
    };
    managers[2].send(pending).await.unwrap();
    let (link, data) = frame(&mut managers[to]).await;
    let Received::Packet(pending) = nodes[to].receive(&link.carrier, data.payload).unwrap() else {
        panic!("endpoint");
    };
    pending
        .release(|action| match action {
            Action::Deliver { packet, .. } => assert_eq!(packet.as_bytes(), expected),
            _ => panic!("exact-path delivery"),
        })
        .unwrap();
}

async fn all_links(managers: &[Manager], ids: &[NodeId]) {
    for (a, b) in [(0, 2), (2, 1)] {
        converged(&managers[a], &managers[b], ids[a], ids[b], None).await;
    }
}

#[tokio::test]
async fn staggered_three_node_managers_and_real_demand_publication_keep_original_circuit() {
    let mut fixture = support::Fixture::reverse().await;
    let ids = fixture.ids();
    let path = vec![ids[0], ids[2], ids[1]];
    assert!(
        fixture.identities[0]
            .snapshot()
            .configuration()
            .unwrap()
            .permits[&fixture.demands[0]]
            .paths
            .iter()
            .any(|candidate| candidate.nodes == path)
    );
    let factory = Factory::new(&mut fixture, Arc::new(Faults::default()));
    let mut managers = Vec::new();
    for identity in &fixture.identities {
        managers.push(
            Manager::start(identity.clone(), factory.clone(), Default::default())
                .await
                .unwrap(),
        );
    }
    all_links(&managers, &ids).await;
    let mut nodes: Vec<_> = fixture
        .identities
        .iter()
        .zip(&managers)
        .map(|(identity, manager)| secure(identity, manager))
        .collect();
    let init = nodes[0].start(fixture.demands[0], path.clone()).unwrap();
    let cid = init.circuit();
    managers[0].send(init).await.unwrap();
    let (link, data) = frame(&mut managers[2]).await;
    let Received::Forward(pending) = nodes[2].receive(&link.carrier, data.payload).unwrap() else {
        panic!("relay init");
    };
    managers[2].send(pending).await.unwrap();
    let (link, data) = frame(&mut managers[1]).await;
    let Received::Established {
        response: Some(pending),
        ..
    } = nodes[1].receive(&link.carrier, data.payload).unwrap()
    else {
        panic!("terminal init");
    };
    managers[1].send(pending).await.unwrap();
    let (link, data) = frame(&mut managers[2]).await;
    let Received::Forward(pending) = nodes[2].receive(&link.carrier, data.payload).unwrap() else {
        panic!("relay response");
    };
    managers[2].send(pending).await.unwrap();
    let (link, data) = frame(&mut managers[0]).await;
    assert!(matches!(
        nodes[0].receive(&link.carrier, data.payload).unwrap(),
        Received::Established { .. }
    ));
    route_packet(
        &mut managers,
        &mut nodes,
        cid,
        false,
        packet(&fixture, false),
    )
    .await;
    // Each physical owner refreshes separately; logical circuit/flow owners stay.
    for index in [0, 2, 1] {
        managers[index].shutdown().await.unwrap();
        managers[index]
            .reconcile(fixture.identities[index].clone())
            .await
            .unwrap();
        all_links(&managers, &ids).await;
        for (node, manager) in nodes.iter_mut().zip(&managers) {
            node.set_carriers(manager.carriers().unwrap()).unwrap();
        }
        route_packet(&mut managers, &mut nodes, cid, true, packet(&fixture, true)).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    all_links(&managers, &ids).await;
    for (node, manager) in nodes.iter_mut().zip(&managers) {
        node.set_carriers(manager.carriers().unwrap()).unwrap();
    }
    route_packet(&mut managers, &mut nodes, cid, true, packet(&fixture, true)).await;
    let lease = fixture.controls[0].renew_instance().await.unwrap();
    let original_channels: Vec<_> = managers
        .iter()
        .map(|manager| manager.snapshot().unwrap().links)
        .collect();
    fixture.controls[0]
        .renew_demand(&meshway_protocol::RenewDemandRequest {
            instance: meshway_protocol::InstanceRequest {
                instance_id: lease.instance_id,
                generation: lease.generation,
            },
            request_id: fixture.demands[0],
            expires_at_ms: lease.expires_at_ms,
        })
        .await
        .unwrap();
    for index in [0, 2, 1] {
        fixture.controls[index]
            .refresh_configuration()
            .await
            .unwrap();
        let identity = fixture.controls[index]
            .dataplane_identity(fixture.controls[index].execution_snapshot().unwrap())
            .unwrap();
        nodes[index].reconcile(identity.clone()).unwrap();
        managers[index].reconcile(identity.clone()).await.unwrap();
        assert_eq!(
            managers[index].snapshot().unwrap().links,
            original_channels[index],
            "the direct publication preserves the mandatory-path physical channels"
        );
        fixture.identities[index] = identity;
        all_links(&managers, &ids).await;
        for (node, manager) in nodes.iter_mut().zip(&managers) {
            node.set_carriers(manager.carriers().unwrap()).unwrap();
        }
        route_packet(&mut managers, &mut nodes, cid, true, packet(&fixture, true)).await;
    }
    all_links(&managers, &ids).await;
    for (index, (node, manager)) in nodes.iter_mut().zip(&managers).enumerate() {
        node.set_carriers(manager.carriers().unwrap()).unwrap();
        // Endpoints own the business flow; a relay forwards at circuit level only.
        let endpoint = path.first() == Some(&ids[index]) || path.last() == Some(&ids[index]);
        assert_eq!(node.retained_flows(), usize::from(endpoint));
        assert!(
            node.ready_circuits()
                .unwrap()
                .iter()
                .any(|info| info.id == cid && info.path == path)
        );
    }
    route_packet(&mut managers, &mut nodes, cid, true, packet(&fixture, true)).await;
    route_packet(
        &mut managers,
        &mut nodes,
        cid,
        false,
        packet(&fixture, false),
    )
    .await;
    for manager in &mut managers {
        manager.shutdown().await.unwrap();
    }
    fixture.close().await;
}
