//! Real socket/Noise recovery regressions. The original real-59s failing probe
//! remains untouched in the independent field-review TMP archive.
use super::security::intercept_peer;
use super::*;

async fn tcp_pair(a: &Runtime, b: &Runtime) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (outgoing, incoming) = tokio::join!(
        tokio::net::TcpStream::connect(listener.local_addr().unwrap()),
        listener.accept()
    );
    let outgoing = outgoing.unwrap();
    let incoming = incoming.unwrap().0;
    outgoing.set_nodelay(true).unwrap();
    incoming.set_nodelay(true).unwrap();
    let (a, b) = tokio::join!(
        a.attach_stream(
            outgoing,
            StreamRole::Dialer {
                peer: b.node_id().into()
            },
            TransportKind::NativeTcp
        ),
        b.attach_stream(incoming, StreamRole::Acceptor, TransportKind::NativeTcp)
    );
    a.unwrap();
    b.unwrap();
}
async fn receive(rx: &mut mpsc::Receiver<ReceivedPacket>, payload: &[u8]) {
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap()
            .payload,
        payload
    );
}
async fn absent(runtime: &Runtime, peer: &str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while runtime.connected_peers().iter().any(|id| id == peer) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn hinted(runtime: &Runtime, route: &[String], id: [u8; 16]) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !lock(&runtime.handle.inner.recovery).hinted(route, id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
fn hint(receiver: &Runtime, route: &[String], id: [u8; 16]) {
    receiver
        .handle
        .inner
        .send(
            &route[route.len() - 2],
            Packet {
                phase: Phase::UnknownSession,
                id,
                path: route.to_vec(),
                body: b"unknown-session-v1".to_vec(),
            },
        )
        .unwrap();
}
async fn restart(relayed: bool, one_way: bool, reverse_roles: bool) {
    let mut configs = configs();
    let mut route = if relayed {
        path()
    } else {
        vec!["nyc".into(), "sha".into()]
    };
    if reverse_roles {
        route.reverse();
    }
    let (source_index, destination_index) = if reverse_roles { (2, 0) } else { (0, 2) };
    let reverse: Vec<_> = route.iter().cloned().rev().collect();
    for config in &mut configs {
        config.allowed_paths = if one_way {
            vec![route.clone()]
        } else {
            vec![route.clone(), reverse.clone()]
        };
    }
    let a = Runtime::start(configs[source_index].clone()).await.unwrap();
    let b = Runtime::start(configs[destination_index].clone())
        .await
        .unwrap();
    let middle = if relayed {
        Some(Runtime::start(configs[1].clone()).await.unwrap())
    } else {
        None
    };
    let neighbor = middle.as_ref().unwrap_or(&a);
    if let Some(middle) = &middle {
        tcp_pair(&a, middle).await;
    }
    tcp_pair(&b, neighbor).await;
    let token = lock(&a.handle.inner.peers)[&route[1]].token;
    let mut at_b = b.take_packet_receiver().unwrap();
    a.send_packet(route.clone(), b"baseline".to_vec())
        .await
        .unwrap();
    receive(&mut at_b, b"baseline").await;
    let (old, expires) = {
        let sessions = a.handle.inner.sending.lock().await;
        (sessions[&route].id, sessions[&route].expires)
    };
    let mut at_a = a.take_packet_receiver().unwrap();
    if !one_way {
        b.send_packet(reverse.clone(), b"reverse-baseline".to_vec())
            .await
            .unwrap();
        receive(&mut at_a, b"reverse-baseline").await;
    }
    let destination = b.node_id().to_owned();
    b.shutdown().await;
    drop(at_b);
    drop(b);
    absent(neighbor, &destination).await;
    let b = Runtime::start(configs[destination_index].clone())
        .await
        .unwrap();
    let mut at_b = b.take_packet_receiver().unwrap();
    tcp_pair(&b, neighbor).await;
    if relayed {
        assert_eq!(lock(&a.handle.inner.peers)[&route[1]].token, token);
    }
    assert_eq!(
        a.probe(route.clone(), b"resync-v1 arbitrary echo".to_vec())
            .await
            .unwrap()
            .payload,
        b"resync-v1 arbitrary echo"
    );
    if !one_way {
        b.send_packet(reverse.clone(), b"fresh reverse does not reset".to_vec())
            .await
            .unwrap();
        receive(&mut at_a, b"fresh reverse does not reset").await;
    }
    assert_eq!(a.handle.inner.sending.lock().await[&route].id, old);
    let started = Instant::now();
    a.send_packet(route.clone(), b"lost old packet".to_vec())
        .await
        .unwrap();
    hinted(&a, &route, old).await;
    assert_eq!(
        a.handle.inner.sending.lock().await[&route].id,
        old,
        "hint alone cannot reset"
    );
    a.send_packet(route.clone(), b"recovered".to_vec())
        .await
        .unwrap();
    receive(&mut at_b, b"recovered").await;
    assert_ne!(a.handle.inner.sending.lock().await[&route].id, old);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(expires > Instant::now() + Duration::from_secs(50));
    if relayed {
        assert_eq!(lock(&a.handle.inner.peers)[&route[1]].token, token);
    }
    if one_way {
        assert!(matches!(
            b.send_packet(reverse, vec![1]).await,
            Err(Error::UnauthorizedPath)
        ));
    }
    println!(
        "E2E_RECOVERY relayed={relayed} one_way={one_way} reverse_roles={reverse_roles} recovery_us={} old_ttl_remaining_ms={}",
        started.elapsed().as_micros(),
        expires
            .saturating_duration_since(Instant::now())
            .as_millis()
    );
    a.shutdown().await;
    b.shutdown().await;
    if let Some(middle) = middle {
        middle.shutdown().await;
    }
}
#[tokio::test]
async fn e2e_recovery_restart_one_hop() {
    restart(false, false, false).await;
}
#[tokio::test]
async fn e2e_recovery_restart_relay_socket_unchanged() {
    restart(true, false, false).await;
}
#[tokio::test]
async fn e2e_recovery_control_return_without_reverse_business_grant() {
    restart(true, true, false).await;
}

#[tokio::test]
async fn e2e_recovery_original_field_direction_sha_lax_nyc_restart() {
    restart(true, false, true).await;
}

#[tokio::test]
async fn e2e_recovery_false_hint_positive_answer_and_outer_reconnect_keep_key() {
    let fixture = Fixture::native(TransportKind::NativeTcp).await;
    let mut rx = fixture.sha.take_packet_receiver().unwrap();
    fixture.nyc.send_packet(path(), vec![1]).await.unwrap();
    receive(&mut rx, &[1]).await;
    let id = fixture.nyc.handle.inner.sending.lock().await[&path()].id;
    hint(&fixture.sha, &path(), [99; 16]);
    hint(&fixture.sha, &path(), id);
    hinted(&fixture.nyc, &path(), id).await;
    fixture.nyc.send_packet(path(), vec![2]).await.unwrap();
    receive(&mut rx, &[2]).await;
    assert_eq!(
        fixture.nyc.handle.inner.sending.lock().await[&path()].id,
        id
    );
    let token = lock(&fixture.nyc.handle.inner.peers)["lax"].token;
    fixture.nyc.handle.inner.unregister("lax", &token);
    absent(&fixture.lax, "nyc").await;
    fixture
        .nyc
        .wait_for_peer("lax", Duration::from_secs(2))
        .await
        .unwrap();
    fixture.nyc.send_packet(path(), vec![3]).await.unwrap();
    receive(&mut rx, &[3]).await;
    assert_eq!(
        fixture.nyc.handle.inner.sending.lock().await[&path()].id,
        id
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn e2e_recovery_cancellation_and_withdrawal_during_network_await() {
    for withdraw in [false, true] {
        let fixture = Fixture::native(TransportKind::NativeTcp).await;
        let mut rx = fixture.sha.take_packet_receiver().unwrap();
        fixture.nyc.send_packet(path(), vec![1]).await.unwrap();
        receive(&mut rx, &[1]).await;
        let id = fixture.nyc.handle.inner.sending.lock().await[&path()].id;
        hint(&fixture.sha, &path(), id);
        hinted(&fixture.nyc, &path(), id).await;
        let (mut queued, original) = intercept_peer(&fixture.nyc, "lax");
        let source = fixture.nyc.clone();
        let call =
            tokio::spawn(
                async move { source.send_packet(path(), b"must not leak".to_vec()).await },
            );
        let mut init = queued.recv().await.unwrap();
        let frames = init.batch.into_frames();
        assert_eq!(
            Packet::decode(&frames[0]).unwrap().phase as u8,
            Phase::ResyncInit as u8
        );
        init.batch = Batch::new(frames).unwrap();
        assert_eq!(lock(&fixture.nyc.handle.inner.pending).len(), 1);
        if withdraw {
            fixture.nyc.retire_packet_generation();
            assert!(matches!(call.await.unwrap(), Err(Error::Closed)));
        } else {
            call.abort();
            assert!(call.await.unwrap_err().is_cancelled());
        }
        assert!(lock(&fixture.nyc.handle.inner.pending).is_empty());
        assert_eq!(
            fixture.nyc.handle.inner.sending.lock().await[&path()].id,
            id
        );
        // Deliver the old control Init after cancellation. Its late response
        // has no pending owner and cannot publish a reset/application payload.
        original.send(init).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), rx.recv())
                .await
                .is_err()
        );
        assert!(queued.try_recv().is_err());
        assert_eq!(
            fixture.nyc.handle.inner.sending.lock().await[&path()].id,
            id
        );
        fixture.shutdown().await;
    }
}

#[tokio::test]
async fn e2e_recovery_normal_bidirectional_real_59s_rollover_does_not_ping_pong() {
    let fixture = Fixture::native(TransportKind::NativeTcp).await;
    let reverse: Vec<_> = path().into_iter().rev().collect();
    let mut at_a = fixture.nyc.take_packet_receiver().unwrap();
    let mut at_b = fixture.sha.take_packet_receiver().unwrap();
    let started = Instant::now();
    let mut initial = None;
    let mut previous = None;
    let mut changes = 0;
    loop {
        fixture.nyc.send_packet(path(), vec![1]).await.unwrap();
        receive(&mut at_b, &[1]).await;
        fixture
            .sha
            .send_packet(reverse.clone(), vec![2])
            .await
            .unwrap();
        receive(&mut at_a, &[2]).await;
        let ids = (
            fixture.nyc.handle.inner.sending.lock().await[&path()].id,
            fixture.sha.handle.inner.sending.lock().await[&reverse].id,
        );
        if initial.is_none() {
            initial = Some(ids);
        }
        if previous.is_some_and(|previous| previous != ids) {
            changes += 1;
        }
        previous = Some(ids);
        if started.elapsed() >= Duration::from_secs(60) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_ne!(initial, previous);
    assert!(changes <= 2, "ordinary rollover caused {changes} rekeys");
    assert_eq!(
        fixture
            .nyc
            .statistics()
            .native_tcp
            .drop_reasons
            .authentication,
        0
    );
    assert_eq!(
        fixture
            .sha
            .statistics()
            .native_tcp
            .drop_reasons
            .authentication,
        0
    );
    println!(
        "E2E_RECOVERY real_rollover_elapsed_ms={} paired_key_changes={changes}",
        started.elapsed().as_millis()
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn e2e_recovery_forged_ready_wrong_path_wrong_id_and_capacity_keep_active_key() {
    for attack in ["key", "path", "id", "capacity"] {
        let fixture = Fixture::native(TransportKind::NativeTcp).await;
        let mut rx = fixture.sha.take_packet_receiver().unwrap();
        fixture.nyc.send_packet(path(), vec![1]).await.unwrap();
        receive(&mut rx, &[1]).await;
        let old = fixture.nyc.handle.inner.sending.lock().await[&path()].id;
        hint(&fixture.sha, &path(), old);
        hinted(&fixture.nyc, &path(), old).await;
        let mut capacity = Vec::new();
        if attack == "capacity" {
            for n in 0..fixture.nyc.handle.inner.config.limits.max_pending_probes {
                let id = (n as u128).to_be_bytes();
                let (tx, _rx) = mpsc::channel(2);
                lock(&fixture.nyc.handle.inner.pending)
                    .insert(id, WaitingProbe { path: path(), tx });
                capacity.push(probe::PendingGuard {
                    inner: fixture.nyc.handle.inner.clone(),
                    id,
                });
            }
            fixture.nyc.send_packet(path(), vec![2]).await.unwrap();
            receive(&mut rx, &[2]).await;
        } else {
            let (mut queued, original) = intercept_peer(&fixture.nyc, "lax");
            let source = fixture.nyc.clone();
            let call = tokio::spawn(async move { source.send_packet(path(), vec![2]).await });
            let first = queued.recv().await.unwrap().batch.into_frames();
            let mut response = Packet::decode(&first[0]).unwrap();
            // Restore normal data enqueue while the independent resync waits.
            lock(&fixture.nyc.handle.inner.peers)
                .get_mut("lax")
                .unwrap()
                .tx = original;
            response.phase = Phase::ResyncReady;
            response.body = vec![0x77; 96];
            if attack == "path" {
                response.path.reverse();
            }
            if attack == "id" {
                response.id = [0x33; 16];
            }
            fixture.sha.handle.inner.send("lax", response).unwrap();
            call.await.unwrap().unwrap();
            receive(&mut rx, &[2]).await;
        }
        assert_eq!(
            fixture.nyc.handle.inner.sending.lock().await[&path()].id,
            old,
            "{attack}"
        );
        drop(capacity);
        assert!(lock(&fixture.nyc.handle.inner.pending).is_empty());
        fixture.shutdown().await;
    }
}

#[tokio::test]
async fn e2e_recovery_cancelled_missing_answer_cannot_evict_later_new_key() {
    let fixture = Fixture::native(TransportKind::NativeTcp).await;
    let mut rx = fixture.sha.take_packet_receiver().unwrap();
    fixture.nyc.send_packet(path(), vec![1]).await.unwrap();
    receive(&mut rx, &[1]).await;
    let old = fixture.nyc.handle.inner.sending.lock().await[&path()].id;
    // Exact endpoint state loss, with every outer socket kept unchanged.
    lock(&fixture.sha.handle.inner.endpoints).remove(&("nyc".into(), old));
    fixture.nyc.send_packet(path(), vec![2]).await.unwrap();
    hinted(&fixture.nyc, &path(), old).await;
    let (mut queued, original) = intercept_peer(&fixture.sha, "lax");
    let source = fixture.nyc.clone();
    let call = tokio::spawn(async move {
        source
            .send_packet(path(), b"cancelled secret".to_vec())
            .await
    });
    let ready = queued.recv().await.unwrap();
    original.send(ready).await.unwrap();
    let answer = queued.recv().await.unwrap();
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    assert!(lock(&fixture.nyc.handle.inner.pending).is_empty());
    lock(&fixture.sha.handle.inner.peers)
        .get_mut("lax")
        .unwrap()
        .tx = original.clone();
    tokio::time::sleep(Duration::from_millis(1050)).await;
    fixture.nyc.send_packet(path(), vec![3]).await.unwrap();
    hinted(&fixture.nyc, &path(), old).await;
    fixture.nyc.send_packet(path(), vec![4]).await.unwrap();
    receive(&mut rx, &[4]).await;
    let new = fixture.nyc.handle.inner.sending.lock().await[&path()].id;
    assert_ne!(new, old);
    original.send(answer).await.unwrap();
    fixture.nyc.send_packet(path(), vec![5]).await.unwrap();
    receive(&mut rx, &[5]).await;
    assert_eq!(
        fixture.nyc.handle.inner.sending.lock().await[&path()].id,
        new
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .is_err()
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn e2e_recovery_known_mac_error_and_nonce_replay_emit_no_hint() {
    let fixture = Fixture::native(TransportKind::NativeTcp).await;
    let mut rx = fixture.sha.take_packet_receiver().unwrap();
    fixture.nyc.send_packet(path(), vec![1]).await.unwrap();
    receive(&mut rx, &[1]).await;
    let id = fixture.nyc.handle.inner.sending.lock().await[&path()].id;
    let (mut queued, original) = intercept_peer(&fixture.nyc, "lax");
    fixture.nyc.send_packet(path(), vec![2]).await.unwrap();
    let outbound = queued.recv().await.unwrap();
    let frames = outbound.batch.into_frames();
    lock(&fixture.nyc.handle.inner.peers)
        .get_mut("lax")
        .unwrap()
        .tx = original;
    let good = Packet::decode(&frames[0]).unwrap();
    fixture.nyc.handle.inner.send("lax", good.clone()).unwrap();
    receive(&mut rx, &[2]).await;
    fixture.nyc.handle.inner.send("lax", good.clone()).unwrap();
    let mut bad = good;
    bad.body[..8].copy_from_slice(&100u64.to_be_bytes());
    *bad.body.last_mut().unwrap() ^= 1;
    fixture.nyc.handle.inner.send("lax", bad).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!lock(&fixture.nyc.handle.inner.recovery).hinted(&path(), id));
    assert_eq!(fixture.sha.statistics().native_tcp.drop_reasons.replay, 1);
    assert_eq!(
        fixture
            .sha
            .statistics()
            .native_tcp
            .drop_reasons
            .authentication,
        1
    );
    assert!(rx.try_recv().is_err());
    fixture.shutdown().await;
}

#[tokio::test]
async fn e2e_recovery_authenticated_answer_wrong_old_id_challenge_and_cipher_rejected() {
    use crate::crypto::{Cipher, read_handshake, write_handshake};
    for attack in [
        "old_id",
        "fingerprint",
        "challenge",
        "cipher",
        "mode",
        "prologue",
    ] {
        let fixture = Fixture::native(TransportKind::NativeTcp).await;
        let mut rx = fixture.sha.take_packet_receiver().unwrap();
        fixture.nyc.send_packet(path(), vec![1]).await.unwrap();
        receive(&mut rx, &[1]).await;
        let old = fixture.nyc.handle.inner.sending.lock().await[&path()].id;
        hint(&fixture.sha, &path(), old);
        hinted(&fixture.nyc, &path(), old).await;
        let (mut queued, original) = intercept_peer(&fixture.nyc, "lax");
        let source = fixture.nyc.clone();
        let call = tokio::spawn(async move { source.send_packet(path(), vec![2]).await });
        let mut first =
            Packet::decode(&queued.recv().await.unwrap().batch.into_frames()[0]).unwrap();
        let mut responder = fixture
            .sha
            .handle
            .inner
            .identity
            .handshake(None, &first.e2e_prologue())
            .unwrap();
        assert_eq!(
            read_handshake(&mut responder, &first.body).unwrap(),
            b"resync-v1"
        );
        first.body = write_handshake(
            &mut responder,
            if attack == "mode" {
                b"echo-v1"
            } else {
                b"resync-v1"
            },
        )
        .unwrap();
        first.phase = Phase::ResyncReady;
        if attack == "prologue" {
            first.id[0] ^= 1;
        }
        if attack == "mode" || attack == "prologue" {
            lock(&fixture.nyc.handle.inner.peers)
                .get_mut("lax")
                .unwrap()
                .tx = original;
            fixture.sha.handle.inner.send("lax", first).unwrap();
        } else {
            fixture.sha.handle.inner.send("lax", first).unwrap();
            let query =
                Packet::decode(&queued.recv().await.unwrap().batch.into_frames()[0]).unwrap();
            let mut cipher = Cipher::finish(responder).unwrap();
            let mut answer = cipher.open(&query.body).unwrap();
            assert_eq!(&answer[..16], &old);
            if attack == "old_id" {
                answer[0] ^= 1;
            }
            if attack == "challenge" {
                answer[48] ^= 1;
            }
            if attack == "fingerprint" {
                answer[16] ^= 1;
            }
            answer.push(0); // claims missing, but fails the authenticated binding
            let mut body = cipher.seal(&answer).unwrap();
            if attack == "cipher" {
                *body.last_mut().unwrap() ^= 1;
            }
            lock(&fixture.nyc.handle.inner.peers)
                .get_mut("lax")
                .unwrap()
                .tx = original;
            fixture
                .sha
                .handle
                .inner
                .send(
                    "lax",
                    Packet {
                        phase: Phase::ResyncAnswer,
                        body,
                        ..query
                    },
                )
                .unwrap();
        }
        call.await.unwrap().unwrap();
        receive(&mut rx, &[2]).await;
        assert_eq!(
            fixture.nyc.handle.inner.sending.lock().await[&path()].id,
            old,
            "{attack}"
        );
        assert!(lock(&fixture.nyc.handle.inner.pending).is_empty());
        fixture.shutdown().await;
    }
}

#[tokio::test]
async fn e2e_recovery_replayed_init_query_and_hint_rate_do_not_reset_keys() {
    let fixture = Fixture::native(TransportKind::NativeTcp).await;
    let mut rx = fixture.sha.take_packet_receiver().unwrap();
    fixture.nyc.send_packet(path(), vec![1]).await.unwrap();
    receive(&mut rx, &[1]).await;
    let old = fixture.nyc.handle.inner.sending.lock().await[&path()].id;
    hint(&fixture.sha, &path(), old);
    hinted(&fixture.nyc, &path(), old).await;
    let (mut queued, original) = intercept_peer(&fixture.nyc, "lax");
    let source = fixture.nyc.clone();
    let call = tokio::spawn(async move { source.send_packet(path(), vec![2]).await });
    let mut init = queued.recv().await.unwrap();
    let init_frames = init.batch.into_frames();
    let captured_init = Packet::decode(&init_frames[0]).unwrap();
    init.batch = Batch::new(init_frames).unwrap();
    original.send(init).await.unwrap();
    let mut query = queued.recv().await.unwrap();
    let query_frames = query.batch.into_frames();
    let captured_query = Packet::decode(&query_frames[0]).unwrap();
    query.batch = Batch::new(query_frames).unwrap();
    original.send(query).await.unwrap();
    let payload = queued.recv().await.unwrap();
    original.send(payload).await.unwrap();
    call.await.unwrap().unwrap();
    receive(&mut rx, &[2]).await;
    lock(&fixture.nyc.handle.inner.peers)
        .get_mut("lax")
        .unwrap()
        .tx = original;
    // Repeated hints during cooldown cannot create another control handshake.
    for _ in 0..20 {
        hint(&fixture.sha, &path(), old);
    }
    fixture.nyc.send_packet(path(), vec![3]).await.unwrap();
    receive(&mut rx, &[3]).await;
    assert!(queued.try_recv().is_err());
    fixture
        .nyc
        .handle
        .inner
        .send("lax", captured_init.clone())
        .unwrap();
    fixture
        .nyc
        .handle
        .inner
        .send("lax", captured_query.clone())
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1050)).await;
    fixture.nyc.handle.inner.send("lax", captured_init).unwrap();
    fixture
        .nyc
        .handle
        .inner
        .send("lax", captured_query)
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Captured query is not key confirmation for the replayed Init's new
    // responder ephemeral. Existing packet receiver/sender remain functional.
    assert_eq!(
        fixture.nyc.handle.inner.sending.lock().await[&path()].id,
        old
    );
    fixture.nyc.send_packet(path(), vec![4]).await.unwrap();
    receive(&mut rx, &[4]).await;
    assert_eq!(
        fixture.nyc.handle.inner.sending.lock().await[&path()].id,
        old
    );
    assert!(
        fixture
            .sha
            .statistics()
            .native_tcp
            .drop_reasons
            .authentication
            > 0
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn e2e_recovery_retired_ingress_generation_cannot_record_hint() {
    let fixture = Fixture::native(TransportKind::NativeTcp).await;
    let mut rx = fixture.sha.take_packet_receiver().unwrap();
    fixture.nyc.send_packet(path(), vec![1]).await.unwrap();
    receive(&mut rx, &[1]).await;
    let id = fixture.nyc.handle.inner.sending.lock().await[&path()].id;
    let old = generation::Fence::new();
    old.retire();
    fixture.nyc.handle.inner.deliver(Incoming {
        socket_generation: None,
        peer: "lax".into(),
        token: lock(&fixture.nyc.handle.inner.peers)["lax"].token,
        transport: TransportKind::NativeTcp,
        generation: old,
        body: Packet {
            phase: Phase::UnknownSession,
            id,
            path: path(),
            body: b"unknown-session-v1".to_vec(),
        }
        .encode()
        .unwrap(),
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!lock(&fixture.nyc.handle.inner.recovery).hinted(&path(), id));
    fixture.nyc.send_packet(path(), vec![2]).await.unwrap();
    receive(&mut rx, &[2]).await;
    assert_eq!(
        fixture.nyc.handle.inner.sending.lock().await[&path()].id,
        id
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn e2e_recovery_endpoint_query_distinguishes_exact_cipher_not_only_public_id() {
    let fixture = Fixture::native(TransportKind::NativeTcp).await;
    let mut rx = fixture.sha.take_packet_receiver().unwrap();
    fixture.nyc.send_packet(path(), vec![1]).await.unwrap();
    receive(&mut rx, &[1]).await;
    let (id, hash) = {
        let sessions = fixture.nyc.handle.inner.sending.lock().await;
        (sessions[&path()].id, sessions[&path()].cipher.id)
    };
    let mut wrong = hash;
    wrong[0] ^= 1;
    let inner = &fixture.nyc.handle.inner;
    assert!(
        tokio::time::timeout(
            Duration::from_secs(2),
            inner
                .packet_generation()
                .run(recovery::missing(inner, &path(), id, wrong))
        )
        .await
        .unwrap()
        .unwrap()
    );
    assert_eq!(
        fixture.nyc.handle.inner.sending.lock().await[&path()].id,
        id
    );
    tokio::time::sleep(Duration::from_millis(1050)).await;
    assert!(
        !tokio::time::timeout(
            Duration::from_secs(2),
            inner
                .packet_generation()
                .run(recovery::missing(inner, &path(), id, hash))
        )
        .await
        .unwrap()
        .unwrap()
    );
    fixture.nyc.send_packet(path(), vec![2]).await.unwrap();
    receive(&mut rx, &[2]).await;
    assert_eq!(
        fixture.nyc.handle.inner.sending.lock().await[&path()].id,
        id
    );
    fixture.shutdown().await;
}
