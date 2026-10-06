//! Real Server publications and opaque Control successors, using production
//! Noise/proof/flow owners. No caller-minted execution authority.
use super::{
    datagram,
    support::{Node, request, secure::SecureMesh},
};
use meshway_configuration::{
    AccessRule,
    execution::{BusinessProtocol, BusinessTarget, DemandDecision},
};
use meshway_control::DataPlaneIdentity;
use meshway_dataplane::secure::{CircuitState, Received};
use meshway_protocol::{CancelDemandRequest, DemandRequest, RenewDemandRequest};

async fn identity(node: &mut Node) -> DataPlaneIdentity {
    let snapshot = node.sync_current().await;
    node.control.dataplane_identity(snapshot).unwrap()
}

async fn reconcile(fixture: &mut SecureMesh) {
    fixture
        .a
        .reconcile(identity(&mut fixture.mesh.a).await)
        .unwrap();
    fixture
        .r
        .reconcile(identity(&mut fixture.mesh.r).await)
        .unwrap();
    fixture
        .b
        .reconcile(identity(&mut fixture.mesh.b).await)
        .unwrap();
}

fn forward(fixture: &mut SecureMesh, circuit: [u8; 32]) -> Vec<u8> {
    let wire = fixture
        .a
        .send(circuit, datagram(&fixture.mesh, false, false))
        .unwrap()
        .release(|bytes| bytes)
        .unwrap();
    let Received::Forward(next) = fixture
        .r
        .receive(&fixture.ar.carrier, wire.clone())
        .unwrap()
    else {
        panic!("relay");
    };
    let Received::Packet(packet) = fixture
        .b
        .receive(&fixture.rb.carrier, next.release(|bytes| bytes).unwrap())
        .unwrap()
    else {
        panic!("terminal");
    };
    packet.release(|_| ()).unwrap();
    wire
}

fn reply(fixture: &mut SecureMesh, circuit: [u8; 32]) {
    let wire = fixture
        .b
        .send(circuit, datagram(&fixture.mesh, false, true))
        .unwrap()
        .release(|bytes| bytes)
        .unwrap();
    let Received::Forward(next) = fixture.r.receive(&fixture.rb.carrier, wire).unwrap() else {
        panic!("reply relay");
    };
    let Received::Packet(packet) = fixture
        .a
        .receive(&fixture.ar.carrier, next.release(|bytes| bytes).unwrap())
        .unwrap()
    else {
        panic!("reply origin");
    };
    packet.release(|_| ()).unwrap();
}

#[tokio::test]
async fn adding_another_destination_preserves_flow_nonce_and_replay_but_cancels_old_output() {
    let mut fixture = SecureMesh::start().await;
    let source = fixture.mesh.a.id;
    let other = fixture.mesh.r.id;
    fixture
        .mesh
        .server
        .configure(&fixture.mesh.owner, |input| {
            input.access.push(AccessRule {
                remark: String::new(),
                src: vec![format!("node:{source}")],
                dst: vec![format!("node:{other}")],
                ip: vec!["udp:42424".into()],
                bidirectional: false,
            });
        })
        .await;
    reconcile(&mut fixture).await;
    let circuit = fixture.handshake().await;
    let replay = forward(&mut fixture, circuit);
    reply(&mut fixture, circuit);
    let queued = fixture
        .a
        .send(circuit, datagram(&fixture.mesh, false, false))
        .unwrap();
    let old = fixture.mesh.a.control.execution_snapshot().unwrap();
    let response = fixture
        .mesh
        .a
        .control
        .create_demand(&DemandRequest {
            instance: fixture.mesh.a.instance(),
            request_id: request(),
            target: BusinessTarget::Mesh { node: other },
            protocol: BusinessProtocol::Udp,
            port: Some(42424.try_into().unwrap()),
            profile: "mesh".into(),
            duration_seconds: 120,
        })
        .await
        .unwrap();
    assert!(matches!(
        response.decision,
        DemandDecision::Permitted { .. }
    ));
    reconcile(&mut fixture).await;
    assert!(old.ensure_current().is_err());
    assert!(queued.release(|_| panic!("cancelled old output")).is_err());
    for node in [&fixture.a, &fixture.r, &fixture.b] {
        assert_eq!(node.retained_flows(), 1);
        assert_eq!(node.ready_circuits().unwrap()[0].id, circuit);
    }
    // A reply before a new forward proves the original unique flow survived.
    reply(&mut fixture, circuit);
    assert!(fixture.r.receive(&fixture.ar.carrier, replay).is_err());
    // Resetting the sender nonce or receiver replay window would fail this pair.
    forward(&mut fixture, circuit);
    reply(&mut fixture, circuit);
    let before = fixture.a.circuit_info(circuit).unwrap().unwrap();
    reconcile(&mut fixture).await;
    assert_eq!(fixture.a.retained_flows(), 1);
    assert_eq!(
        fixture.a.circuit_info(circuit).unwrap().unwrap().state,
        before.state
    );
    fixture.close().await;
}

#[tokio::test]
async fn explicit_renewal_extends_live_circuit_but_never_reopens_a_closed_circuit() {
    let mut fixture = SecureMesh::start().await;
    let circuit = fixture.handshake().await;
    forward(&mut fixture, circuit);
    let closed = fixture.handshake().await;
    fixture.a.close(closed);
    fixture.r.close(closed);
    fixture.b.close(closed);
    let old = fixture.mesh.a.control.execution_snapshot().unwrap();
    let old_permit = old.configuration().unwrap().permits[&fixture.mesh.demand].clone();
    fixture.mesh.a.control.renew_instance().await.unwrap();
    let cap = fixture
        .mesh
        .a
        .control
        .status()
        .instance
        .unwrap()
        .expires_at_ms;
    let response = fixture
        .mesh
        .a
        .control
        .renew_demand(&RenewDemandRequest {
            instance: fixture.mesh.a.instance(),
            request_id: fixture.mesh.demand,
            expires_at_ms: cap,
        })
        .await
        .unwrap();
    let DemandDecision::Permitted { permit } = response.decision else {
        panic!("renewed permit");
    };
    assert!(permit.expires_at_ms > old_permit.expires_at_ms);
    assert_eq!(
        meshway_policy::permit_authorization_digest(&permit).unwrap(),
        meshway_policy::permit_authorization_digest(&old_permit).unwrap()
    );
    assert_ne!(
        meshway_configuration::digest(&permit).unwrap(),
        meshway_configuration::digest(&old_permit).unwrap()
    );
    reconcile(&mut fixture).await;
    assert!(old.ensure_current().is_err());
    reply(&mut fixture, circuit);
    forward(&mut fixture, circuit);
    for node in [&fixture.a, &fixture.r, &fixture.b] {
        assert_eq!(
            node.circuit_info(closed).unwrap().unwrap().state,
            CircuitState::Closed
        );
    }
    fixture.close().await;
}

#[tokio::test]
async fn observed_revocation_then_regrant_cannot_be_skipped_by_a_late_reconciler() {
    let mut fixture = SecureMesh::start().await;
    let circuit = fixture.handshake().await;
    forward(&mut fixture, circuit);
    fixture
        .mesh
        .server
        .configure(&fixture.mesh.owner, |input| input.access.clear())
        .await;
    let withdrawn = fixture.mesh.a.sync_current().await;
    assert!(
        !withdrawn
            .configuration()
            .unwrap()
            .permits
            .contains_key(&fixture.mesh.demand)
    );
    fixture
        .r
        .reconcile(identity(&mut fixture.mesh.r).await)
        .unwrap();
    assert_eq!(fixture.r.retained_flows(), 0);
    assert_eq!(
        fixture.r.circuit_info(circuit).unwrap().unwrap().state,
        CircuitState::Closed
    );
    let a = fixture.mesh.a.id;
    let b = fixture.mesh.b.id;
    fixture
        .mesh
        .server
        .configure(&fixture.mesh.owner, |input| {
            input.access = vec![AccessRule {
                remark: String::new(),
                src: vec![format!("node:{a}")],
                dst: vec![format!("node:{b}")],
                ip: vec!["udp:42424".into()],
                bidirectional: false,
            }];
        })
        .await;
    let replacement = identity(&mut fixture.mesh.a).await;
    assert!(fixture.a.reconcile(replacement).is_err());
    fixture
        .r
        .reconcile(identity(&mut fixture.mesh.r).await)
        .unwrap();
    assert_eq!(fixture.r.retained_flows(), 0);
    assert_eq!(
        fixture.r.circuit_info(circuit).unwrap().unwrap().state,
        CircuitState::Closed
    );
    fixture.close().await;
}

#[tokio::test]
async fn cancellation_breaks_local_lineage_and_same_id_recreation_is_rejected() {
    let mut fixture = SecureMesh::start().await;
    let circuit = fixture.handshake().await;
    forward(&mut fixture, circuit);
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
        fixture
            .a
            .reconcile(identity(&mut fixture.mesh.a).await)
            .is_err()
    );
    assert!(
        fixture
            .mesh
            .a
            .control
            .create_demand(&DemandRequest {
                instance: fixture.mesh.a.instance(),
                request_id: fixture.mesh.demand,
                target: BusinessTarget::Mesh {
                    node: fixture.mesh.b.id,
                },
                protocol: BusinessProtocol::Udp,
                port: Some(42424.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 120,
            })
            .await
            .is_err()
    );
    fixture.close().await;
}

#[tokio::test]
async fn an_expired_permit_cannot_be_revived_while_the_node_execution_lease_remains_live() {
    let mut fixture = SecureMesh::start().await;
    fixture.mesh.demand = request();
    let response = fixture
        .mesh
        .a
        .control
        .create_demand(&DemandRequest {
            instance: fixture.mesh.a.instance(),
            request_id: fixture.mesh.demand,
            target: BusinessTarget::Mesh {
                node: fixture.mesh.b.id,
            },
            protocol: BusinessProtocol::Udp,
            port: Some(42424.try_into().unwrap()),
            profile: "mesh".into(),
            // Leave time for the three real HTTP installations and handshake;
            // this verifies expiry while execution stays live, not setup speed.
            duration_seconds: 5,
        })
        .await
        .unwrap();
    let DemandDecision::Permitted { permit } = response.decision else {
        panic!("short permitted demand");
    };
    reconcile(&mut fixture).await;
    let old = fixture.mesh.a.control.execution_snapshot().unwrap();
    let circuit = fixture.handshake().await;
    forward(&mut fixture, circuit);
    let queued = fixture
        .a
        .send(circuit, datagram(&fixture.mesh, false, false))
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(7),
        old.invalidated_at(permit.expires_at_ms),
    )
    .await
    .unwrap();
    old.ensure_current().unwrap();
    assert!(queued.ensure_current().is_err());
    assert!(
        fixture
            .a
            .send(circuit, datagram(&fixture.mesh, false, false))
            .is_err()
    );
    let fresh = identity(&mut fixture.mesh.a).await;
    fresh.snapshot().ensure_current().unwrap();
    fresh.snapshot().continuation_from(&old).unwrap();
    assert!(
        !fresh
            .snapshot()
            .configuration()
            .unwrap()
            .permits
            .contains_key(&fixture.mesh.demand)
    );
    fixture.a.reconcile(fresh).unwrap();
    assert_eq!(
        fixture.a.circuit_info(circuit).unwrap().unwrap().state,
        CircuitState::Closed
    );
    assert!(
        fixture
            .a
            .send(circuit, datagram(&fixture.mesh, false, false))
            .is_err()
    );
    fixture.close().await;
}
