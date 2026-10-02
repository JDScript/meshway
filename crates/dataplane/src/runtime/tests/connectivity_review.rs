//! Independent bdc778f review counterexamples. Intentionally assert required behavior.
use super::*;
use crate::crypto::{Cipher, read_handshake, write_handshake};

fn endpoint(socket: &UdpPunchSocket) -> SocketAddr {
    ([127, 0, 0, 1], socket.info.local_addr.port()).into()
}

async fn pair() -> (Runtime, Runtime, UdpPunchSocket, UdpPunchSocket) {
    let config = configs();
    let a = Runtime::start(config[0].clone()).await.unwrap();
    let b = Runtime::start(config[1].clone()).await.unwrap();
    let sa = a
        .open_udp_punch_socket("lax", "review-lax", false)
        .await
        .unwrap();
    let sb = b
        .open_udp_punch_socket("nyc", "review-nyc", false)
        .await
        .unwrap();
    (a, b, sa, sb)
}

#[tokio::test]
async fn review_new_attempt_must_not_succeed_from_an_old_session() {
    let (a, b, sa, sb) = pair().await;
    let (oa, ob) = tokio::join!(
        a.punch_udp(&sa, vec![endpoint(&sb)], 0, Duration::from_secs(2)),
        b.punch_udp(&sb, vec![endpoint(&sa)], 0, Duration::from_secs(2)),
    );
    oa.unwrap();
    ob.unwrap();
    let old_token = lock(&a.handle.inner.peers).get("lax").unwrap().token;
    b.shutdown().await;
    // No peer or reciprocal attempt exists now. A different priority also cannot
    // be satisfied by the old priority-0 channel.
    let result = a
        .punch_udp(&sa, vec![endpoint(&sb)], 7, Duration::from_millis(500))
        .await;
    let reused = lock(&a.handle.inner.peers)
        .get("lax")
        .is_some_and(|c| c.token == old_token);
    eprintln!("new attempt after peer shutdown: {result:?}; reused old token={reused}");
    a.shutdown().await;
    assert!(
        matches!(result, Err(Error::Timeout)),
        "old session is not this attempt's proof"
    );
}

#[tokio::test]
async fn review_queued_success_must_reject_an_exited_socket_engine() {
    let (a, b, sa, sb) = pair().await;
    let mut ca = Box::pin(a.punch_udp(&sa, vec![endpoint(&sb)], 0, Duration::from_secs(2)));
    let mut cb = Box::pin(b.punch_udp(&sb, vec![endpoint(&sa)], 0, Duration::from_secs(2)));
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(ca.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(cb.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while a.handle.inner.udp_punch_slots.available_permits() != 16
            || b.handle.inner.udp_punch_slots.available_permits() != 16
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(a.peer_transport("lax").is_some());
    // Same engine-exit injection used by existing discovery tests. Success is
    // already in the oneshot, but the public caller has not consumed it yet.
    let task = a.handle.native.lock().await.tasks.pop().unwrap();
    task.abort();
    let _ = task.await;
    assert!(a.udp_sockets().is_empty());
    let result = ca.await;
    eprintln!("queued success with empty live socket inventory: {result:?}");
    let _ = cb.await;
    tokio::join!(a.shutdown(), b.shutdown());
    assert!(
        matches!(result, Err(Error::Closed)),
        "closed engine cannot return live socket evidence"
    );
}

#[tokio::test]
async fn review_exited_engines_must_release_dynamic_socket_capacity() {
    let a = Runtime::start(configs().remove(0)).await.unwrap();
    for _ in 0..16 {
        let socket = a
            .open_udp_punch_socket("lax", "review-retry", false)
            .await
            .unwrap();
        let task = a.handle.native.lock().await.tasks.pop().unwrap();
        task.abort();
        let _ = task.await;
        drop(socket);
        assert!(a.udp_sockets().is_empty());
    }
    let result = a
        .open_udp_punch_socket("sha", "review-other-peer", false)
        .await;
    eprintln!(
        "17th allocation with no live UDP sockets: {:?}",
        result.as_ref().map(|s| &s.info)
    );
    a.shutdown().await;
    assert!(
        result.is_ok(),
        "failed engines permanently consumed all slots for another peer"
    );
}

async fn receive_kind(socket: &UdpSocket, kind: u8) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let mut bytes = vec![0; MAX_NATIVE_UDP_DATAGRAM];
            let (n, _) = socket.recv_from(&mut bytes).await.unwrap();
            bytes.truncate(n);
            if bytes.first() == Some(&kind) {
                return bytes;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_cancel_during_confirm_commit_must_not_install() {
    confirm_commit_race(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_deadline_during_confirm_commit_must_not_install() {
    confirm_commit_race(false).await;
}

async fn confirm_commit_race(cancel: bool) {
    let config = configs();
    let a = Runtime::start(config[0].clone()).await.unwrap(); // nyc > lax: responder
    let sa = a
        .open_udp_punch_socket("lax", "review-cancel", false)
        .await
        .unwrap();
    let raw = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let remote = raw.local_addr().unwrap();
    let timeout = if cancel {
        Duration::from_secs(5)
    } else {
        Duration::from_millis(500)
    };
    let mut call = Box::pin(a.punch_udp(&sa, vec![remote], 0, timeout));
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(call.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    let mut call = Some(call);
    receive_kind(&raw, 0).await;
    let identity = Identity::from_private_key(config[1].private_key.clone());
    let key = public_key(&config[0].public_key).unwrap();
    let mut state = identity
        .handshake(Some(&key), &wire::hop_prologue(TransportKind::NativeUdp))
        .unwrap();
    let mut hello = vec![1];
    hello.extend(write_handshake(&mut state, &[1, 0, 0]).unwrap());
    raw.send_to(&hello, endpoint(&sa)).await.unwrap();
    let response = receive_kind(&raw, 2).await;
    read_handshake(&mut state, &response[1..]).unwrap();
    let mut cipher = Cipher::finish(state).unwrap();
    let mut confirm = vec![4];
    confirm.extend_from_slice(&cipher.id);
    confirm.extend(cipher.seal(wire::HOP_CONFIRM).unwrap());

    // Model peers-registry contention across threads during the synchronous
    // confirm/commit section; no production code hooks or implementation edits.
    {
        let _guard = lock(&a.handle.inner.peers);
        raw.try_send_to(&confirm, endpoint(&sa)).unwrap();
        // Keep this section synchronous: a blocked engine worker must not also
        // hold up the timer or scheduling used to cancel the caller.
        std::thread::sleep(Duration::from_millis(if cancel { 100 } else { 600 }));
        if cancel {
            drop(call.take());
        }
    }
    if let Some(call) = call {
        let result = call.await;
        eprintln!("attempt result after commit outlived deadline: {result:?}");
        assert!(matches!(result, Err(Error::Timeout)));
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let peers = a.connected_peers();
    eprintln!("selected peers after confirm commit (cancel={cancel}): {peers:?}");
    a.shutdown().await;
    assert!(
        peers.is_empty(),
        "cancelled/expired attempt installed after its owner finished"
    );
}
