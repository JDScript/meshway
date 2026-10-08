use super::*;

#[tokio::test]
async fn real_udp_four_message_loss_shared_listener_and_unpolled_output_fence() {
    let mut fixture = support::Fixture::new(false, false).await;
    let ids = fixture.ids();
    let faults = Arc::new(Faults::default());
    *faults.drops.lock().unwrap() = BTreeSet::from([1, 2, 3, 4]);
    let factory = Factory::new(&mut fixture, faults.clone());
    let limits = Limits {
        session: meshway_transport::Limits::new(4096, 16, 65_536, 16, 16).unwrap(),
        ..Default::default()
    };
    let mut b = Manager::start(fixture.identities[1].clone(), factory.clone(), limits)
        .await
        .unwrap();
    let mut a = Manager::start(
        fixture.identities[0].clone(),
        factory.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    linked(&a, 1, TransportProtocol::NativeUdp).await;
    let mut c = Manager::start(fixture.identities[2].clone(), factory, Default::default())
        .await
        .unwrap();
    linked(&c, 1, TransportProtocol::NativeUdp).await;
    linked(&b, 2, TransportProtocol::NativeUdp).await;
    let facts = b.snapshot().unwrap();
    assert_eq!(facts.links[0].socket_id, facts.links[1].socket_id);
    assert_eq!(facts.listeners[0].socket_id, facts.links[0].socket_id);
    assert!(faults.drops.lock().unwrap().is_empty());

    // Unassociated sources cannot inject Mesh frames or replay a genuine ACK
    // into a different source tuple. Empty/oversize datagrams do not kill peers.
    let stranger = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = facts.listeners[0].local;
    let ack = faults
        .controls
        .lock()
        .unwrap()
        .iter()
        .find(|(kind, _, _)| *kind == 3)
        .unwrap()
        .2
        .clone();
    // Above the adjacency bound while still below macOS's UDP send limit.
    let mut oversized = vec![0; adjacency::MAX_MESSAGE + 1];
    oversized[..8].copy_from_slice(b"MWADJv1\0");
    for bytes in [vec![], b"MWCIR-invalid".to_vec(), ack, oversized] {
        stranger.send_to(&bytes, address).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(b.snapshot().unwrap().links.len(), 2);

    let mut secure = meshway_dataplane::secure::SecureNode::new(
        fixture.identities[0].clone(),
        Default::default(),
        a.carriers().unwrap(),
    )
    .unwrap();
    let pending = secure
        .start(fixture.demands[0], vec![ids[0], ids[1]])
        .unwrap();
    faults.block_data.store(true, Ordering::SeqCst);
    let before = faults.writes.load(Ordering::SeqCst);
    let mut send = Box::pin(a.send(pending));
    assert!(futures_util::poll!(send.as_mut()).is_pending());
    assert!(faults.blocked.load(Ordering::SeqCst));
    let scope = a
        .shared
        .links
        .lock()
        .unwrap()
        .get(&ids[1])
        .unwrap()
        .scope
        .clone();
    scope.cancellation.cancel();
    faults.block_data.store(false, Ordering::SeqCst);
    faults.waker.wake();
    // Deliberately keep `send` alive without polling it again.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(faults.writes.load(Ordering::SeqCst), before);
    drop(send);

    // The other peer still uses the same listener descriptor and its own guard.
    let mut secure_c = meshway_dataplane::secure::SecureNode::new(
        fixture.identities[2].clone(),
        Default::default(),
        c.carriers().unwrap(),
    )
    .unwrap();
    let pending = secure_c
        .start(fixture.demands[1], vec![ids[2], ids[1]])
        .unwrap();
    c.send(pending).await.unwrap();
    let (link, received) = frame(&mut b).await;
    assert_eq!(link.peer, ids[2]);
    let mut secure_b = meshway_dataplane::secure::SecureNode::new(
        fixture.identities[1].clone(),
        Default::default(),
        b.carriers().unwrap(),
    )
    .unwrap();
    assert!(matches!(
        secure_b.receive(&link.carrier, received.payload).unwrap(),
        meshway_dataplane::secure::Received::Established { .. }
    ));
    for manager in [&mut a, &mut b, &mut c] {
        manager.shutdown().await.unwrap();
    }
    fixture.close().await;
}

pub(super) async fn real_ipv6_udp_blackhole_selects_allowed_stream_without_replacing_secure_owner()
{
    let mut fixture = support::Fixture::new(true, true).await;
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
    linked(&a, 1, TransportProtocol::NativeUdp).await;
    linked(&b, 1, TransportProtocol::NativeUdp).await;
    let mut secure_a = meshway_dataplane::secure::SecureNode::new(
        fixture.identities[0].clone(),
        Default::default(),
        a.carriers().unwrap(),
    )
    .unwrap();
    let mut secure_b = meshway_dataplane::secure::SecureNode::new(
        fixture.identities[1].clone(),
        Default::default(),
        b.carriers().unwrap(),
    )
    .unwrap();
    let pending = secure_a
        .start(fixture.demands[0], vec![ids[0], ids[1]])
        .unwrap();
    let circuit = pending.circuit();
    a.send(pending).await.unwrap();
    let (link, received) = frame(&mut b).await;
    let meshway_dataplane::secure::Received::Established {
        response: Some(response),
        ..
    } = secure_b.receive(&link.carrier, received.payload).unwrap()
    else {
        panic!("end-to-end handshake");
    };
    b.send(response).await.unwrap();
    let (link, received) = frame(&mut a).await;
    assert!(matches!(
        secure_a.receive(&link.carrier, received.payload).unwrap(),
        meshway_dataplane::secure::Received::Established { .. }
    ));
    faults.blackhole.store(true, Ordering::SeqCst);
    linked(&a, 1, TransportProtocol::NativeTcp).await;
    linked(&b, 1, TransportProtocol::NativeTcp).await;
    secure_a.set_carriers(a.carriers().unwrap()).unwrap();
    secure_b.set_carriers(b.carriers().unwrap()).unwrap();
    assert!(
        secure_a
            .ready_circuits()
            .unwrap()
            .iter()
            .any(|ready| ready.id == circuit)
    );
    faults.blackhole.store(false, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        a.snapshot().unwrap().links[0].carrier.protocol,
        TransportProtocol::NativeTcp
    );
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

pub(super) async fn simultaneous_configured_initiators_converge_on_one_logical_carrier() {
    let mut fixture = support::Fixture::reverse().await;
    let faults = Arc::new(Faults::default());
    let factory = Factory::new(&mut fixture, faults);
    let (a, b) = tokio::join!(
        Manager::start(
            fixture.identities[0].clone(),
            factory.clone(),
            Default::default()
        ),
        Manager::start(fixture.identities[1].clone(), factory, Default::default()),
    );
    let (mut a, mut b) = (a.unwrap(), b.unwrap());
    linked(&a, 1, TransportProtocol::NativeUdp).await;
    linked(&b, 1, TransportProtocol::NativeUdp).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let a = a.snapshot().unwrap();
            let b = b.snapshot().unwrap();
            if a.links.len() == 1 && b.links.len() == 1 && a.links[0].id == b.links[0].id {
                assert_eq!(a.links[0].carrier, b.links[0].carrier);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let id = a.snapshot().unwrap().links[0].id;
    tokio::time::sleep(Duration::from_secs(7)).await;
    assert_eq!(a.snapshot().unwrap().links[0].id, id);
    assert_eq!(b.snapshot().unwrap().links[0].id, id);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}
