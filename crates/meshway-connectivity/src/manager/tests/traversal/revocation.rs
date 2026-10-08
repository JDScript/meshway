use super::*;

pub(super) async fn revocation_recovery(indirect_first: Option<bool>) {
    let timeline = AcquisitionTimeline::new();
    let StunService {
        address: stun_address,
        stop,
        service,
        ..
    } = StunService::start(false).await;
    let mut fixture = fixture::Fixture::builder()
        .stun(stun_address)
        .indirect_first(indirect_first)
        .build()
        .await;
    let ids = fixture.ids();
    let factory = NatFactory::new(
        &mut fixture,
        stun_address,
        Arc::new(Traffic {
            timeline: Some(timeline.clone()),
            ..Default::default()
        }),
    );
    let mut gateways = Vec::new();
    for control in fixture.controls.drain(..2) {
        gateways.push(Gateway::applied(control, Some(timeline.clone())).await);
    }
    let generation = Sequence::MIN.checked_next().unwrap();
    let mut a = fixture
        .manager(0, factory.clone())
        .traversal(gateways[0].clone(), generation)
        .start()
        .await;
    let mut b = fixture
        .manager(1, factory.clone())
        .traversal(gateways[1].clone(), generation)
        .start()
        .await;
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
