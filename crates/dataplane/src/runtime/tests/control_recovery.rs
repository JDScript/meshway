use super::security::{capacity_one_fixture, intercept_peer};
use super::*;

#[tokio::test(start_paused = true)]
async fn retried_probe_cancellation_and_shutdown_leave_no_timer_or_pending_slot() {
    for shutdown in [false, true] {
        let fixture=capacity_one_fixture(3_000).await;
        let token=lock(&fixture.nyc.handle.inner.peers)["lax"].token;
        let (mut requests,_original)=intercept_peer(&fixture.nyc,"lax");
        let source=fixture.nyc.clone();
        let call=tokio::spawn(async move {source.probe_neighbor_observed("lax",Duration::from_secs(3)).await});
        let first=requests.recv().await.unwrap().batch.into_frames();
        let retry=requests.recv().await.unwrap().batch.into_frames();
        assert_eq!(first,retry);
        assert_eq!(lock(&fixture.nyc.handle.inner.neighbor_pending).len(),1);
        if shutdown {
            fixture.nyc.shutdown().await;
            assert!(matches!(call.await.unwrap(),Err(Error::Closed)));
        } else {
            call.abort();
            assert!(call.await.unwrap_err().is_cancelled());
        }
        tokio::time::advance(Duration::from_secs(4)).await;
        tokio::task::yield_now().await;
        assert!(lock(&fixture.nyc.handle.inner.neighbor_pending).is_empty());
        assert!(requests.try_recv().is_err(),"cancelled retry must not send later");
        if shutdown {
            assert!(fixture.nyc.peer_connections().is_empty());
        } else {
            assert_eq!(lock(&fixture.nyc.handle.inner.peers)["lax"].token,token);
        }
        fixture.shutdown().await;
    }
}

#[tokio::test]
async fn native_acceptor_control_loss_trace_has_no_spurious_blackout() {
    let mut configs = configs();
    let direct = vec!["nyc".to_owned(), "sha".to_owned()];
    for config in &mut configs {
        config.allowed_paths = vec![direct.clone()];
    }
    configs[0].listeners.push(ListenerConfig {
        transport: TransportKind::NativeUdp,
        bind: "127.0.0.1:0".parse().unwrap(),
        id: None,
        allowed_peers: Some(vec!["sha".into()]),
    });
    let nyc = Runtime::start(configs[0].clone()).await.unwrap();
    configs[2].outgoing.push(OutgoingCandidate {
        peer: "nyc".into(),
        transport: TransportKind::NativeUdp,
        endpoint: nyc.bound_listeners()[0].address.to_string(),
        priority: 0,
        id: None,
    });
    let sha = Runtime::start(configs[2].clone()).await.unwrap();
    nyc.wait_for_peer("sha", Duration::from_secs(3))
        .await
        .unwrap();
    sha.wait_for_peer("nyc", Duration::from_secs(3))
        .await
        .unwrap();
    let mut receiver = sha.take_packet_receiver().unwrap();
    nyc.send_packet(direct.clone(), vec![1]).await.unwrap();
    assert_eq!(receiver.recv().await.unwrap().payload, vec![1]);
    let token = lock(&nyc.handle.inner.peers)["sha"].token;
    let key = nyc.handle.inner.sending.lock().await[&direct].id;
    let (mut queued, original) = intercept_peer(&sha, "nyc");
    let (dropped, seen) = tokio::sync::oneshot::channel();
    let started = Instant::now();
    let forwarder = tokio::spawn(async move {
        let mut dropped = Some(dropped);
        while let Some(outbound) = queued.recv().await {
            let frames = outbound.batch.into_frames();
            if frames.len() == 1
                && frames[0].starts_with(&[neighbor::NAMESPACE, 2])
                && let Some(signal) = dropped.take()
            {
                signal.send(started.elapsed()).unwrap();
                continue;
            }
            if original
                .send(Outbound {
                    token: outbound.token,
                    batch: Batch::new(frames).unwrap(),
                    generation: outbound.generation,
                })
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let source = nyc.clone();
    let probe = tokio::spawn(async move {
        source
            .probe_neighbor_observed("sha", Duration::from_millis(300))
            .await
    });
    let reply_dropped = seen.await.unwrap();
    nyc.send_packet(direct.clone(), vec![2]).await.unwrap();
    assert_eq!(receiver.recv().await.unwrap().payload, vec![2]);
    let business_delivered = started.elapsed();
    let result = probe.await.unwrap();
    let probe_finished = started.elapsed();
    let retired = nyc.peer_transport("sha").is_none();
    let send_while_missing = if retired {
        Some(nyc.send_packet(direct.clone(), vec![3]).await)
    } else {
        None
    };
    // The acceptor has no outgoing candidate. On the broken implementation its
    // healthy dialer still retains the old channel until the six-second timer.
    nyc.wait_for_peer("sha", Duration::from_secs(9))
        .await
        .unwrap();
    let restored = started.elapsed();
    nyc.send_packet(direct.clone(), vec![4]).await.unwrap();
    assert_eq!(receiver.recv().await.unwrap().payload, vec![4]);
    let same_channel = lock(&nyc.handle.inner.peers)["sha"].token == token;
    let same_e2e_cache = nyc.handle.inner.sending.lock().await[&direct].id == key;
    println!(
        "{}",
        serde_json::json!({
            "scope":"bounded native UDP trace; one deliberately lost reply, no lease/config changes",
            "reply_lost_ms":reply_dropped.as_secs_f64()*1000.,
            "authenticated_business_delivered_ms":business_delivered.as_secs_f64()*1000.,
            "probe_finished_ms":probe_finished.as_secs_f64()*1000.,
            "probe_result":format!("{:?}",result.as_ref().map(|_|())),
            "channel_retired":retired,"peer_available_ms":restored.as_secs_f64()*1000.,
            "send_while_missing":format!("{send_while_missing:?}"),
            "same_channel":same_channel,
            "same_e2e_cache":same_e2e_cache,
            "generation_retired":nyc.handle.inner.packet_generation().retired(),
            "statistics":nyc.statistics(),
        })
    );
    nyc.shutdown().await;
    sha.shutdown().await;
    forwarder.await.unwrap();
    assert!(
        result.is_ok(),
        "one lost reply caused healthy native UDP channel retirement and multi-second outage"
    );
    assert!(!retired);
}

#[tokio::test]
async fn one_lost_neighbor_reply_does_not_retire_a_working_channel() {
    let fixture = capacity_one_fixture(3_000).await;
    let mut receiver = fixture.sha.take_packet_receiver().unwrap();
    fixture.nyc.send_packet(path(), vec![1]).await.unwrap();
    assert_eq!(receiver.recv().await.unwrap().payload, vec![1]);
    let token = lock(&fixture.nyc.handle.inner.peers)["lax"].token;
    let (mut queued, original) = intercept_peer(&fixture.lax, "nyc");
    let (dropped, observed) = tokio::sync::oneshot::channel();
    let forwarder = tokio::spawn(async move {
        let mut dropped = Some(dropped);
        while let Some(outbound) = queued.recv().await {
            let frames = outbound.batch.into_frames();
            if frames.len() == 1
                && frames[0].starts_with(&[neighbor::NAMESPACE, 2])
                && let Some(signal) = dropped.take()
            {
                signal.send(()).unwrap();
                continue;
            }
            original
                .send(Outbound {
                    token: outbound.token,
                    batch: Batch::new(frames).unwrap(),
                    generation: outbound.generation,
                })
                .await
                .unwrap();
        }
    });
    let source = fixture.nyc.clone();
    let probe = tokio::spawn(async move {
        source
            .probe_neighbor_observed("lax", Duration::from_millis(300))
            .await
    });
    observed.await.unwrap();
    // Strong evidence that this same carrier still delivers real E2E traffic
    // while precisely one authenticated control reply is lost.
    fixture.nyc.send_packet(path(), vec![2]).await.unwrap();
    assert_eq!(receiver.recv().await.unwrap().payload, vec![2]);
    let observation = probe
        .await
        .unwrap()
        .expect("one lost reply retired a working carrier");
    assert!(observation.round_trip < Duration::from_millis(300));
    assert_eq!(lock(&fixture.nyc.handle.inner.peers)["lax"].token, token);
    fixture.nyc.send_packet(path(), vec![3]).await.unwrap();
    assert_eq!(receiver.recv().await.unwrap().payload, vec![3]);
    fixture.shutdown().await;
    forwarder.await.unwrap();
}

#[tokio::test]
async fn neighbor_reply_recovers_after_real_business_queue_saturation() {
    let fixture = capacity_one_fixture(3_000).await;
    let reverse: Vec<_> = path().into_iter().rev().collect();
    let mut received = fixture.nyc.take_packet_receiver().unwrap();
    fixture
        .sha
        .send_packet(reverse.clone(), vec![1])
        .await
        .unwrap();
    assert_eq!(received.recv().await.unwrap().payload, vec![1]);
    let token = lock(&fixture.nyc.handle.inner.peers)["lax"].token;
    let (mut queued, original) = intercept_peer(&fixture.lax, "nyc");
    let capacity = queued.max_capacity();
    for _ in 0..capacity / 16 {
        fixture
            .sha
            .send_packet_batch(reverse.clone(), vec![vec![7]; 16])
            .await
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(1), async {
        while queued.len() != capacity {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let before = fixture
        .lax
        .statistics()
        .native_tcp
        .drop_reasons
        .outbound_queue_full;
    let source = fixture.nyc.clone();
    let probe = tokio::spawn(async move {
        source
            .probe_neighbor_observed("lax", Duration::from_millis(600))
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while fixture
            .lax
            .statistics()
            .native_tcp
            .drop_reasons
            .outbound_queue_full
            == before
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // The real enqueue rejected the reply behind bounded authenticated business
    // traffic. Restore that same channel; no key, token, lease or grant changes.
    lock(&fixture.lax.handle.inner.peers)
        .get_mut("nyc")
        .unwrap()
        .tx = original.clone();
    for _ in 0..capacity {
        original.send(queued.recv().await.unwrap()).await.unwrap();
    }
    for _ in 0..capacity {
        assert_eq!(received.recv().await.unwrap().payload, vec![7]);
    }
    probe
        .await
        .unwrap()
        .expect("brief queue saturation retired a working carrier");
    assert_eq!(lock(&fixture.nyc.handle.inner.peers)["lax"].token, token);
    fixture.shutdown().await;
}
