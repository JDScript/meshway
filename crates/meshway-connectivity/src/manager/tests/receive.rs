use super::*;

pub(super) async fn udp_and_stream_receive_timestamp_precedes_delayed_consumer_poll() {
    // The sender lock and actual queued event establish the ordering. There is
    // no minimum scheduling delay and no lease-sized sleep in this contract.
    for protocol in [TransportProtocol::NativeUdp, TransportProtocol::NativeTcp] {
        let mut fixture = support::Fixture::builder()
            .stream(protocol == TransportProtocol::NativeTcp)
            .build()
            .await;
        let ids = fixture.ids();
        let faults = Arc::new(Faults::default());
        faults
            .blackhole
            .store(protocol == TransportProtocol::NativeTcp, Ordering::SeqCst);
        let factory = Factory::new(&mut fixture, faults);
        let mut b = fixture.manager(1, factory.clone()).start().await;
        let mut a = fixture.manager(0, factory.clone()).start().await;
        linked(&a, 1, protocol).await;
        linked(&b, 1, protocol).await;
        let a_link = a.snapshot().unwrap().links[0].clone();
        let b_link = b.snapshot().unwrap().links[0].clone();
        assert_eq!(a_link.acquisition, LinkAcquisition::PublishedEndpoint);
        assert_eq!(b_link.acquisition, LinkAcquisition::AcceptedInbound);
        while b.events.try_recv().is_ok() {}
        while b.control_events.try_recv().is_ok() {}
        let mut secure = meshway_dataplane::secure::SecureNode::new(
            fixture.identities[0].clone(),
            Default::default(),
            a.carriers().unwrap(),
        )
        .unwrap();
        let pending = secure
            .start(fixture.demands[0], vec![ids[0], ids[1]])
            .unwrap();
        let before = std::time::Instant::now();
        let sender = a.shared.links.lock().unwrap()[&ids[1]].sender.clone();
        let locked = sender.lock().await;
        let mut sending = Box::pin(a.send_with_receipt(pending));
        assert!(futures_util::poll!(sending.as_mut()).is_pending());
        tokio::task::yield_now().await;
        let unlocked_at = std::time::Instant::now();
        drop(locked);
        let mut sends = JoinSet::new();
        sends.spawn(sending);
        let receipt = sends.join_next().await.unwrap().unwrap().unwrap();
        assert!(receipt.admitted_at >= unlocked_at);
        assert_eq!(receipt.link, a_link);
        tokio::time::timeout_at(b.shared.identity.snapshot().expires_at().into(), async {
            while b.control_events.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let queued_at = std::time::Instant::now();
        tokio::task::yield_now().await;
        let Event::Frame {
            link,
            frame,
            received_at,
        } = b.recv().await
        else {
            panic!("completed frame must already be queued before consumer polling");
        };
        assert_eq!(*link, b_link);
        assert!(received_at >= before);
        assert!(received_at >= receipt.admitted_at);
        assert!(received_at <= queued_at);
        let mut receiver = meshway_dataplane::secure::SecureNode::new(
            fixture.identities[1].clone(),
            Default::default(),
            b.carriers().unwrap(),
        )
        .unwrap();
        assert!(matches!(
            receiver.receive(&link.carrier, frame.payload).unwrap(),
            meshway_dataplane::secure::Received::Established { .. }
        ));
        a.shutdown().await.unwrap();
        b.shutdown().await.unwrap();
        fixture.close().await;
    }
}

#[tokio::test]
async fn udp_burst_yields_to_a_one_slot_event_consumer_without_growing_the_queue() {
    let mut fixture = support::Fixture::builder().build().await;
    let ids = fixture.ids();
    let factory = Factory::new(&mut fixture, Arc::new(Faults::default()));
    let mut b = fixture
        .manager(1, factory.clone())
        .limits(Limits {
            events: 1,
            ..Default::default()
        })
        .start()
        .await;
    let mut a = fixture.manager(0, factory.clone()).start().await;
    linked(&a, 1, TransportProtocol::NativeUdp).await;
    linked(&b, 1, TransportProtocol::NativeUdp).await;
    while b.events.try_recv().is_ok() {}
    while b.control_events.try_recv().is_ok() {}
    let fact = b.snapshot().unwrap().links[0].clone();
    b.shared
        .events
        .try_send(Event::Frame {
            link: Box::new(fact),
            frame: meshway_transport::Frame::new(vec![3; 9], None),
            received_at: std::time::Instant::now(),
        })
        .unwrap();
    b.shared.control_events.try_send(Event::Changed).unwrap();
    assert!(matches!(
        b.recv_batch_with_data(false).await.as_slice(),
        [Event::Changed]
    ));
    assert_eq!(
        b.events.len(),
        1,
        "backpressure must retain unconsumed data"
    );
    assert!(matches!(b.recv().await, Event::Frame { .. }));
    // Register the receiving task before the burst. This test concerns the
    // authenticated carrier's opaque payload queue, not IP packet admission.
    assert!(futures_util::poll!(Box::pin(b.recv())).is_pending());
    let (sender, scope, remote) = {
        let links = a.shared.links.lock().unwrap();
        let link = &links[&ids[1]];
        (link.sender.clone(), link.scope.clone(), link.fact.remote)
    };
    let mut frames = (0..4u8)
        .map(|n| {
            let mut bytes = vec![0xff; 10];
            bytes[8] = 3;
            bytes[9] = n;
            meshway_transport::Frame::new(bytes, Some(remote))
        })
        .collect::<std::collections::VecDeque<_>>();
    let admitted = sender.lock().await.try_send(
        &mut frames,
        meshway_transport::SendContext {
            cancellation: scope.cancellation.clone(),
            deadline: Some(scope.deadline),
            lifetime: Some(Arc::new(scope)),
        },
    );
    assert_eq!(admitted.accepted, 4);
    admitted.completion.unwrap().wait().await.unwrap();
    let received = tokio::time::timeout(Duration::from_secs(2), async {
        let mut frames = Vec::new();
        while frames.len() < 4 {
            if let Event::Frame { frame, .. } = b.recv().await {
                frames.push(frame.payload.into_vec());
            }
        }
        frames
    })
    .await
    .unwrap();
    assert_eq!(
        received,
        (0..4u8)
            .map(|n| {
                let mut bytes = vec![0xff; 10];
                bytes[8] = 3;
                bytes[9] = n;
                bytes
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(b.events.max_capacity(), 1);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn receive_batch_is_bounded_ordered_and_fences_queued_frames_after_control_stop() {
    let mut fixture = support::Fixture::builder().build().await;
    let ids = fixture.ids();
    let factory = Factory::new(&mut fixture, Arc::new(Faults::default()));
    let limits = Limits {
        events: 64,
        session: meshway_transport::Limits::new(128 * 1024, 64, 1024 * 1024, 64, 64).unwrap(),
        ..Default::default()
    };
    let mut b = fixture
        .manager(1, factory.clone())
        .limits(limits)
        .start()
        .await;
    let mut a = fixture
        .manager(0, factory.clone())
        .limits(limits)
        .start()
        .await;
    linked(&a, 1, TransportProtocol::NativeUdp).await;
    linked(&b, 1, TransportProtocol::NativeUdp).await;
    while b.events.try_recv().is_ok() {}
    while b.control_events.try_recv().is_ok() {}
    let (sender, scope, remote) = {
        let links = a.shared.links.lock().unwrap();
        let link = &links[&ids[1]];
        (link.sender.clone(), link.scope.clone(), link.fact.remote)
    };
    let mut frames = (0..40u8)
        .map(|n| Frame::new(vec![0xff, n], Some(remote)))
        .collect();
    let admitted = sender.lock().await.try_send(
        &mut frames,
        meshway_transport::SendContext {
            cancellation: scope.cancellation.clone(),
            deadline: Some(scope.deadline),
            lifetime: Some(Arc::new(scope)),
        },
    );
    assert_eq!(admitted.accepted, 40);
    admitted.completion.unwrap().wait().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while b.events.len() < 40 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let first = b.recv_batch().await;
    assert_eq!(first.len(), 32);
    let payloads: Vec<_> = first
        .into_iter()
        .map(|event| {
            let Event::Frame { frame, .. } = event else {
                panic!("authenticated carrier data");
            };
            frame.payload.into_vec()
        })
        .collect();
    assert_eq!(
        payloads,
        (0..32u8).map(|n| vec![0xff, n]).collect::<Vec<_>>()
    );
    assert!(b.events.len() >= 8);
    fixture.controls[1].stop().await.unwrap();
    assert!(matches!(b.recv_batch().await.as_slice(), [Event::Closed]));
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}
