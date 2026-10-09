use super::{
    datagram,
    support::{
        diagnostic::{handshake, renew_observer, setup},
        request,
        secure::SecureMesh,
    },
};
use meshway_dataplane::secure::{
    CircuitAuthorization, CircuitState, Error, PendingFrame, Received,
};
use meshway_model::{Digest, EventId};
use std::time::{Duration, Instant};

fn measurement() -> EventId {
    EventId::new(request().into_uuid()).unwrap()
}
fn wire(frame: PendingFrame) -> Vec<u8> {
    frame.release(|bytes| bytes.into_vec()).unwrap()
}
fn forward(result: Result<Received, Error>) -> PendingFrame {
    let Received::Forward(frame) = result.unwrap() else {
        panic!("opaque relay")
    };
    frame
}

fn terminal_response(fixture: &mut SecureMesh, request: PendingFrame) -> PendingFrame {
    let request = forward(fixture.r.receive(&fixture.ar.carrier, wire(request)));
    let Received::ProbeRequest { response, .. } = fixture
        .b
        .receive(&fixture.rb.carrier, wire(request))
        .unwrap()
    else {
        panic!("terminal diagnostic")
    };
    response
}

async fn delayed_probe_roundtrip(
    fixture: &mut SecureMesh,
    pending: PendingFrame,
    id: EventId,
    challenge: Digest,
    forward_delay: Duration,
) -> Instant {
    let circuit = pending.circuit();
    fixture.ar.dial.send(pending).await;
    // Delay remote receipt, rather than the source's actual send/admission.
    tokio::time::sleep(forward_delay).await;
    let request = fixture.ar.listen.recv().await;
    let request = forward(fixture.r.receive(&fixture.ar.carrier, request));
    fixture.rb.dial.send(request).await;
    let request = fixture.rb.listen.recv().await;
    let arrived_at = Instant::now();
    let Received::ProbeRequest { response, .. } =
        fixture.b.receive(&fixture.rb.carrier, request).unwrap()
    else {
        panic!("terminal diagnostic")
    };
    fixture.rb.listen.send(response).await;
    let response = fixture.rb.dial.recv().await;
    let response = forward(fixture.r.receive(&fixture.rb.carrier, response));
    fixture.ar.listen.send(response).await;
    let response = fixture.ar.dial.recv().await;
    assert!(
        matches!(fixture.a.receive(&fixture.ar.carrier, response).unwrap(), Received::ProbeReply { circuit: returned, measurement_id, challenge: returned_challenge } if returned == circuit && measurement_id == id && returned_challenge == challenge)
    );
    arrived_at
}

#[tokio::test]
async fn diagnostic_source_cadence_survives_compressed_remote_arrivals() {
    // C: the diagnostic's 1000 ms source cadence must pass while the remote
    // read is held; virtual time cannot establish real TCP receive timestamps.
    let (mut fixture, task) = setup().await;
    let circuit = handshake(&mut fixture, &task).await;
    let first_id = measurement();
    let challenge = Digest::from_bytes([21; 32]);
    let first = fixture
        .a
        .probe(
            circuit,
            first_id,
            challenge,
            Instant::now() + Duration::from_secs(5),
            &fixture.ar.carrier,
        )
        .unwrap();
    let first_arrival = delayed_probe_roundtrip(
        &mut fixture,
        first,
        first_id,
        challenge,
        Duration::from_millis(1010),
    )
    .await;
    // Single inflight: the first authenticated reply has completed before the
    // next source request, which is now permitted by its original 1 Hz cadence.
    let second_id = measurement();
    let second = fixture
        .a
        .probe(
            circuit,
            second_id,
            challenge,
            Instant::now() + Duration::from_secs(5),
            &fixture.ar.carrier,
        )
        .unwrap();
    let second_arrival =
        delayed_probe_roundtrip(&mut fixture, second, second_id, challenge, Duration::ZERO).await;
    assert!(
        second_arrival.duration_since(first_arrival) < Duration::from_secs(1),
        "test must exercise compressed arrivals"
    );
    for node in [&fixture.a, &fixture.r, &fixture.b] {
        assert_eq!(node.retained_flows(), 0);
    }
    fixture.close().await;
}

#[tokio::test]
async fn diagnostic_remote_burst_budget_is_shared_across_cids_at_relay_and_terminal() {
    // C: emit at the real 1000 ms source cadence, then deliver a compressed
    // burst. Skipping the cadence would test source rejection instead.
    for terminal in [false, true] {
        let (mut fixture, task) = setup().await;
        let circuits = [
            handshake(&mut fixture, &task).await,
            handshake(&mut fixture, &task).await,
            handshake(&mut fixture, &task).await,
        ];
        let mut queued = Vec::new();
        for (index, circuit) in circuits.into_iter().enumerate() {
            if index > 0 {
                tokio::time::sleep(Duration::from_millis(1010)).await;
            }
            let request = fixture
                .a
                .probe(
                    circuit,
                    measurement(),
                    [22; 32].into(),
                    Instant::now() + Duration::from_secs(8),
                    &fixture.ar.carrier,
                )
                .unwrap();
            let request = if terminal {
                // R sees lawful cadence; only B sees the compressed burst.
                forward(fixture.r.receive(&fixture.ar.carrier, wire(request)))
            } else {
                request
            };
            queued.push(wire(request));
        }
        let last = queued.pop().unwrap();
        for request in queued {
            let result = if terminal {
                fixture.b.receive(&fixture.rb.carrier, request)
            } else {
                fixture.r.receive(&fixture.ar.carrier, request)
            };
            assert!(matches!(
                result.unwrap(),
                Received::ProbeRequest { .. } | Received::Forward(_)
            ));
        }
        // A new CID, valid proof and distinct measurement cannot buy a third
        // token. Repeated rejected admissions cannot replenish or reset debt.
        for _ in 0..4 {
            let result = if terminal {
                fixture.b.receive(&fixture.rb.carrier, last.clone())
            } else {
                fixture.r.receive(&fixture.ar.carrier, last.clone())
            };
            assert!(matches!(result, Err(Error::Capacity)));
        }
        fixture.close().await;
    }
}

#[tokio::test]
async fn diagnostic_real_control_full_path_noise_reply_and_business_isolation() {
    let (mut fixture, task) = setup().await;
    assert!(
        fixture
            .a
            .start_authorized(
                CircuitAuthorization::Diagnostic(task.id),
                vec![fixture.mesh.a.id, fixture.mesh.b.id],
                &fixture.ar.carrier
            )
            .is_err()
    );
    let circuit = handshake(&mut fixture, &task).await;
    let business = fixture.handshake().await;
    let id = measurement();
    let challenge = Digest::from_bytes([0x73; 32]);
    let deadline = Instant::now() + Duration::from_secs(3);
    assert!(
        fixture
            .a
            .probe(business, id, challenge, deadline, &fixture.ar.carrier)
            .is_err()
    );
    assert!(
        fixture
            .b
            .probe(circuit, id, challenge, deadline, &fixture.rb.carrier)
            .is_err()
    );
    assert!(
        fixture
            .a
            .send(circuit, datagram(&fixture.mesh, false, false))
            .is_err()
    );
    let pending = fixture
        .a
        .probe(circuit, id, challenge, deadline, &fixture.ar.carrier)
        .unwrap();
    // The actual transport accepts the request before the receipt exists.
    let before = Instant::now();
    let receipt = meshway_dataplane::carrier::send_scoped_with_receipt(
        &mut fixture.ar.dial.sender,
        pending,
        Some(fixture.ar.dial.peer),
        None,
    )
    .await
    .unwrap();
    assert!(receipt.admitted_at >= before && receipt.admitted_at <= Instant::now());
    let first = fixture.ar.listen.recv().await;
    assert!(!first.windows(32).any(|bytes| bytes == challenge.as_bytes()));
    let mut corrupt = first.clone();
    let relay_tag = corrupt.len() - 32;
    corrupt[relay_tag] ^= 1;
    assert!(fixture.r.receive(&fixture.ar.carrier, corrupt).is_err());
    assert!(
        fixture
            .b
            .receive(&fixture.ar.carrier, first.clone())
            .is_err()
    );
    let next = forward(fixture.r.receive(&fixture.ar.carrier, first.clone()));
    assert!(fixture.r.receive(&fixture.ar.carrier, first).is_err());
    fixture.rb.dial.send(next).await;
    let bytes = fixture.rb.listen.recv().await;
    let Received::ProbeRequest {
        circuit: terminal,
        measurement_id,
        response,
    } = fixture
        .b
        .receive(&fixture.rb.carrier, bytes.clone())
        .unwrap()
    else {
        panic!("probe request")
    };
    assert_eq!((terminal, measurement_id), (circuit, id));
    assert!(fixture.b.receive(&fixture.rb.carrier, bytes).is_err());
    fixture.rb.listen.send(response).await;
    let bytes = fixture.rb.dial.recv().await;
    let response = forward(fixture.r.receive(&fixture.rb.carrier, bytes.clone()));
    assert!(fixture.r.receive(&fixture.rb.carrier, bytes).is_err());
    fixture.ar.listen.send(response).await;
    let bytes = fixture.ar.dial.recv().await;
    assert!(
        matches!(fixture.a.receive(&fixture.ar.carrier, bytes.clone()).unwrap(), Received::ProbeReply { circuit: returned, measurement_id, challenge: value } if returned == circuit && measurement_id == id && value == challenge)
    );
    assert!(fixture.a.receive(&fixture.ar.carrier, bytes).is_err());
    for node in [&fixture.a, &fixture.r, &fixture.b] {
        assert_eq!(node.retained_flows(), 0);
    }
    let packet = fixture
        .a
        .send(business, datagram(&fixture.mesh, false, false))
        .unwrap();
    let next = forward(fixture.r.receive(&fixture.ar.carrier, wire(packet)));
    assert!(matches!(
        fixture.b.receive(&fixture.rb.carrier, wire(next)).unwrap(),
        Received::Packet(_)
    ));
    fixture.close().await;
}

#[tokio::test]
async fn diagnostic_observation_rate_cannot_be_bypassed_with_another_cid() {
    let (mut fixture, task) = setup().await;
    let one = handshake(&mut fixture, &task).await;
    let two = handshake(&mut fixture, &task).await;
    let deadline = Instant::now() + Duration::from_secs(3);
    let first = fixture
        .a
        .probe(
            one,
            measurement(),
            [1; 32].into(),
            deadline,
            &fixture.ar.carrier,
        )
        .unwrap();
    assert!(matches!(
        fixture.a.probe(
            two,
            measurement(),
            [2; 32].into(),
            deadline,
            &fixture.ar.carrier
        ),
        Err(Error::Capacity)
    ));
    fixture.a.close(one);
    assert!(first.release(|_| ()).is_err());
    assert!(matches!(
        fixture.a.probe(
            two,
            measurement(),
            [2; 32].into(),
            deadline,
            &fixture.ar.carrier
        ),
        Err(Error::Capacity)
    ));
    fixture.close().await;
}

#[tokio::test]
async fn diagnostic_continuation_retains_cipher_but_cancels_original_probe_and_reply() {
    // C: the renewed circuit retains the original 1000 ms diagnostic cadence;
    // a second probe before that window tests rate limiting, not continuation.
    let (mut fixture, mut task) = setup().await;
    renew_observer(&mut fixture, &mut task).await;
    let circuit = handshake(&mut fixture, &task).await;
    let request = fixture
        .a
        .probe(
            circuit,
            measurement(),
            [3; 32].into(),
            Instant::now() + Duration::from_secs(5),
            &fixture.ar.carrier,
        )
        .unwrap();
    let response = terminal_response(&mut fixture, request);
    let reply_wire = wire(response);
    let queued = forward(fixture.r.receive(&fixture.rb.carrier, reply_wire));
    let old = fixture.mesh.a.control.execution_snapshot().unwrap();
    fixture.renew().await;
    assert!(old.ensure_current().is_err());
    assert!(queued.release(|_| ()).is_err());
    for node in [&fixture.a, &fixture.r, &fixture.b] {
        assert_eq!(
            node.circuit_info(circuit).unwrap().unwrap().state,
            CircuitState::Established
        );
    }
    renew_observer(&mut fixture, &mut task).await;
    tokio::time::sleep(Duration::from_millis(1010)).await;
    let id = measurement();
    let request = fixture
        .a
        .probe(
            circuit,
            id,
            [4; 32].into(),
            Instant::now() + Duration::from_secs(3),
            &fixture.ar.carrier,
        )
        .unwrap();
    let response = terminal_response(&mut fixture, request);
    let response = forward(fixture.r.receive(&fixture.rb.carrier, wire(response)));
    assert!(
        matches!(fixture.a.receive(&fixture.ar.carrier, wire(response)).unwrap(), Received::ProbeReply { measurement_id, .. } if measurement_id == id)
    );
    fixture
        .mesh
        .server
        .cancel_observation(&fixture.mesh.owner, task.subscription_id.unwrap())
        .await;
    fixture.reconcile().await;
    for node in [&fixture.a, &fixture.r, &fixture.b] {
        assert!(
            node.circuit_info(circuit)
                .unwrap()
                .is_none_or(|info| info.state == CircuitState::Closed)
        );
    }
    fixture.close().await;
}

#[tokio::test]
async fn relay_update_ends_inflight_probe_but_delivers_business_reply_exactly_once() {
    let (mut fixture, task) = setup().await;
    let diagnostic = handshake(&mut fixture, &task).await;
    let business = fixture.handshake().await;
    let old = fixture.mesh.r.control.execution_snapshot().unwrap();
    let lease = fixture.mesh.a.control.renew_instance().await.unwrap();
    fixture
        .mesh
        .a
        .control
        .renew_demand(&meshway_protocol::RenewDemandRequest {
            instance: fixture.mesh.a.instance(),
            request_id: fixture.mesh.demand,
            expires_at_ms: lease.expires_at_ms,
        })
        .await
        .unwrap();
    // Reproduce the real staggered installation: A and B use the successor,
    // while R admits the request immediately before its retained handoff.
    for (node, secure) in [
        (&mut fixture.mesh.a, &mut fixture.a),
        (&mut fixture.mesh.b, &mut fixture.b),
    ] {
        let snapshot = node.sync_current().await;
        secure
            .reconcile(node.control.dataplane_identity(snapshot).unwrap())
            .unwrap();
    }
    let id = measurement();
    let challenge = Digest::from_bytes([31; 32]);
    let request = fixture
        .a
        .probe(
            diagnostic,
            id,
            challenge,
            Instant::now() + Duration::from_secs(5),
            &fixture.ar.carrier,
        )
        .unwrap();
    let request_wire = wire(request);
    let request = forward(fixture.r.receive(&fixture.ar.carrier, request_wire.clone()));
    let Received::ProbeRequest { response, .. } = fixture
        .b
        .receive(&fixture.rb.carrier, wire(request))
        .unwrap()
    else {
        panic!("terminal diagnostic")
    };
    let diagnostic_reply = wire(response);
    let packet = fixture
        .a
        .send(business, datagram(&fixture.mesh, false, false))
        .unwrap();
    let packet = forward(fixture.r.receive(&fixture.ar.carrier, wire(packet)));
    let Received::Packet(packet) = fixture
        .b
        .receive(&fixture.rb.carrier, wire(packet))
        .unwrap()
    else {
        panic!("terminal business")
    };
    packet.release(|_| ()).unwrap();
    let business_reply = wire(
        fixture
            .b
            .send(business, datagram(&fixture.mesh, false, true))
            .unwrap(),
    );
    let SecureMesh {
        mut mesh,
        a,
        mut r,
        b,
        ar,
        rb,
    } = fixture;
    let (gate, arrived, resume) = super::authority_race::boundary(
        "meshway_dataplane::secure::authority_boundary",
        "probe_maintenance",
    );
    let worker = tokio::task::spawn_blocking(move || {
        let _ = tracing::subscriber::with_default(gate, || r.maintain());
        r
    });
    tokio::time::timeout(Duration::from_secs(10), arrived)
        .await
        .unwrap()
        .unwrap();
    // Publication arrives after maintenance's entry authority check, but
    // before it examines the already-committed relay correlation.
    let snapshot = mesh.r.sync_current().await;
    assert_ne!(old.reference(), snapshot.reference());
    assert!(old.ensure_current().is_err());
    resume.send(()).unwrap();
    let mut r = worker.await.unwrap();
    r.reconcile(mesh.r.control.dataplane_identity(snapshot).unwrap())
        .unwrap();
    let mut fixture = SecureMesh {
        mesh,
        a,
        r,
        b,
        ar,
        rb,
    };
    // The relay correlation belonged to the replaced snapshot: it ends with
    // it and is never rebound. The scheduler sends a new measurement instead.
    fixture.r.maintain().unwrap();
    assert!(matches!(
        fixture
            .r
            .receive(&fixture.rb.carrier, diagnostic_reply.clone()),
        Err(Error::Replay)
    ));
    assert!(matches!(
        fixture.r.receive(&fixture.ar.carrier, request_wire),
        Err(Error::Replay)
    ));
    let packet = forward(
        fixture
            .r
            .receive(&fixture.rb.carrier, business_reply.clone()),
    );
    let packet = wire(packet);
    let Received::Packet(delivery) = fixture
        .a
        .receive(&fixture.ar.carrier, packet.clone())
        .unwrap()
    else {
        panic!("business reply")
    };
    delivery.release(|_| ()).unwrap();
    assert!(matches!(
        fixture.a.receive(&fixture.ar.carrier, packet),
        Err(Error::Replay)
    ));
    assert!(matches!(
        fixture.r.receive(&fixture.rb.carrier, business_reply),
        Err(Error::Replay)
    ));
    assert_eq!(fixture.r.retained_flows(), 0);
    fixture.close().await;
}

#[tokio::test]
async fn diagnostic_original_probe_expiry_rejects_queued_and_late_reply() {
    let (mut fixture, task) = setup().await;
    let circuit = handshake(&mut fixture, &task).await;
    let request = fixture
        .a
        .probe(
            circuit,
            measurement(),
            [7; 32].into(),
            Instant::now() + Duration::from_millis(300),
            &fixture.ar.carrier,
        )
        .unwrap();
    let response = terminal_response(&mut fixture, request);
    let response = forward(fixture.r.receive(&fixture.rb.carrier, wire(response)));
    let bytes = wire(response);
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(fixture.a.receive(&fixture.ar.carrier, bytes).is_err());
    fixture.close().await;
}

#[tokio::test]
async fn retained_relay_correlation_never_renews_probe_deadline_or_source_scope() {
    // C: the original 2 s probe deadline must expire through relay handoff;
    // the 10 s source-scope case instead completes before half its deadline.
    for refresh_source in [false, true] {
        let (mut fixture, task) = setup().await;
        let circuit = handshake(&mut fixture, &task).await;
        if refresh_source {
            // Publish a real successor before issuing the short-lived probe.
            // Demand feedback does not renew the old local snapshot; A keeps
            // using it until the later source-only installation below.
            let response = fixture
                .mesh
                .a
                .control
                .create_demand(&meshway_protocol::DemandRequest {
                    instance: fixture.mesh.a.instance(),
                    request_id: request(),
                    target: meshway_configuration::execution::BusinessTarget::Mesh {
                        node: fixture.mesh.b.id,
                    },
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
        // Scope invalidation is checked before expiry, independently of the
        // real HTTP handoff's scheduling under the full workspace test load.
        let lifetime = if refresh_source { 10 } else { 2 };
        let deadline = Instant::now() + Duration::from_secs(lifetime);
        let request = fixture
            .a
            .probe(
                circuit,
                measurement(),
                [32; 32].into(),
                deadline,
                &fixture.ar.carrier,
            )
            .unwrap();
        let response = terminal_response(&mut fixture, request);
        let reply = wire(response);
        let old = fixture.mesh.a.control.execution_snapshot().unwrap();
        if refresh_source {
            // Source-scope fencing needs only the source's real lease handoff.
            // Three Nodes' HTTP installations need not race this probe. The
            // other branch separately covers retained relay correlation expiry.
            let snapshot = fixture.mesh.a.sync_current().await;
            fixture
                .a
                .reconcile(fixture.mesh.a.control.dataplane_identity(snapshot).unwrap())
                .unwrap();
            assert!(old.ensure_current().is_err());
            assert!(
                deadline.saturating_duration_since(Instant::now())
                    >= Duration::from_secs(lifetime) / 2,
                "source-only handoff must fit half the original probe deadline"
            );
            let reply = forward(fixture.r.receive(&fixture.rb.carrier, reply));
            assert!(fixture.a.receive(&fixture.ar.carrier, wire(reply)).is_err());
        } else {
            fixture.renew().await;
            tokio::time::sleep_until((deadline + Duration::from_millis(10)).into()).await;
            assert!(fixture.r.receive(&fixture.rb.carrier, reply).is_err());
        }
        fixture.close().await;
    }
}

#[tokio::test]
async fn diagnostic_cached_handshake_retry_uses_same_cipher_and_original_guard() {
    let (mut fixture, task) = setup().await;
    let init = fixture
        .a
        .start_authorized(
            CircuitAuthorization::Diagnostic(task.id),
            fixture.mesh.path.clone(),
            &fixture.ar.carrier,
        )
        .unwrap();
    let circuit = init.circuit();
    let original = wire(init);
    let relay = forward(fixture.r.receive(&fixture.ar.carrier, original.clone()));
    let Received::Established {
        response: Some(lost),
        ..
    } = fixture.b.receive(&fixture.rb.carrier, wire(relay)).unwrap()
    else {
        panic!("terminal handshake")
    };
    let lost = wire(lost);
    assert!(fixture.a.retry_handshake(circuit).unwrap().is_none());
    tokio::time::sleep(Duration::from_millis(260)).await;
    let retry = fixture.a.retry_handshake(circuit).unwrap().unwrap();
    let retry_bytes = wire(retry);
    assert_eq!(retry_bytes, original);
    let retry = forward(fixture.r.receive(&fixture.ar.carrier, retry_bytes));
    let Received::Established {
        response: Some(recovered),
        ..
    } = fixture.b.receive(&fixture.rb.carrier, wire(retry)).unwrap()
    else {
        panic!("cached terminal response")
    };
    let recovered = wire(recovered);
    assert_eq!(recovered, lost);
    let recovered = forward(fixture.r.receive(&fixture.rb.carrier, recovered));
    assert!(
        matches!(fixture.a.receive(&fixture.ar.carrier, wire(recovered)).unwrap(), Received::Established { circuit: id, response: None } if id == circuit)
    );
    let id = measurement();
    let request = fixture
        .a
        .probe(
            circuit,
            id,
            [9; 32].into(),
            Instant::now() + Duration::from_secs(3),
            &fixture.ar.carrier,
        )
        .unwrap();
    let reply = terminal_response(&mut fixture, request);
    let reply = forward(fixture.r.receive(&fixture.rb.carrier, wire(reply)));
    assert!(
        matches!(fixture.a.receive(&fixture.ar.carrier, wire(reply)).unwrap(), Received::ProbeReply { measurement_id, .. } if measurement_id == id)
    );
    fixture.close().await;
}

#[tokio::test]
async fn diagnostic_source_instance_replacement_cannot_reuse_old_authority_or_cipher() {
    let (mut fixture, task) = setup().await;
    let circuit = handshake(&mut fixture, &task).await;
    let old = fixture.mesh.a.control.execution_snapshot().unwrap();
    let capabilities = old.configuration().unwrap().self_keys.capabilities.clone();
    let queued = fixture
        .a
        .probe(
            circuit,
            measurement(),
            [10; 32].into(),
            Instant::now() + Duration::from_secs(3),
            &fixture.ar.carrier,
        )
        .unwrap();
    fixture.mesh.a.control.stop().await.unwrap();
    assert!(queued.release(|_| ()).is_err());
    fixture.mesh.a.control.authenticate().await.unwrap();
    fixture
        .mesh
        .a
        .control
        .acquire_instance(capabilities)
        .await
        .unwrap();
    let snapshot = fixture.mesh.a.sync_current().await;
    assert_ne!(snapshot.instance().instance_id, task.instance_id);
    assert!(
        fixture
            .a
            .reconcile(
                fixture
                    .mesh
                    .a
                    .control
                    .dataplane_identity(snapshot.clone())
                    .unwrap()
            )
            .is_err()
    );
    let mut fresh = meshway_dataplane::secure::SecureNode::new(
        fixture.mesh.a.control.dataplane_identity(snapshot).unwrap(),
        Default::default(),
        std::collections::BTreeMap::from([(fixture.mesh.r.id, fixture.ar.carrier.clone())]),
    )
    .unwrap();
    assert!(
        fresh
            .start_authorized(
                CircuitAuthorization::Diagnostic(task.id),
                fixture.mesh.path.clone(),
                &fixture.ar.carrier
            )
            .is_err()
    );
    assert!(
        fresh
            .probe(
                circuit,
                measurement(),
                [10; 32].into(),
                Instant::now() + Duration::from_secs(3),
                &fixture.ar.carrier
            )
            .is_err()
    );
    drop(fresh);
    fixture.close().await;
}
