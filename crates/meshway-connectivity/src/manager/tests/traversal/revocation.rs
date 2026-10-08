use super::*;

pub(super) async fn revocation_recovery(indirect_first: Option<bool>) {
    let timeline = AcquisitionTimeline::new();
    let stun = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let stun_address = public(stun.local_addr().unwrap());
    let stop = CancellationToken::new();
    let stopped = stop.clone();
    let service = tokio::spawn(async move {
        let mut bytes = [0; meshway_stun::MAX_DATAGRAM_SIZE];
        loop {
            let (len, source) = tokio::select! { _ = stopped.cancelled() => break, result = stun.recv_from(&mut bytes) => result.unwrap() };
            let request = meshway_stun::parse_request(&bytes[..len]).unwrap();
            stun.send_to(
                &meshway_stun::encode_response(&request, public(source)),
                source,
            )
            .await
            .unwrap();
        }
    });
    let mut fixture = match indirect_first {
        Some(indirect_first) => {
            fixture::Fixture::traversal_ordered(stun_address, indirect_first).await
        }
        None => fixture::Fixture::traversal(stun_address).await,
    };
    let ids = fixture.ids();
    let sockets = [
        fixture.udp.take().unwrap(),
        fixture.reverse_udp.take().unwrap(),
    ]
    .into_iter()
    .map(|socket| (socket.local_addr().unwrap(), socket))
    .collect();
    let factory = Arc::new(NatFactory {
        sockets: Mutex::new(sockets),
        stun: stun_address,
        traffic: Arc::new(Traffic {
            timeline: Some(timeline.clone()),
            ..Default::default()
        }),
    });
    let mut gateways = Vec::new();
    for mut control in fixture.controls.drain(..2) {
        control.acknowledge_saved().await.unwrap();
        let ticket = control.begin_application().await.unwrap();
        control
            .finish_application(&ticket, Ok(()), chrono::Utc::now())
            .await
            .unwrap();
        gateways.push(Arc::new(Gateway {
            queries: AtomicUsize::new(0),
            control: AsyncMutex::new(control),
            acknowledged: Mutex::new(Vec::new()),
            timeline: Some(timeline.clone()),
        }));
    }
    let generation = Sequence::MIN.checked_next().unwrap();
    let mut a = Manager::start_with_traversal(
        fixture.identities[0].clone(),
        factory.clone(),
        Default::default(),
        gateways[0].clone(),
        generation,
    )
    .await
    .unwrap();
    let mut b = Manager::start_with_traversal(
        fixture.identities[1].clone(),
        factory,
        Default::default(),
        gateways[1].clone(),
        generation,
    )
    .await
    .unwrap();
    let mut sa = meshway_dataplane::secure::SecureNode::new(
        fixture.identities[0].clone(),
        Default::default(),
        BTreeMap::new(),
    )
    .unwrap();
    let mut sb = meshway_dataplane::secure::SecureNode::new(
        fixture.identities[1].clone(),
        Default::default(),
        BTreeMap::new(),
    )
    .unwrap();
    let mut old = None;
    for restored in [false, true] {
        timeline.record(format_args!("phase restored={restored}"));
        if restored {
            fixture.set_access(false).await;
            fixture.identities[0] = gateways[0].install(&mut a, &mut sa, false).await;
            fixture.identities[1] = gateways[1].install(&mut b, &mut sb, false).await;
            assert!(a.snapshot().unwrap().links.is_empty());
            assert!(b.snapshot().unwrap().links.is_empty());
            assert!(sa.ready_circuits().unwrap().is_empty());
            fixture.set_access(true).await;
            fixture.identities[0] = gateways[0].install(&mut a, &mut sa, false).await;
            fixture.identities[1] = gateways[1].install(&mut b, &mut sb, false).await;
        }
        for identity in &fixture.identities {
            let cfg = identity.snapshot().configuration().unwrap();
            timeline.record(format_args!("grants restored={restored} node={} instance={:?} execution_expires={} permits={:?}", cfg.node_id, identity.snapshot().instance(), identity.snapshot().expires_at_ms(), cfg.permits));
        }
        let acquisition = tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                let left = a.snapshot().unwrap();
                let right = b.snapshot().unwrap();
                if left.links.len() == 1
                    && right.links.len() == 1
                    && left.links[0].id == right.links[0].id
                {
                    break;
                }
                gateways[0].report_changes(left).await;
                gateways[1].report_changes(right).await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        timeline.record(format_args!(
            "acquisition restored={restored} result={acquisition:?} left={:?} right={:?}",
            a.snapshot(),
            b.snapshot()
        ));
        if acquisition.is_err() {
            timeline.dump();
        }
        acquisition.expect("acquisition after real Server authorization");
        let channel = a.snapshot().unwrap().links[0].id;
        sa.set_carriers(a.carriers().unwrap()).unwrap();
        sb.set_carriers(b.carriers().unwrap()).unwrap();
        let init = sa.start(fixture.demands[0], vec![ids[0], ids[1]]).unwrap();
        let cid = init.circuit();
        a.send(init).await.unwrap();
        let (link, received) = frame(&mut b).await;
        let meshway_dataplane::secure::Received::Established {
            response: Some(response),
            ..
        } = sb.receive(&link.carrier, received.payload).unwrap()
        else {
            panic!("fresh responder");
        };
        b.send(response).await.unwrap();
        let (link, received) = frame(&mut a).await;
        assert!(matches!(
            sa.receive(&link.carrier, received.payload).unwrap(),
            meshway_dataplane::secure::Received::Established { .. }
        ));
        reattachment::exchange(
            &a,
            &mut b,
            &mut sa,
            &mut sb,
            cid,
            reattachment::packet(&fixture, false),
        )
        .await;
        if let Some((old_channel, old_cid)) = old {
            assert_ne!(channel, old_channel);
            assert_ne!(cid, old_cid);
        }
        old = Some((channel, cid));
    }
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    timeline.dump();
    drop(a);
    drop(b);
    stop.cancel();
    service.await.unwrap();
    for gateway in gateways {
        Arc::try_unwrap(gateway)
            .ok()
            .unwrap()
            .control
            .into_inner()
            .shutdown()
            .await
            .unwrap();
    }
    fixture.close().await;
}
