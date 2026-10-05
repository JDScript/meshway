//! Scripted handshake loss through the real Control/Noise/proof owners. These
//! tests discard exact frames, not random packets, and set no latency threshold.
use super::{
    datagram,
    support::{request, secure::SecureMesh},
};
use meshway_configuration::execution::{BusinessProtocol, BusinessTarget, DemandDecision};
use meshway_dataplane::secure::{CircuitState, Error, Limits, PendingFrame, Received, SecureNode};
use meshway_protocol::{CancelDemandRequest, DemandRequest};
use std::{collections::BTreeMap, time::Duration};

fn bytes(frame: PendingFrame) -> Vec<u8> {
    frame.release(|wire| wire).unwrap()
}

fn forwarded(result: Result<Received, Error>) -> Vec<u8> {
    let Received::Forward(frame) = result.unwrap() else {
        panic!("authenticated opaque forwarding");
    };
    bytes(frame)
}

fn response(result: Result<Received, Error>, cid: [u8; 32]) -> Vec<u8> {
    let Received::Established {
        circuit,
        response: Some(frame),
    } = result.unwrap()
    else {
        panic!("terminal returns the same response stage");
    };
    assert_eq!(circuit, cid);
    bytes(frame)
}

async fn retry(source: &mut SecureNode, cid: [u8; 32]) -> Vec<u8> {
    // Wait beyond the stage interval since the most recently delivered output,
    // so downstream stages are due too. This is pacing, not a performance gate.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let due = source.handshake_retry_at(cid).unwrap().unwrap();
    tokio::time::sleep_until(due.into()).await;
    let frame = source.retry_handshake(cid).unwrap().unwrap();
    assert_eq!(frame.circuit(), cid);
    bytes(frame)
}

fn finish(fixture: &mut SecureMesh, cid: [u8; 32], init: Vec<u8>) -> (Vec<u8>, Vec<u8>) {
    let init = forwarded(fixture.r.receive(&fixture.ar.carrier, init));
    let response = response(fixture.b.receive(&fixture.rb.carrier, init), cid);
    let upstream = forwarded(fixture.r.receive(&fixture.rb.carrier, response.clone()));
    assert!(
        matches!(fixture.a.receive(&fixture.ar.carrier, upstream.clone()).unwrap(),
        Received::Established { circuit, response: None } if circuit == cid)
    );
    (response, upstream)
}

fn exchange(fixture: &mut SecureMesh, cid: [u8; 32], source_port: u16) {
    let forward = datagram(&fixture.mesh, false, false);
    let facts = forward.facts();
    let forward = super::support::packet::udp(
        facts.traffic.source,
        facts.traffic.destination,
        source_port,
        42424,
        b"recovered",
    );
    let first = bytes(fixture.a.send(cid, forward).unwrap());
    let wire = forwarded(fixture.r.receive(&fixture.ar.carrier, first.clone()));
    let Received::Packet(packet) = fixture.b.receive(&fixture.rb.carrier, wire).unwrap() else {
        panic!("terminal plaintext");
    };
    packet.release(|_| ()).unwrap();
    assert!(fixture.r.receive(&fixture.ar.carrier, first).is_err());
    let reply = super::support::packet::udp(
        facts.traffic.destination,
        facts.traffic.source,
        42424,
        source_port,
        b"reply",
    );
    let wire = bytes(fixture.b.send(cid, reply).unwrap());
    let wire = forwarded(fixture.r.receive(&fixture.rb.carrier, wire));
    let Received::Packet(packet) = fixture.a.receive(&fixture.ar.carrier, wire).unwrap() else {
        panic!("original-path reply");
    };
    packet.release(|_| ()).unwrap();
}

#[tokio::test]
async fn every_init_and_response_hop_recovers_using_one_cid_and_exact_cached_outputs() {
    let mut fixture = SecureMesh::start().await;
    for lost_hop in 0..4 {
        let frame = fixture
            .a
            .start(fixture.mesh.demand, fixture.mesh.path.clone())
            .unwrap();
        let cid = frame.circuit();
        let deadline = fixture.a.circuit_info(cid).unwrap().unwrap().expires_at;
        let original = bytes(frame);
        let mut original_response = None;
        let mut original_upstream = None;
        if lost_hop > 0 {
            let init = forwarded(fixture.r.receive(&fixture.ar.carrier, original.clone()));
            if lost_hop > 1 {
                let wire = response(fixture.b.receive(&fixture.rb.carrier, init), cid);
                original_response = Some(wire.clone());
                if lost_hop > 2 {
                    original_upstream =
                        Some(forwarded(fixture.r.receive(&fixture.rb.carrier, wire)));
                }
            }
        }
        let repeated = retry(&mut fixture.a, cid).await;
        assert_eq!(repeated, original);
        assert_eq!(
            fixture.a.circuit_info(cid).unwrap().unwrap().expires_at,
            deadline
        );
        let (response, upstream) = finish(&mut fixture, cid, repeated);
        if let Some(original) = original_response {
            assert_eq!(response, original);
        }
        if let Some(original) = original_upstream {
            assert_eq!(upstream, original);
        }
        // Late duplicate RESPONSE is an idempotent receipt, not another Noise
        // read. It cannot create flows or reset the transport nonce windows.
        assert!(
            matches!(fixture.a.receive(&fixture.ar.carrier, upstream).unwrap(),
            Received::Established { circuit, response: None } if circuit == cid)
        );
        assert!(fixture.a.handshake_retry_at(cid).unwrap().is_none());
        assert!(fixture.a.retry_handshake(cid).unwrap().is_none());
        for node in [&fixture.a, &fixture.r, &fixture.b] {
            assert_eq!(node.retained_circuits(), lost_hop + 1);
            assert_eq!(node.retained_flows(), lost_hop);
            assert_eq!(
                node.circuit_info(cid).unwrap().unwrap().state,
                CircuitState::Established
            );
        }
        exchange(&mut fixture, cid, 50100 + lost_hop as u16);
    }
    fixture.close().await;
}

#[tokio::test]
async fn a_peer_with_a_later_real_permit_accepts_the_original_retried_init() {
    let mut fixture = SecureMesh::start().await;
    let demand = request();
    let created = fixture
        .mesh
        .a
        .control
        .create_demand(&DemandRequest {
            instance: fixture.mesh.a.instance(),
            request_id: demand,
            target: BusinessTarget::Mesh {
                node: fixture.mesh.b.id,
            },
            protocol: BusinessProtocol::Udp,
            port: Some(42424.try_into().unwrap()),
            profile: "mesh".into(),
            duration_seconds: 120,
        })
        .await
        .unwrap();
    assert!(matches!(created.decision, DemandDecision::Permitted { .. }));
    let snapshot = fixture.mesh.a.sync_current().await;
    fixture
        .a
        .reconcile(fixture.mesh.a.control.dataplane_identity(snapshot).unwrap())
        .unwrap();
    let init = fixture.a.start(demand, fixture.mesh.path.clone()).unwrap();
    let cid = init.circuit();
    let original = bytes(init);
    assert!(
        fixture
            .r
            .receive(&fixture.ar.carrier, original.clone())
            .is_err()
    );
    assert_eq!(fixture.r.retained_circuits(), 0);
    let snapshot = fixture.mesh.r.sync_current().await;
    fixture
        .r
        .reconcile(fixture.mesh.r.control.dataplane_identity(snapshot).unwrap())
        .unwrap();
    let repeated = retry(&mut fixture.a, cid).await;
    assert_eq!(repeated, original);
    let forwarded_init = forwarded(fixture.r.receive(&fixture.ar.carrier, repeated));
    // Terminal also learns this permit later; rejection cannot mint a CID.
    assert!(
        fixture
            .b
            .receive(&fixture.rb.carrier, forwarded_init)
            .is_err()
    );
    assert_eq!(fixture.b.retained_circuits(), 0);
    let snapshot = fixture.mesh.b.sync_current().await;
    fixture
        .b
        .reconcile(fixture.mesh.b.control.dataplane_identity(snapshot).unwrap())
        .unwrap();
    let repeated = retry(&mut fixture.a, cid).await;
    assert_eq!(repeated, original);
    finish(&mut fixture, cid, repeated);
    for node in [&fixture.a, &fixture.r, &fixture.b] {
        assert_eq!(node.retained_circuits(), 1);
        assert_eq!(node.retained_flows(), 0);
    }
    fixture.close().await;
}

#[tokio::test]
async fn close_proof_rejection_and_cancel_never_revive_a_cached_handshake() {
    let mut fixture = SecureMesh::start().await;
    let frame = fixture
        .a
        .start(fixture.mesh.demand, fixture.mesh.path.clone())
        .unwrap();
    let cid = frame.circuit();
    let init = bytes(frame);
    let terminal_init = forwarded(fixture.r.receive(&fixture.ar.carrier, init.clone()));
    let _lost_response = response(
        fixture
            .b
            .receive(&fixture.rb.carrier, terminal_init.clone()),
        cid,
    );
    let mut tampered = terminal_init.clone();
    *tampered.last_mut().unwrap() ^= 1;
    assert!(matches!(
        fixture.b.receive(&fixture.rb.carrier, tampered),
        Err(Error::Proof)
    ));
    assert_eq!(
        fixture.b.circuit_info(cid).unwrap().unwrap().state,
        CircuitState::Established
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let Received::Established {
        response: Some(queued_response),
        ..
    } = fixture
        .b
        .receive(&fixture.rb.carrier, terminal_init.clone())
        .unwrap()
    else {
        panic!("cached response before close");
    };
    fixture.b.close(cid);
    assert!(
        queued_response
            .release(|_| panic!("closed response escaped"))
            .is_err()
    );
    assert!(matches!(
        fixture.b.receive(&fixture.rb.carrier, terminal_init),
        Err(Error::State)
    ));
    fixture.r.close(cid);
    assert!(matches!(
        fixture.r.receive(&fixture.ar.carrier, init),
        Err(Error::State)
    ));

    tokio::time::sleep(Duration::from_millis(300)).await;
    let pending = fixture.a.retry_handshake(cid).unwrap().unwrap();
    fixture
        .mesh
        .a
        .control
        .cancel_demand(&CancelDemandRequest {
            instance: fixture.mesh.a.instance(),
            request_id: fixture.mesh.demand,
        })
        .await
        .unwrap();
    assert!(
        pending
            .release(|_| panic!("cancelled retransmission escaped"))
            .is_err()
    );
    assert!(fixture.a.retry_handshake(cid).is_err());
    fixture.close().await;
}

#[tokio::test]
async fn scope_handoff_never_rebinds_cached_outputs_even_when_noise_is_retained() {
    let mut fixture = SecureMesh::start().await;
    let frame = fixture
        .a
        .start(fixture.mesh.demand, fixture.mesh.path.clone())
        .unwrap();
    let cid = frame.circuit();
    let init = bytes(frame);
    let terminal_init = forwarded(fixture.r.receive(&fixture.ar.carrier, init.clone()));
    let response = response(
        fixture
            .b
            .receive(&fixture.rb.carrier, terminal_init.clone()),
        cid,
    );
    let _lost = forwarded(fixture.r.receive(&fixture.rb.carrier, response.clone()));
    tokio::time::sleep(Duration::from_millis(300)).await;
    let queued = fixture.a.retry_handshake(cid).unwrap().unwrap();
    let deadline = fixture.a.circuit_info(cid).unwrap().unwrap().expires_at;
    fixture.renew().await;
    assert!(
        queued
            .release(|_| panic!("old guard was extended"))
            .is_err()
    );
    assert!(fixture.a.retry_handshake(cid).unwrap().is_none());
    assert_eq!(
        fixture.a.circuit_info(cid).unwrap().unwrap().expires_at,
        deadline
    );
    assert!(fixture.r.receive(&fixture.ar.carrier, init).is_err());
    assert!(
        fixture
            .b
            .receive(&fixture.rb.carrier, terminal_init)
            .is_err()
    );
    assert!(fixture.r.receive(&fixture.rb.carrier, response).is_err());
    assert_eq!(fixture.a.retained_circuits(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn handshake_deadline_bounds_cached_response_without_expiring_an_established_terminal() {
    let mut fixture = SecureMesh::start().await;
    // Configure the production deadline for this one terminal; no opaque
    // authority or wall clock is forged. The origin keeps its full default.
    fixture.b = SecureNode::new(
        fixture
            .mesh
            .b
            .control
            .dataplane_identity(fixture.mesh.b.control.execution_snapshot().unwrap())
            .unwrap(),
        Limits {
            handshake_timeout: Duration::from_millis(500),
            ..Default::default()
        },
        BTreeMap::from([(fixture.mesh.r.id, fixture.rb.carrier.clone())]),
    )
    .unwrap();
    let init = fixture
        .a
        .start(fixture.mesh.demand, fixture.mesh.path.clone())
        .unwrap();
    let cid = init.circuit();
    let init = forwarded(fixture.r.receive(&fixture.ar.carrier, bytes(init)));
    let wire = response(fixture.b.receive(&fixture.rb.carrier, init.clone()), cid);
    let wire = forwarded(fixture.r.receive(&fixture.rb.carrier, wire));
    fixture.a.receive(&fixture.ar.carrier, wire).unwrap();
    tokio::time::sleep(Duration::from_millis(550)).await;
    assert!(fixture.b.receive(&fixture.rb.carrier, init).is_err());
    assert_eq!(
        fixture.b.circuit_info(cid).unwrap().unwrap().state,
        CircuitState::Established
    );
    exchange(&mut fixture, cid, 50200);
    fixture.close().await;
}

#[tokio::test]
async fn exhausted_handshake_keeps_its_cid_tombstone_and_cannot_retry_after_expiry() {
    let mut fixture = SecureMesh::start().await;
    fixture.a = SecureNode::new(
        fixture
            .mesh
            .a
            .control
            .dataplane_identity(fixture.mesh.a.control.execution_snapshot().unwrap())
            .unwrap(),
        Limits {
            max_circuits: 1,
            handshake_timeout: Duration::from_millis(500),
            ..Default::default()
        },
        BTreeMap::from([(fixture.mesh.r.id, fixture.ar.carrier.clone())]),
    )
    .unwrap();
    let pending = fixture
        .a
        .start(fixture.mesh.demand, fixture.mesh.path.clone())
        .unwrap();
    let cid = pending.circuit();
    let deadline = fixture.a.circuit_info(cid).unwrap().unwrap().expires_at;
    tokio::time::sleep_until((deadline + Duration::from_millis(10)).into()).await;
    assert!(pending.release(|_| panic!("expired INIT escaped")).is_err());
    assert!(fixture.a.retry_handshake(cid).is_err());
    assert!(fixture.a.handshake_retry_at(cid).is_err());
    assert_eq!(
        fixture.a.circuit_info(cid).unwrap().unwrap().state,
        CircuitState::Closed
    );
    assert_eq!(fixture.a.retained_circuits(), 1);
    assert!(matches!(
        fixture
            .a
            .start(fixture.mesh.demand, fixture.mesh.path.clone()),
        Err(Error::Capacity)
    ));
    fixture.close().await;
}

#[tokio::test]
async fn unavailable_original_carrier_is_fenced_and_failed_retry_still_consumes_its_pacing_slot() {
    let mut fixture = SecureMesh::start().await;
    let frame = fixture
        .a
        .start(fixture.mesh.demand, fixture.mesh.path.clone())
        .unwrap();
    let cid = frame.circuit();
    let original = bytes(frame);
    let due = fixture.a.handshake_retry_at(cid).unwrap().unwrap();
    fixture.a.set_carriers(BTreeMap::new()).unwrap();
    tokio::time::sleep_until(due.into()).await;
    assert!(matches!(
        fixture.a.retry_handshake(cid),
        Err(Error::Context)
    ));
    assert!(fixture.a.handshake_retry_at(cid).unwrap().unwrap() > due);
    fixture
        .a
        .set_carriers(BTreeMap::from([(
            fixture.mesh.r.id,
            fixture.ar.carrier.clone(),
        )]))
        .unwrap();
    let repeated = retry(&mut fixture.a, cid).await;
    assert_eq!(repeated, original);
    finish(&mut fixture, cid, repeated);
    assert_eq!(fixture.a.retained_circuits(), 1);
    fixture.close().await;
}
