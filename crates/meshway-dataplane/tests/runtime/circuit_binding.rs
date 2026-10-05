//! A lost final Noise response leaves a real local endpoint, but it must not
//! select the return cipher for a flow delivered over a different circuit.
use super::{
    datagram,
    support::secure::{IO_WAIT, SecureMesh},
};
use meshway_dataplane::{
    carrier,
    secure::{CircuitState, Received},
};
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct RetiredChannel(CancellationToken);
impl meshway_transport::SendLifetime for RetiredChannel {
    fn is_current(&self) -> bool {
        !self.0.is_cancelled()
    }
    fn invalidated(self: Arc<Self>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move { self.0.cancelled().await })
    }
}

#[tokio::test]
async fn lost_response_and_new_circuit_do_not_select_an_orphan_return_cipher() {
    let mut fixture = SecureMesh::start().await;
    let first = fixture
        .a
        .start(fixture.mesh.demand, fixture.mesh.path.clone())
        .unwrap();
    let second = fixture
        .a
        .start(fixture.mesh.demand, fixture.mesh.path.clone())
        .unwrap();
    // Exercise the adverse BTreeMap ordering deterministically, without
    // guessing random IDs or repeating the test until their order suits it.
    let (orphan, current) = if first.circuit() < second.circuit() {
        (first, second)
    } else {
        (second, first)
    };
    let orphan_id = orphan.circuit();
    let current_id = current.circuit();
    for (lost, init) in [(true, orphan), (false, current)] {
        fixture.ar.dial.send(init).await;
        let wire = fixture.ar.listen.recv().await;
        let Received::Forward(init) = fixture.r.receive(&fixture.ar.carrier, wire).unwrap() else {
            panic!("relay init");
        };
        fixture.rb.dial.send(init).await;
        let wire = fixture.rb.listen.recv().await;
        let Received::Established {
            response: Some(response),
            ..
        } = fixture.b.receive(&fixture.rb.carrier, wire).unwrap()
        else {
            panic!("terminal response");
        };
        if lost {
            let channel = Arc::new(RetiredChannel(CancellationToken::new()));
            channel.0.cancel();
            assert!(
                carrier::send_scoped(
                    &mut fixture.rb.listen.sender,
                    response,
                    Some(fixture.rb.listen.peer),
                    Some(channel)
                )
                .await
                .is_err()
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(20), fixture.rb.dial.receiver.recv())
                    .await
                    .is_err()
            );
            assert_eq!(
                fixture.b.circuit_info(orphan_id).unwrap().unwrap().state,
                CircuitState::Established
            );
            assert_eq!(
                fixture.r.circuit_info(orphan_id).unwrap().unwrap().state,
                CircuitState::Handshaking
            );
            fixture.a.close(orphan_id);
        } else {
            fixture.rb.listen.send(response).await;
            let wire = fixture.rb.dial.recv().await;
            let Received::Forward(response) = fixture.r.receive(&fixture.rb.carrier, wire).unwrap()
            else {
                panic!("relay response");
            };
            fixture.ar.listen.send(response).await;
            let wire = fixture.ar.dial.recv().await;
            assert!(
                matches!(fixture.a.receive(&fixture.ar.carrier, wire).unwrap(), Received::Established { circuit, response: None } if circuit == current_id)
            );
        }
    }
    for ipv6 in [false, true] {
        let reply = datagram(&fixture.mesh, ipv6, true);
        assert!(
            fixture
                .b
                .local_circuit(fixture.mesh.demand, &fixture.mesh.path, &reply)
                .unwrap()
                .is_none(),
            "completed Noise responses alone cannot choose a terminal flow cipher"
        );
        let forward = datagram(&fixture.mesh, ipv6, false);
        fixture
            .ar
            .dial
            .send(fixture.a.send(current_id, forward).unwrap())
            .await;
        let wire = fixture.ar.listen.recv().await;
        let Received::Forward(forward) = fixture.r.receive(&fixture.ar.carrier, wire).unwrap()
        else {
            panic!("relay data");
        };
        fixture.rb.dial.send(forward).await;
        let wire = fixture.rb.listen.recv().await;
        assert!(
            fixture
                .b
                .receive(&fixture.ar.carrier, wire.clone())
                .is_err()
        );
        assert!(
            fixture.b.local_flow(&reply).unwrap().is_none(),
            "a wrong incoming carrier cannot establish a cipher binding"
        );
        let replay = wire.clone();
        let Received::Packet(delivery) = fixture.b.receive(&fixture.rb.carrier, wire).unwrap()
        else {
            panic!("terminal data");
        };
        delivery.release(|_| ()).unwrap();
        assert!(fixture.b.receive(&fixture.rb.carrier, replay).is_err());
        let reply = datagram(&fixture.mesh, ipv6, true);
        let flow = fixture.b.local_flow(&reply).unwrap().unwrap();
        assert_eq!(flow.circuit, Some(current_id));
        let selected = fixture
            .b
            .local_circuit(flow.demand, &flow.path, &reply)
            .unwrap()
            .unwrap();
        assert_eq!(
            selected.id, current_id,
            "return traffic must use the circuit that actually delivered its forward flow"
        );
        assert!(
            fixture
                .b
                .send(orphan_id, datagram(&fixture.mesh, ipv6, true))
                .is_err(),
            "even an explicit caller cannot send the flow's reply over the orphan cipher"
        );
        assert_eq!(
            fixture
                .b
                .local_flow(&datagram(&fixture.mesh, ipv6, true))
                .unwrap()
                .unwrap()
                .circuit,
            Some(current_id),
            "rejected explicit output does not change the binding"
        );
        fixture
            .rb
            .listen
            .send(fixture.b.send(selected.id, reply).unwrap())
            .await;
        let wire = fixture.rb.dial.recv().await;
        let Received::Forward(reply) = fixture.r.receive(&fixture.rb.carrier, wire).unwrap() else {
            panic!("relay reply");
        };
        fixture.ar.listen.send(reply).await;
        let wire = fixture.ar.dial.recv().await;
        let Received::Packet(delivery) = fixture.a.receive(&fixture.ar.carrier, wire).unwrap()
        else {
            panic!("original origin");
        };
        delivery.release(|_| ()).unwrap();
    }
    tokio::time::timeout(IO_WAIT, fixture.close())
        .await
        .unwrap();
}
