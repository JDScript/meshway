use super::*;

pub(super) async fn native_tcp_session_survives_staggered_control_publications_without_reopening() {
    let mut fixture = fixture::Fixture::new(true, false).await;
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
    let mut fixture = fixture::Fixture::new(false, false).await;
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

pub(super) async fn staggered_three_node_managers_and_real_demand_publication_keep_original_circuit()
 {
    let mut fixture = fixture::Fixture::reverse().await;
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
