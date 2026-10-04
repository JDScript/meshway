//! Actual registered Control keys, ordered proof chain and native TCP wire I/O.
use super::{
    datagram, release,
    support::{
        packet,
        secure::{IO_WAIT, SENTINEL, SecureMesh},
    },
};
use meshway_dataplane::{Direction, Hop, secure::Received};

#[tokio::test]
async fn circuit_close_and_owner_drop_fence_outputs_while_control_remains_live() {
    let mut fixture = SecureMesh::start().await;
    let circuit = fixture.handshake().await;
    let queued = fixture
        .a
        .send(circuit, datagram(&fixture.mesh, false, false))
        .unwrap();
    let wire = fixture
        .a
        .send(circuit, datagram(&fixture.mesh, true, false))
        .unwrap()
        .release(|bytes| bytes)
        .unwrap();
    let Received::Forward(forward) = fixture.r.receive(&fixture.ar.carrier, wire).unwrap() else {
        panic!("opaque relay");
    };
    let Received::Packet(delivery) = fixture
        .b
        .receive(&fixture.rb.carrier, forward.release(|bytes| bytes).unwrap())
        .unwrap()
    else {
        panic!("endpoint delivery");
    };
    fixture.a.close(circuit);
    fixture.b.close(circuit);
    fixture
        .mesh
        .a
        .control
        .execution_snapshot()
        .unwrap()
        .ensure_current()
        .unwrap();
    fixture
        .mesh
        .b
        .control
        .execution_snapshot()
        .unwrap()
        .ensure_current()
        .unwrap();
    tokio::time::timeout(IO_WAIT, queued.invalidated())
        .await
        .unwrap();
    tokio::time::timeout(IO_WAIT, delivery.invalidated())
        .await
        .unwrap();
    let mut released = false;
    assert!(queued.release(|_| released = true).is_err());
    assert!(delivery.release(|_| released = true).is_err());
    assert!(!released);
    let pending = fixture
        .a
        .start(fixture.mesh.demand, fixture.mesh.path.clone())
        .unwrap();
    let replacement = meshway_dataplane::secure::SecureNode::new(
        fixture
            .mesh
            .a
            .control
            .dataplane_identity(fixture.mesh.a.control.execution_snapshot().unwrap())
            .unwrap(),
        Default::default(),
        std::collections::BTreeMap::from([(fixture.mesh.r.id, fixture.ar.carrier.clone())]),
    )
    .unwrap();
    drop(std::mem::replace(&mut fixture.a, replacement));
    tokio::time::timeout(IO_WAIT, pending.invalidated())
        .await
        .unwrap();
    assert!(pending.release(|_| released = true).is_err());
    assert!(!released);
    fixture
        .mesh
        .a
        .control
        .execution_snapshot()
        .unwrap()
        .ensure_current()
        .unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn real_control_crypto_native_tcp_ipv4_ipv6_and_exact_reverse_path() {
    let mut fixture = SecureMesh::start().await;
    let circuit = fixture.handshake().await;
    for ipv6 in [false, true] {
        let address = |node: &super::Node| {
            let snapshot = node.control.execution_snapshot().unwrap();
            let addresses = &snapshot.configuration().unwrap().addresses;
            if ipv6 {
                addresses.ipv6.unwrap().into()
            } else {
                addresses.ipv4.unwrap().into()
            }
        };
        let make = || {
            packet::udp(
                address(&fixture.mesh.a),
                address(&fixture.mesh.b),
                50000,
                42424,
                SENTINEL,
            )
        };
        let expected = make().into_bytes();
        let pending = fixture.a.send(circuit, make()).unwrap();
        fixture.ar.dial.send(pending).await;
        let bytes = fixture.ar.listen.recv().await;
        let Received::Forward(pending) = fixture.r.receive(&fixture.ar.carrier, bytes).unwrap()
        else {
            panic!("relay decrypted application data")
        };
        fixture.rb.dial.send(pending).await;
        let bytes = fixture.rb.listen.recv().await;
        let Received::Packet(delivery) = fixture.b.receive(&fixture.rb.carrier, bytes).unwrap()
        else {
            panic!("missing endpoint delivery")
        };
        assert_eq!(
            release(&fixture.mesh, delivery, Hop::Deliver, Direction::Forward).into_bytes(),
            expected
        );
        let reply = datagram(&fixture.mesh, ipv6, true);
        let expected = datagram(&fixture.mesh, ipv6, true).into_bytes();
        let pending = fixture.b.send(circuit, reply).unwrap();
        fixture.rb.listen.send(pending).await;
        let bytes = fixture.rb.dial.recv().await;
        let Received::Forward(pending) = fixture.r.receive(&fixture.rb.carrier, bytes).unwrap()
        else {
            panic!("relay decrypted reply")
        };
        fixture.ar.listen.send(pending).await;
        let bytes = fixture.ar.dial.recv().await;
        let Received::Packet(delivery) = fixture.a.receive(&fixture.ar.carrier, bytes).unwrap()
        else {
            panic!("missing reply delivery")
        };
        assert_eq!(
            release(&fixture.mesh, delivery, Hop::Deliver, Direction::Reply).into_bytes(),
            expected
        );
    }
    fixture.close().await;
}

#[tokio::test]
async fn authentic_frames_reject_tampering_replay_and_skipped_relay_without_delivery() {
    let mut fixture = SecureMesh::start().await;
    assert!(
        fixture
            .a
            .start(
                fixture.mesh.demand,
                vec![fixture.mesh.a.id, fixture.mesh.b.id]
            )
            .is_err()
    );
    let circuit = fixture.handshake().await;
    let wire = fixture
        .a
        .send(circuit, datagram(&fixture.mesh, false, false))
        .unwrap()
        .release(|bytes| bytes)
        .unwrap();
    assert!(
        fixture
            .b
            .receive(&fixture.rb.carrier, wire.clone())
            .is_err()
    );
    let mut tampered = wire.clone();
    *tampered.last_mut().unwrap() ^= 1;
    assert!(fixture.r.receive(&fixture.ar.carrier, tampered).is_err());
    let Received::Forward(forward) = fixture
        .r
        .receive(&fixture.ar.carrier, wire.clone())
        .unwrap()
    else {
        panic!("relay must stay opaque")
    };
    assert!(fixture.r.receive(&fixture.ar.carrier, wire).is_err());
    let wire = forward.release(|bytes| bytes).unwrap();
    let mut tampered = wire.clone();
    *tampered.last_mut().unwrap() ^= 1;
    assert!(fixture.b.receive(&fixture.rb.carrier, tampered).is_err());
    assert!(matches!(
        fixture
            .b
            .receive(&fixture.rb.carrier, wire.clone())
            .unwrap(),
        Received::Packet(_)
    ));
    assert!(fixture.b.receive(&fixture.rb.carrier, wire).is_err());
    fixture.close().await;
}

#[tokio::test]
async fn control_cancellation_publication_and_stop_fence_crypto_and_plaintext_outputs() {
    use meshway_protocol::CancelDemandRequest;
    let mut fixture = SecureMesh::start().await;
    let circuit = fixture.handshake().await;
    let queued = fixture
        .a
        .send(circuit, datagram(&fixture.mesh, false, false))
        .unwrap();
    let wire = fixture
        .a
        .send(circuit, datagram(&fixture.mesh, true, false))
        .unwrap()
        .release(|bytes| bytes)
        .unwrap();
    let Received::Forward(forward) = fixture.r.receive(&fixture.ar.carrier, wire).unwrap() else {
        panic!("relay must stay opaque")
    };
    let wire = forward.release(|bytes| bytes).unwrap();
    let Received::Packet(delivery) = fixture.b.receive(&fixture.rb.carrier, wire).unwrap() else {
        panic!("missing delivery")
    };
    let instance = fixture.mesh.a.instance();
    fixture
        .mesh
        .a
        .control
        .cancel_demand(&CancelDemandRequest {
            instance,
            request_id: fixture.mesh.demand,
        })
        .await
        .unwrap();
    tokio::time::timeout(IO_WAIT, queued.invalidated())
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(
            IO_WAIT,
            meshway_dataplane::carrier::send(
                &mut fixture.ar.dial.sender,
                queued,
                Some(fixture.ar.dial.peer),
            )
        )
        .await
        .unwrap()
        .is_err()
    );
    // The formal sink may close the stream when rejecting the operation; either
    // closure or a read timeout proves that no stale ciphertext frame arrived.
    assert!(!matches!(
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            fixture.ar.listen.receiver.recv()
        )
        .await,
        Ok(Ok(_))
    ));
    let mut sent = false;
    fixture
        .mesh
        .server
        .configure(&fixture.mesh.owner, |input| input.access.clear())
        .await;
    fixture.mesh.b.sync_current().await;
    tokio::time::timeout(IO_WAIT, delivery.invalidated())
        .await
        .unwrap();
    assert!(delivery.release(|_| sent = true).is_err());
    assert!(!sent);
    fixture.close().await;

    let mut fixture = SecureMesh::start().await;
    let pending = fixture
        .a
        .start(fixture.mesh.demand, fixture.mesh.path.clone())
        .unwrap();
    fixture.mesh.a.control.stop().await.unwrap();
    tokio::time::timeout(IO_WAIT, pending.invalidated())
        .await
        .unwrap();
    assert!(pending.release(|_| sent = true).is_err());
    assert!(!sent);
    fixture.close().await;
}
