//! Ordinary IP echo over the protected full path, not a diagnostics/CLI probe.
use super::{
    release,
    support::{Node, packet, request, secure::SecureMesh},
};
use meshway_configuration::execution::{BusinessProtocol, BusinessTarget, DemandDecision};
use meshway_dataplane::{Direction, Hop, Packet, PendingAction, secure::Received};
use meshway_protocol::DemandRequest;
use std::net::IpAddr;

fn echo(fixture: &SecureMesh, ipv6: bool, reply: bool, identifier: u16, sequence: u16) -> Packet {
    let address = |node: &Node| -> IpAddr {
        let snapshot = node.control.execution_snapshot().unwrap();
        let addresses = &snapshot.configuration().unwrap().addresses;
        if ipv6 {
            addresses.ipv6.unwrap().into()
        } else {
            addresses.ipv4.unwrap().into()
        }
    };
    let (source, destination) = if reply {
        (&fixture.mesh.b, &fixture.mesh.a)
    } else {
        (&fixture.mesh.a, &fixture.mesh.b)
    };
    packet::echo(
        address(source),
        address(destination),
        reply,
        identifier,
        sequence,
        b"protected echo",
    )
}

async fn exchange(
    fixture: &mut SecureMesh,
    cid: [u8; 32],
    reply: bool,
    packet: Packet,
) -> PendingAction {
    let forward = if reply {
        fixture
            .rb
            .listen
            .send(fixture.b.send(cid, packet).unwrap())
            .await;
        let bytes = fixture.rb.dial.recv().await;
        fixture.r.receive(&fixture.rb.carrier, bytes).unwrap()
    } else {
        fixture
            .ar
            .dial
            .send(fixture.a.send(cid, packet).unwrap())
            .await;
        let bytes = fixture.ar.listen.recv().await;
        fixture.r.receive(&fixture.ar.carrier, bytes).unwrap()
    };
    let Received::Forward(forward) = forward else {
        panic!("opaque original-path relay");
    };
    let delivery = if reply {
        fixture.ar.listen.send(forward).await;
        let bytes = fixture.ar.dial.recv().await;
        fixture.a.receive(&fixture.ar.carrier, bytes).unwrap()
    } else {
        fixture.rb.dial.send(forward).await;
        let bytes = fixture.rb.listen.recv().await;
        fixture.b.receive(&fixture.rb.carrier, bytes).unwrap()
    };
    let Received::Packet(delivery) = delivery else {
        panic!("endpoint echo");
    };
    delivery
}

#[tokio::test]
async fn protected_dual_stack_echo_keeps_exact_flow_and_reply_path_through_renewal_and_revocation()
{
    let mut fixture = SecureMesh::start().await;
    fixture
        .mesh
        .server
        .configure(&fixture.mesh.owner, |input| {
            input.access[0].ip.push("icmp:*".into())
        })
        .await;
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
            protocol: BusinessProtocol::IcmpEcho,
            port: None,
            profile: "mesh".into(),
            duration_seconds: 120,
        })
        .await
        .unwrap();
    assert!(matches!(
        response.decision,
        DemandDecision::Permitted { .. }
    ));
    fixture.reconcile().await;
    let cid = fixture.handshake().await;
    let mut queued = Vec::new();
    for ipv6 in [false, true] {
        assert!(
            fixture
                .b
                .send(cid, echo(&fixture, ipv6, true, 12, 34))
                .is_err()
        );
        let request = echo(&fixture, ipv6, false, 12, 34);
        let expected = request.as_bytes().to_vec();
        let delivery = exchange(&mut fixture, cid, false, request).await;
        assert_eq!(
            release(&fixture.mesh, delivery, Hop::Deliver, Direction::Forward).into_bytes(),
            expected
        );
        for (identifier, sequence) in [(13, 34), (12, 35)] {
            assert!(
                fixture
                    .b
                    .send(cid, echo(&fixture, ipv6, true, identifier, sequence))
                    .is_err()
            );
        }
        let reply = echo(&fixture, ipv6, true, 12, 34);
        let flow = fixture.b.local_flow(&reply).unwrap().unwrap();
        assert_eq!(flow.demand, fixture.mesh.demand);
        assert_eq!(flow.path, fixture.mesh.path);
        assert_eq!(flow.circuit, Some(cid));
        let facts = reply.facts();
        let backwards_request = packet::echo(
            facts.traffic.source,
            facts.traffic.destination,
            false,
            12,
            34,
            b"protected echo",
        );
        assert!(fixture.b.send(cid, backwards_request).is_err());
        let expected = reply.as_bytes().to_vec();
        let delivery = exchange(&mut fixture, cid, true, reply).await;
        assert_eq!(
            release(&fixture.mesh, delivery, Hop::Deliver, Direction::Reply).into_bytes(),
            expected
        );
        let request = echo(&fixture, ipv6, false, 12, 34);
        queued.push(exchange(&mut fixture, cid, false, request).await);
    }
    let old = fixture.mesh.a.control.execution_snapshot().unwrap();
    fixture.renew().await;
    assert!(old.ensure_current().is_err());
    for delivery in queued {
        assert!(
            delivery
                .release(|_| panic!("old echo delivery escaped"))
                .is_err()
        );
    }
    // No new forward: a reply proves the retained exact echo state/cipher.
    let mut queued = Vec::new();
    for ipv6 in [false, true] {
        let reply = echo(&fixture, ipv6, true, 12, 34);
        let delivery = exchange(&mut fixture, cid, true, reply).await;
        release(&fixture.mesh, delivery, Hop::Deliver, Direction::Reply);
        let reply = echo(&fixture, ipv6, true, 12, 34);
        queued.push(exchange(&mut fixture, cid, true, reply).await);
    }
    for secure in [&fixture.a, &fixture.r, &fixture.b] {
        assert_eq!(secure.retained_flows(), 2);
        assert_eq!(secure.ready_circuits().unwrap()[0].id, cid);
    }
    fixture
        .mesh
        .server
        .configure(&fixture.mesh.owner, |input| input.access.clear())
        .await;
    fixture.reconcile().await;
    for delivery in queued {
        assert!(
            delivery
                .release(|_| panic!("revoked echo reply escaped"))
                .is_err()
        );
    }
    for secure in [&fixture.a, &fixture.r, &fixture.b] {
        assert_eq!(secure.retained_flows(), 0);
        assert!(secure.ready_circuits().unwrap().is_empty());
    }
    assert!(
        fixture
            .a
            .send(cid, echo(&fixture, false, false, 12, 34))
            .is_err()
    );
    fixture.close().await;
}
