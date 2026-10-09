use super::*;

pub(super) async fn exercise_traversal(healthy_only: bool) {
    let StunService {
        address: stun_address,
        stop,
        service,
        observed,
    } = StunService::start(true).await;
    let mut fixture = fixture::Fixture::builder().stun(stun_address).build().await;
    let ids = fixture.ids();
    if healthy_only {
        // The normal fixture admits alternate paths through offline C, even
        // for A's demand. Restrict this case to the exact A-B edge so "healthy"
        // means every admitted adjacency, rather than just one selected link.
        fixture
            .configure(|cfg| {
                cfg.defaults.topology = meshway_configuration::TopologyDefault::Deny;
                cfg.topology = vec![meshway_configuration::TopologyRule {
                    remark: String::new(),
                    action: meshway_configuration::TopologyAction::Allow,
                    a: format!("node:{}", ids[0]),
                    b: format!("node:{}", ids[1]),
                    transports: meshway_configuration::Input::Omitted,
 keepalive: false,
                }];
            })
            .await;
        for index in 0..2 {
            fixture.refresh(index).await;
        }
    }
    let traffic = Arc::new(Traffic::default());
    let factory = NatFactory::new(&mut fixture, stun_address, traffic.clone());
    let mut gateways = Vec::new();
    for control in fixture.controls.drain(..2) {
        gateways.push(Gateway::applied(control, None).await);
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
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if a.snapshot()
                .unwrap()
                .sockets
                .iter()
                .filter(|socket| socket.bound_address.is_ipv4())
                .all(|socket| !socket.endpoints.is_empty())
                && b.snapshot()
                    .unwrap()
                    .sockets
                    .iter()
                    .filter(|socket| socket.bound_address.is_ipv4())
                    .all(|socket| !socket.endpoints.is_empty())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let opened = traffic.opened.load(Ordering::SeqCst);
    for manager in [&a, &b] {
        assert!(manager.snapshot().unwrap().sockets.iter().any(|socket| {
            socket.bound_address.is_ipv6()
                && matches!(socket.reference.resource, ResourceRef::Dial { .. })
        }));
    }
    let initial = listener_socket(&a);
    assert_eq!(
        initial.endpoints[0].origin,
        EndpointSource::Stun {
            observer: stun_address.to_string()
        }
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(initial, listener_socket(&a));
    assert!(a.snapshot().unwrap().links.is_empty());
    assert!(b.snapshot().unwrap().links.is_empty());
    traffic.block_traversal.store(true, Ordering::SeqCst);
    // Only actual report acknowledgement releases prepare on either Node.
    gateways[0].report(a.snapshot().unwrap()).await;
    gateways[1].report(b.snapshot().unwrap()).await;
    {
        use meshway_control::connectivity::{Outcome, prepare_request};
        let mut control = gateways[0].control.lock().await;
        let snapshot = control.execution_snapshot().unwrap();
        let Outcome::Current(discovery) = control.query_udp_traversal(&snapshot).await.unwrap()
        else {
            panic!("metadata");
        };
        let local = listener_socket(&a);
        let remote = listener_socket(&b);
        let carrier = fixture::carriers(&fixture.identities[0])[&ids[1]].clone();
        let mut request = prepare_request(
            &discovery.context,
            carrier,
            local.candidate(&local.endpoints[0]).unwrap(),
            remote.candidate(&remote.endpoints[0]).unwrap(),
        )
        .unwrap();
        request.peer_candidate.observation_digest = [42; 32].into();
        assert!(matches!(
            control
                .prepare_udp_traversal(&discovery.context, &request)
                .await
                .unwrap(),
            Outcome::Unavailable(_)
        ));
        assert!(snapshot.ensure_current().is_ok());
        assert!(control.status().authenticated);
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while traffic.blocked.lock().unwrap().is_none() {
            gateways[0].report_changes(a.snapshot().unwrap()).await;
            gateways[1].report_changes(b.snapshot().unwrap()).await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let blocked = traffic.blocked.lock().unwrap().clone().unwrap();
    let cancelled_digest = {
        let index = if a.snapshot().unwrap().listeners[0].local == blocked.local {
            0
        } else {
            assert_eq!(b.snapshot().unwrap().listeners[0].local, blocked.local);
            1
        };
        let active = traffic
            .authorized
            .lock()
            .unwrap()
            .get(&(blocked.local, blocked.digest))
            .expect("blocked frame must have an exact locally authorized scope")
            .clone();
        assert_eq!(
            active.validated_offer().peer().address,
            blocked.remote.to_string()
        );
        active
            .ensure_current()
            .expect("cancel a live original attempt, not an already expired observation");
        let mut control = gateways[index].control.lock().await;
        let digest = active.validated_offer().offer().digest;
        control
            .cancel_udp_traversal(active.validated_offer())
            .await
            .unwrap();
        assert!(active.ensure_current().is_err());
        assert!(a.snapshot().unwrap().links.is_empty());
        assert!(b.snapshot().unwrap().links.is_empty());
        digest
    };
    traffic.block_traversal.store(false, Ordering::SeqCst);
    blocked.waker.wake_by_ref();
    traffic.waker.wake();
    tokio::time::timeout(Duration::from_secs(12), async {
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
    .await
    .expect("actual candidate acknowledgements must keep running during acquisition");
    let a_link = a.snapshot().unwrap().links[0].clone();
    let b_link = b.snapshot().unwrap().links[0].clone();
    let observed_before: Vec<_> = [&a, &b]
        .into_iter()
        .map(|manager| listener_socket(manager).endpoints)
        .collect();
    if healthy_only {
        for manager in [&a, &b] {
            assert!(
                manager.snapshot().unwrap().sockets.iter().any(|socket| {
                    socket.bound_address.is_ipv6() && socket.endpoints.is_empty()
                })
            );
        }
        // Drain discovery already issued before bilateral channel selection;
        // the following window measures steady observation refresh only.
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let queries: Vec<_> = gateways
        .iter()
        .map(|gateway| gateway.queries.load(Ordering::SeqCst))
        .collect();
    tokio::time::sleep(if healthy_only {
        Duration::from_secs(6)
    } else {
        Duration::from_millis(2250)
    })
    .await;
    for (gateway, before) in gateways.iter().zip(queries) {
        let count = gateway.queries.load(Ordering::SeqCst) - before;
        if healthy_only {
            assert!(
                count <= 1,
                "an unobservable IPv6 socket must not keep polling IPv4-only STUN metadata: {count}"
            );
        } else {
            assert!(count <= 10, "offline alternate peer keeps paced discovery");
        }
    }
    if healthy_only {
        let before: Vec<_> = gateways
            .iter()
            .map(|gateway| gateway.queries.load(Ordering::SeqCst))
            .collect();
        for round in 0..3 {
            fixture
                .configure(|cfg| {
                    cfg.nodes.get_mut(&ids[0]).unwrap().remark = format!("handoff-{round}");
                })
                .await;
            for (gateway, manager) in gateways.iter().zip([&mut a, &mut b]) {
                let previous = manager.shared.identity.snapshot().reference();
                let identity = {
                    let mut control = gateway.control.lock().await;
                    control.refresh_configuration().await.unwrap();
                    control
                        .dataplane_identity(control.execution_snapshot().unwrap())
                        .unwrap()
                };
                assert_ne!(identity.snapshot().reference(), previous);
                manager.reconcile(identity).await.unwrap();
            }
            tokio::time::sleep(Duration::from_millis(2100)).await;
            assert_eq!(a.snapshot().unwrap().links[0].id, a_link.id);
            assert_eq!(b.snapshot().unwrap().links[0].id, b_link.id);
        }
        for (gateway, count) in gateways.iter().zip(before) {
            assert!(
                gateway.queries.load(Ordering::SeqCst) - count <= 4,
                "execution handoffs do not reset the NAT refresh budget"
            );
        }
        // Span the original observations' 20-second maximum lifetime. Healthy
        // retained channels must still have genuinely new socket observations,
        // without obtaining a replacement adjacency or retimestamping old State.
        tokio::time::sleep(Duration::from_secs(20)).await;
        for ((manager, original), link) in [&a, &b]
            .into_iter()
            .zip(observed_before)
            .zip([&a_link, &b_link])
        {
            let fact = listener_socket(manager);
            assert!(
                !fact.endpoints.is_empty(),
                "healthy NAT socket remains observable"
            );
            assert!(fact.endpoints.iter().all(|endpoint| {
                endpoint.expires_at > chrono::Utc::now()
                    && original
                        .iter()
                        .all(|old| endpoint.observed_at > old.observed_at)
            }));
            assert_eq!(manager.snapshot().unwrap().links[0].id, link.id);
        }
        stop.cancel();
        service.await.unwrap();
        a.shutdown().await.unwrap();
        b.shutdown().await.unwrap();
        drop(a);
        drop(b);
        for gateway in gateways {
            Arc::try_unwrap(gateway)
                .ok()
                .expect("joined gateway")
                .control
                .into_inner()
                .shutdown()
                .await
                .unwrap();
        }
        fixture.close().await;
        return;
    }
    assert_eq!(a_link.id, b_link.id);
    for link in [&a_link, &b_link] {
        let channels = traffic.channels.lock().unwrap();
        let active = &channels
            .iter()
            .find(|channel| channel.local == link.local && channel.lifetime.is_current())
            .unwrap()
            .attempt;
        let offer = active.validated_offer();
        assert_eq!(offer.peer().address, link.remote.to_string());
        assert_eq!(
            offer.peer().source,
            meshway_model::observations::EndpointKind::Stun
        );
        assert_eq!(
            link.acquisition,
            LinkAcquisition::CoordinatedUdp {
                attempt_id: offer.offer().attempt_id,
                endpoint_source: offer.peer().source,
            }
        );
    }
    assert_eq!(
        a_link.socket_id,
        a.snapshot().unwrap().listeners[0].socket_id
    );
    assert_eq!(
        b_link.socket_id,
        b.snapshot().unwrap().listeners[0].socket_id
    );
    assert!(observed.lock().unwrap().contains(&a_link.local));
    assert!(observed.lock().unwrap().contains(&b_link.local));
    {
        let sent = traffic.sent.lock().unwrap();
        assert!(!sent.iter().any(|(local, _, bytes)| *local == blocked.local
            && bytes.starts_with(b"MWTRV1\0\0")
            && bytes.get(8..40) == Some(cancelled_digest.as_bytes().as_slice())));
        for link in [&a_link, &b_link] {
            assert!(
                sent.iter()
                    .any(|(local, remote, bytes)| *local == link.local
                        && *remote == stun_address
                        && meshway_stun::classify(bytes).is_some())
            );
            assert!(
                sent.iter()
                    .any(|(local, remote, bytes)| *local == link.local
                        && *remote == link.remote
                        && bytes.starts_with(b"MWTRV1\0\0"))
            );
        }
    }
    {
        let active = traffic
            .channels
            .lock()
            .unwrap()
            .iter()
            .find(|channel| channel.local == a_link.local && channel.lifetime.is_current())
            .expect("scope of the actual selected local channel")
            .attempt
            .clone();
        let offered = active.validated_offer().clone();
        let mut control = gateways[0].control.lock().await;
        control.cancel_udp_traversal(&offered).await.unwrap();
        assert!(active.ensure_current().is_err());
        // Server cancellation closes acquisition, not either authenticated link.
        assert_eq!(a.snapshot().unwrap().links[0].id, a_link.id);
        assert_eq!(b.snapshot().unwrap().links[0].id, b_link.id);
    }
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
    let receipt = a.send_with_receipt(pending).await.unwrap();
    assert_eq!(receipt.link, a_link);
    let (link, received) = frame(&mut b).await;
    let meshway_dataplane::secure::Received::Established {
        response: Some(response),
        ..
    } = secure_b.receive(&link.carrier, received.payload).unwrap()
    else {
        panic!("Noise responder");
    };
    b.send(response).await.unwrap();
    let (link, received) = frame(&mut a).await;
    assert!(matches!(
        secure_a.receive(&link.carrier, received.payload).unwrap(),
        meshway_dataplane::secure::Received::Established { .. }
    ));
    stop.cancel();
    service.await.unwrap();
    // Establish an actual reply flow before independently publishing five DIDs.
    // There is no STUN service for any subsequent reacquisition to fall back on.
    reattachment::exchange(
        &a,
        &mut b,
        &mut secure_a,
        &mut secure_b,
        circuit,
        reattachment::packet(&fixture, false),
    )
    .await;
    let socket_before = listener_socket(&a);
    let old_scope = a.shared.links.lock().unwrap()[&ids[1]].scope.clone();
    let sender = a.shared.links.lock().unwrap()[&ids[1]].sender.clone();
    let pending = secure_a
        .send(circuit, reattachment::packet(&fixture, false))
        .unwrap();
    traffic.block_data.store(true, Ordering::SeqCst);
    let mut unpolled = Box::pin(meshway_dataplane::carrier::send_shared(
        &sender,
        pending,
        a_link.remote,
        Arc::new(old_scope.clone()),
    ));
    assert!(futures_util::poll!(unpolled.as_mut()).is_pending());
    tokio::time::timeout(Duration::from_secs(1), async {
        while !traffic.data_blocked.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let writes = traffic.data_writes.load(Ordering::SeqCst);
    let mut demands = Vec::new();
    for index in 0..5 {
        let request_id = fixture::request();
        {
            let mut control = gateways[0].control.lock().await;
            let instance = control.execution_snapshot().unwrap().instance();
            let response = control
                .create_demand(&meshway_protocol::DemandRequest {
                    instance,
                    request_id,
                    target: meshway_configuration::execution::BusinessTarget::Mesh { node: ids[1] },
                    protocol: meshway_configuration::execution::BusinessProtocol::Udp,
                    port: Some(42424.try_into().unwrap()),
                    profile: "mesh".into(),
                    duration_seconds: 120,
                })
                .await
                .unwrap();
            assert!(matches!(
                response.decision,
                meshway_configuration::execution::DemandDecision::Permitted { .. }
            ));
        }
        demands.push(request_id);
        fixture.identities[0] = gateways[0].synchronize(&mut a, &mut secure_a).await;
        tokio::time::sleep(Duration::from_millis(25)).await;
        fixture.identities[1] = gateways[1].synchronize(&mut b, &mut secure_b).await;
        if index == 0 {
            assert!(old_scope.snapshot.ensure_current().is_err());
            assert!(old_scope.snapshot.retention_deadline().is_ok());
            traffic.block_data.store(false, Ordering::SeqCst);
            traffic.waker.wake();
            tokio::time::sleep(Duration::from_millis(25)).await;
            assert_eq!(
                traffic.data_writes.load(Ordering::SeqCst),
                writes,
                "an unpolled old output must not revive under a new physical gate"
            );
        }
        assert_eq!(a.snapshot().unwrap().links[0], a_link);
        assert_eq!(b.snapshot().unwrap().links[0], b_link);
        assert_eq!(traffic.opened.load(Ordering::SeqCst), opened);
        assert_eq!(traffic.dropped.load(Ordering::SeqCst), 0);
        let socket = listener_socket(&a);
        assert_eq!(
            socket.reference.socket_id,
            socket_before.reference.socket_id
        );
        assert_eq!(
            socket.reference.socket_generation,
            socket_before.reference.socket_generation
        );
        assert_ne!(
            socket.reference.configuration,
            socket_before.reference.configuration
        );
        assert!(
            socket
                .endpoints
                .iter()
                .all(|observation| socket_before.endpoints.contains(observation)),
            "handoff cannot create or retimestamp an observation"
        );
        for observation in &socket_before.endpoints {
            if observation.expires_at.timestamp_millis()
                > i64::try_from(fixture.identities[0].snapshot().now_ms().unwrap()).unwrap()
            {
                assert!(
                    socket.endpoints.contains(observation),
                    "live observations survive handoff unchanged"
                );
            }
        }
        // A only receives business traffic; only signed Pongs renew link health.
        reattachment::exchange(
            &b,
            &mut a,
            &mut secure_b,
            &mut secure_a,
            circuit,
            reattachment::packet(&fixture, true),
        )
        .await;
    }
    drop(unpolled);
    for request_id in &demands {
        {
            let mut control = gateways[0].control.lock().await;
            let lease = control.renew_instance().await.unwrap();
            control
                .renew_demand(&meshway_protocol::RenewDemandRequest {
                    instance: meshway_protocol::InstanceRequest {
                        instance_id: lease.instance_id,
                        generation: lease.generation,
                    },
                    request_id: *request_id,
                    expires_at_ms: lease.expires_at_ms,
                })
                .await
                .unwrap();
        }
        fixture.identities[0] = gateways[0].synchronize(&mut a, &mut secure_a).await;
        fixture.identities[1] = gateways[1].synchronize(&mut b, &mut secure_b).await;
        assert_eq!(a.snapshot().unwrap().links[0], a_link);
        assert_eq!(b.snapshot().unwrap().links[0], b_link);
        reattachment::exchange(
            &b,
            &mut a,
            &mut secure_b,
            &mut secure_a,
            circuit,
            reattachment::packet(&fixture, true),
        )
        .await;
    }
    // C cancels its independent C->B demand. A/B receive a direct publication
    // successor while their own established A->B business authority survives.
    // Local Control::cancel_demand intentionally hard-fences its own lineage.
    // C has no runtime pump: explicitly renew its real instance before its
    // final operation and the later candidate-expiry wait. Otherwise a slow
    // full-workspace run can expire this unused helper before strict shutdown.
    let instance = fixture.controls[0].renew_instance().await.unwrap();
    fixture.controls[0]
        .cancel_demand(&meshway_protocol::CancelDemandRequest {
            instance: meshway_protocol::InstanceRequest {
                instance_id: instance.instance_id,
                generation: instance.generation,
            },
            request_id: fixture.demands[1],
        })
        .await
        .unwrap();
    fixture.identities[0] = gateways[0].synchronize(&mut a, &mut secure_a).await;
    fixture.identities[1] = gateways[1].synchronize(&mut b, &mut secure_b).await;
    assert_eq!(a.snapshot().unwrap().links[0], a_link);
    assert_eq!(b.snapshot().unwrap().links[0], b_link);
    reattachment::exchange(
        &b,
        &mut a,
        &mut secure_b,
        &mut secure_a,
        circuit,
        reattachment::packet(&fixture, true),
    )
    .await;
    // The ten-second grant and twenty-second candidate expiration do not own
    // the selected channel or the established end-to-end circuit.
    tokio::time::sleep(Duration::from_secs(21)).await;
    assert_eq!(a.snapshot().unwrap().links[0], a_link);
    assert_eq!(b.snapshot().unwrap().links[0], b_link);
    assert!(
        secure_a
            .ready_circuits()
            .unwrap()
            .iter()
            .any(|ready| ready.id == circuit)
    );
    // Reauthenticate through the ordinary inbound path on the SAME physical
    // tuple. The old coordinated channel's STUN label must not be inherited.
    // Only raw I/O is injected; the listener processes the real signed exchange.
    let (dialer, listener, identity) = if a_link.carrier.dialer == ids[0] {
        (&a, &b, &fixture.identities[0])
    } else {
        (&b, &a, &fixture.identities[1])
    };
    let dial_fact = dialer.snapshot().unwrap().links[0].clone();
    let sender = {
        let links = dialer.shared.links.lock().unwrap();
        let link = &links[&dial_fact.peer];
        link.sender.clone()
    };
    let plan = Plan::new(identity.snapshot()).unwrap();
    let (initiation, intro) =
        adjacency::Initiation::new(identity, &plan, &dial_fact.carrier).unwrap();
    inject_adjacency(
        &sender,
        dial_fact.remote,
        adjacency::encode(adjacency::Kind::Init, &intro).unwrap(),
    )
    .await;
    let transcript = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let challenge =
                traffic
                    .sent
                    .lock()
                    .unwrap()
                    .iter()
                    .find_map(|(local, remote, bytes)| {
                        if *local != private(dial_fact.remote) || *remote != public(dial_fact.local)
                        {
                            return None;
                        }
                        let (kind, body) = adjacency::decode(bytes).ok()?;
                        if kind != adjacency::Kind::Challenge {
                            return None;
                        }
                        initiation.challenge(&plan, dial_fact.peer, body).ok()
                    });
            if let Some(transcript) = challenge {
                break transcript;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    inject_adjacency(
        &sender,
        dial_fact.remote,
        adjacency::encode(
            adjacency::Kind::Ack,
            &transcript.message(identity, adjacency::Kind::Ack).unwrap(),
        )
        .unwrap(),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let current = listener.snapshot().unwrap().links[0].clone();
            if current.id == transcript.id() {
                assert_ne!(current.id, dial_fact.id);
                assert_eq!(current.acquisition, LinkAcquisition::AcceptedInbound);
                assert_eq!(current.remote, public(dial_fact.local));
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let original = listener_socket(&a).reference;
    fixture.change_udp_buffer(ids[0]).await;
    let incompatible = {
        let mut control = gateways[0].control.lock().await;
        control.refresh_configuration().await.unwrap();
        control
            .dataplane_identity(control.execution_snapshot().unwrap())
            .unwrap()
    };
    assert!(
        a.can_reconcile(&incompatible),
        "direct continuation replaces the changed resource inside the same manager"
    );
    a.reconcile(incompatible.clone()).await.unwrap();
    assert_ne!(listener_socket(&a).reference.socket_id, original.socket_id);
    assert_ne!(
        listener_socket(&a).reference.socket_generation,
        original.socket_generation
    );
    fixture.identities[0] = incompatible;
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    let mut replacement = fixture
        .manager(0, factory.clone())
        .traversal(gateways[0].clone(), generation.checked_next().unwrap())
        .start()
        .await;
    let new = listener_socket(&replacement).reference;
    assert_ne!(original.socket_id, new.socket_id);
    assert_ne!(original.socket_generation, new.socket_generation);
    assert!(listener_socket(&replacement).endpoints.is_empty());
    replacement.shutdown().await.unwrap();
    let last = fixture.identities[0].snapshot().clone();
    gateways[0].control.lock().await.stop().await.unwrap();
    assert!(
        last.retention_deadline().is_err(),
        "a hard Control fence ends inactive FD retention too"
    );
    {
        let mut control = gateways[0].control.lock().await;
        control.authenticate().await.unwrap();
        control
            .acquire_instance(crate::capabilities())
            .await
            .unwrap();
        control.refresh_configuration().await.unwrap();
        let next = control.execution_snapshot().unwrap();
        assert_ne!(last.instance(), next.instance());
        assert!(next.continuation_from(&last).is_err());
    }
    drop(a);
    drop(b);
    drop(replacement);
    for gateway in gateways {
        Arc::try_unwrap(gateway)
            .ok()
            .expect("joined gateway")
            .control
            .into_inner()
            .shutdown()
            .await
            .unwrap();
    }
    fixture.close().await;
}
