use super::*;

fn relay_packet() -> Packet {
    Packet {
        phase: Phase::Packet,
        id: [17; 16],
        path: path(),
        // The relay must treat endpoint ciphertext as opaque.
        body: vec![42; 24],
    }
}

fn queue_channel(
    runtime: &Runtime,
    peer: &str,
    transport: TransportKind,
) -> mpsc::Receiver<Outbound> {
    queue_channel_with_capacity(
        runtime,
        peer,
        transport,
        runtime.handle.inner.config.limits.queue_capacity,
    )
}

fn queue_channel_with_capacity(
    runtime: &Runtime,
    peer: &str,
    transport: TransportKind,
    capacity: usize,
) -> mpsc::Receiver<Outbound> {
    let (tx, rx) = mpsc::channel(capacity);
    let (close, _) = watch::channel(false);
    runtime
        .handle
        .inner
        .register(
            peer.into(),
            Channel {
                token: [17; 32],
                priority: 0,
                transport,
                tx,
                close,
                initiator: true,
                context: Default::default(),
                probe_budget: Default::default(),
            },
        )
        .unwrap();
    rx
}

fn ingress(runtime: &Runtime) {
    runtime.handle.inner.deliver(Incoming {
        socket_generation: None,
        generation: runtime.handle.inner.packet_generation(),
        peer: "nyc".into(),
        token: [17; 32],
        transport: TransportKind::NativeUdp,
        body: relay_packet().encode().unwrap(),
    });
}

async fn wait_for(runtime: &Runtime, condition: impl Fn(&Statistics) -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !condition(&runtime.statistics()) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

fn assert_sum(statistics: &Statistics) {
    for transport in [
        &statistics.native_udp,
        &statistics.native_tcp,
        &statistics.vless,
    ] {
        assert_eq!(transport.dropped, transport.drop_reasons.total());
    }
}

#[tokio::test]
async fn queue_boundaries_preserve_ingress_and_outgoing_attribution_without_double_counting() {
    let mut config = configs().remove(1);
    config.limits.queue_capacity = 1;
    let lax = Runtime::start(config).await.unwrap();
    // Deterministic admission fixture: authenticated registry entries with deliberately
    // unpolled carrier queues. No scheduler timing, sockets, or fake crypto errors.
    let _nyc = queue_channel(&lax, "nyc", TransportKind::NativeUdp);
    let mut sha = queue_channel(&lax, "sha", TransportKind::Vless);
    lax.handle.inner.send("sha", relay_packet()).unwrap();
    ingress(&lax);
    ingress(&lax); // current-thread executor cannot drain ingress before this attempt

    let full = lax.statistics();
    assert_eq!(full.queues.ingress.capacity, 1);
    assert_eq!(full.queues.ingress.occupancy, 1);
    assert_eq!(full.queues.ingress.high_water, 1);
    assert_eq!(full.vless.outbound_queue.occupancy, 1);
    assert_eq!(full.vless.outbound_queue.high_water, 1);
    assert_eq!(full.native_udp.received, 2);
    assert_eq!(full.native_udp.dropped, 1);
    assert_eq!(full.native_udp.drop_reasons.ingress_queue_full, 1);
    assert_eq!(full.native_udp.drop_reasons.authentication, 0);
    assert_sum(&full);

    wait_for(&lax, |s| s.native_udp.drop_reasons.outbound_queue_full == 1).await;
    let rejected = lax.statistics();
    assert_eq!(rejected.native_udp.dropped, 2);
    assert_eq!(rejected.native_udp.forwarded, 0);
    assert_eq!(rejected.vless.dropped, 0);
    assert_eq!(rejected.vless.outbound_queue_rejections.full, 1);
    assert_eq!(rejected.native_udp.outbound_queue_rejections.full, 0);
    assert_sum(&rejected);

    sha.recv().await.unwrap();
    ingress(&lax);
    let forwarded = sha.recv().await.unwrap();
    assert_eq!(forwarded.batch.len(), 1);
    assert_eq!(lax.statistics().native_udp.forwarded, 1);
    assert_eq!(lax.statistics().vless.outbound_queue.occupancy, 0);
    assert_eq!(lax.statistics().vless.outbound_queue.high_water, 1);

    drop(sha);
    ingress(&lax);
    wait_for(&lax, |s| {
        s.native_udp.drop_reasons.outbound_queue_closed == 1
    })
    .await;
    assert_eq!(lax.statistics().native_udp.dropped, 3);
    assert_eq!(lax.statistics().vless.dropped, 0);
    assert_eq!(lax.statistics().vless.outbound_queue_rejections.closed, 1);
    // A local producer owns outgoing attribution instead of ingress attribution.
    assert_eq!(
        lax.handle.inner.send("sha", relay_packet()),
        Err(Error::Unavailable)
    );
    assert_eq!(lax.statistics().vless.drop_reasons.outbound_queue_closed, 1);
    assert_eq!(lax.statistics().vless.outbound_queue_rejections.closed, 2);

    lax.handle.inner.unregister("nyc", &[17; 32]);
    ingress(&lax);
    wait_for(&lax, |s| s.native_udp.drop_reasons.stale_channel == 1).await;
    assert_eq!(lax.statistics().native_udp.drop_reasons.authentication, 0);
    assert_sum(&lax.statistics());
    lax.shutdown().await;
    ingress(&lax);
    assert_eq!(
        lax.statistics()
            .native_udp
            .drop_reasons
            .ingress_queue_closed,
        1
    );
    assert_sum(&lax.statistics());
}

#[tokio::test]
async fn full_endpoint_delivery_is_distinct_from_bad_crypto_and_replay() {
    let mut configs = configs();
    configs[2].limits.queue_capacity = 1;
    let nyc = Runtime::start(configs[0].clone()).await.unwrap();
    let lax = Runtime::start(configs[1].clone()).await.unwrap();
    let sha = Runtime::start(configs[2].clone()).await.unwrap();
    attach_pair(&nyc, &lax, 0).await;
    attach_pair(&sha, &lax, 0).await;
    let mut delivered = sha.take_packet_receiver().unwrap();

    nyc.send_packet(path(), b"fills delivery".to_vec())
        .await
        .unwrap();
    wait_for(&sha, |s| s.queues.endpoint_delivery.occupancy == 1).await;
    nyc.send_packet(path(), b"overflows delivery".to_vec())
        .await
        .unwrap();
    wait_for(&sha, |s| {
        s.native_tcp.drop_reasons.endpoint_delivery_full == 1
    })
    .await;
    let full = sha.statistics();
    assert_eq!(full.native_tcp.dropped, 1);
    assert_eq!(full.native_tcp.drop_reasons.authentication, 0);
    assert_eq!(full.native_tcp.drop_reasons.capacity_other, 0);
    assert_eq!(full.queues.endpoint_delivery.capacity, 1);
    assert_eq!(full.queues.endpoint_delivery.high_water, 1);
    assert_eq!(delivered.recv().await.unwrap().payload, b"fills delivery");

    let (id, body) = {
        let mut sessions = nyc.handle.inner.sending.lock().await;
        let session = sessions.get_mut(&path()).unwrap();
        (
            session.id,
            session.cipher.seal(b"valid after forgery").unwrap(),
        )
    };
    let mut forged = body.clone();
    *forged.last_mut().unwrap() ^= 1;
    let send = |body| {
        nyc.handle.inner.send(
            "lax",
            Packet {
                phase: Phase::Packet,
                id,
                path: path(),
                body,
            },
        )
    };
    send(forged).unwrap();
    wait_for(&sha, |s| s.native_tcp.drop_reasons.authentication == 1).await;
    assert_eq!(sha.statistics().native_tcp.dropped, 2);
    send(body.clone()).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), delivered.recv())
            .await
            .unwrap()
            .unwrap()
            .payload,
        b"valid after forgery"
    );
    send(body).unwrap();
    wait_for(&sha, |s| s.native_tcp.drop_reasons.replay == 1).await;
    assert_eq!(sha.statistics().native_tcp.dropped, 3);

    drop(delivered);
    nyc.send_packet(path(), b"closed delivery".to_vec())
        .await
        .unwrap();
    wait_for(&sha, |s| {
        s.native_tcp.drop_reasons.endpoint_delivery_closed == 1
    })
    .await;
    let final_stats = sha.statistics();
    assert_eq!(final_stats.native_tcp.dropped, 4);
    assert_sum(&final_stats);
    let json = serde_json::to_value(final_stats).unwrap();
    assert_eq!(
        json["native_tcp"]["drop_reasons"]["endpoint_delivery_full"],
        1
    );
    assert_eq!(json["queues"]["endpoint_delivery"]["high_water"], 1);
    Fixture { nyc, lax, sha }.shutdown().await;
}

fn drain_frames(mut receiver: mpsc::Receiver<Outbound>) -> tokio::task::JoinHandle<usize> {
    tokio::spawn(async move {
        let mut frames = 0;
        while let Some(outbound) = receiver.recv().await {
            frames += outbound.batch.len();
        }
        frames
    })
}

#[tokio::test(flavor = "current_thread")]
async fn cooperative_producer_budget_allows_progress_during_a_burst_larger_than_ingress() {
    const CAPACITY: usize = 8;
    const BURST: usize = 64;
    for budget in [0, 2] {
        let mut config = configs().remove(1);
        config.limits.queue_capacity = CAPACITY;
        config.limits.work_budget_frames = budget;
        let lax = Runtime::start(config).await.unwrap();
        let _nyc = queue_channel(&lax, "nyc", TransportKind::NativeUdp);
        let consumer = drain_frames(queue_channel(&lax, "sha", TransportKind::Vless));
        let mut work = work_budget::WorkBudget::new(budget);

        // Model a continuously ready UDP producer at the authenticated ingress boundary.
        // All attempts stay in this poll when disabled: no sockets/timers can accidentally
        // provide fairness. Enabled uses the identical helper as the real UDP engine.
        for _ in 0..BURST {
            ingress(&lax);
            work.completed_frame().await;
        }
        let during = lax.statistics();
        let expected = if budget == 0 {
            assert_eq!(during.native_udp.forwarded, 0);
            assert_eq!(
                during.native_udp.drop_reasons.ingress_queue_full,
                (BURST - CAPACITY) as u64
            );
            CAPACITY
        } else {
            assert!(
                during.native_udp.forwarded > 0,
                "consumer must progress before producer finishes"
            );
            assert_eq!(during.native_udp.dropped, 0);
            assert_eq!(during.vless.outbound_queue_rejections.full, 0);
            BURST
        };
        wait_for(&lax, |s| s.native_udp.forwarded == expected as u64).await;
        let finished = lax.statistics();
        assert_eq!(finished.native_udp.received, BURST as u64);
        assert_eq!(finished.native_udp.drop_reasons.authentication, 0);
        assert_eq!(finished.native_udp.drop_reasons.outbound_queue_full, 0);
        assert_eq!(finished.native_udp.dropped, (BURST - expected) as u64);
        assert_sum(&finished);
        lax.shutdown().await;
        assert_eq!(consumer.await.unwrap(), expected);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cooperative_dispatch_budget_allows_outgoing_consumer_to_drain_a_ready_burst() {
    const BURST: usize = 8;
    const OUTGOING_CAPACITY: usize = 2;
    for budget in [0, 1] {
        let mut config = configs().remove(1);
        config.limits.queue_capacity = BURST;
        config.limits.work_budget_frames = budget;
        let lax = Runtime::start(config).await.unwrap();
        let _nyc = queue_channel(&lax, "nyc", TransportKind::NativeUdp);
        // Deliberately smaller test carrier queue isolates dispatch fairness from the
        // producer: prefill ingress before any task is polled, then run a ready consumer.
        let consumer = drain_frames(queue_channel_with_capacity(
            &lax,
            "sha",
            TransportKind::Vless,
            OUTGOING_CAPACITY,
        ));
        for _ in 0..BURST {
            ingress(&lax);
        }
        wait_for(&lax, |s| {
            s.native_udp.forwarded + s.native_udp.dropped == BURST as u64
        })
        .await;
        let finished = lax.statistics();
        let expected = if budget == 0 {
            OUTGOING_CAPACITY
        } else {
            BURST
        };
        assert_eq!(finished.native_udp.forwarded, expected as u64);
        assert_eq!(
            finished.native_udp.drop_reasons.outbound_queue_full,
            (BURST - expected) as u64
        );
        assert_eq!(
            finished.vless.outbound_queue_rejections.full,
            (BURST - expected) as u64
        );
        assert_eq!(finished.native_udp.drop_reasons.ingress_queue_full, 0);
        assert_eq!(finished.native_udp.drop_reasons.authentication, 0);
        assert_eq!(finished.vless.dropped, 0);
        assert_sum(&finished);
        lax.shutdown().await;
        assert_eq!(consumer.await.unwrap(), expected);
    }
}

#[test]
fn work_budget_defaults_and_same_image_ab_config_are_bounded() {
    let mut value = serde_json::to_value(configs().remove(0)).unwrap();
    value["limits"]
        .as_object_mut()
        .unwrap()
        .remove("work_budget_frames");
    let defaulted: LabConfig = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(defaulted.limits.work_budget_frames, 16);
    defaulted.validate().unwrap();
    for budget in [0, 1, 16, 64] {
        value["limits"]["work_budget_frames"] = serde_json::json!(budget);
        let config: LabConfig = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(config.limits.work_budget_frames, budget);
        config.validate().unwrap();
    }
    value["limits"]["work_budget_frames"] = serde_json::json!(65);
    let config: LabConfig = serde_json::from_value(value).unwrap();
    assert_eq!(
        config.validate(),
        Err(Error::InvalidConfig("invalid runtime limits"))
    );
}
