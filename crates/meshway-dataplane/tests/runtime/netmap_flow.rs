//! Netmap local admission on real SecureNodes A→R→B: first packets, in-place
//! content updates, revocation, withdrawal and relay transit admission.
use super::support::netmap::{NetmapMesh, rule, sel};
use meshway_configuration::execution::UnavailableReason;
use meshway_dataplane::{
    Packet,
    secure::{
        Applied, CircuitGrant, CircuitState, Error, Flushed, Outbound, PendingFrame, Received,
        pipeline,
    },
};
use std::{collections::BTreeMap, time::Duration};

pub(super) fn wire(frame: PendingFrame) -> Vec<u8> {
    frame.release(|bytes| bytes.into_vec()).unwrap()
}

/// A's output through R to B; returns B's receive result.
pub(super) fn deliver(f: &mut NetmapMesh, frame: PendingFrame) -> Result<Received, Error> {
    let Received::Forward(next) = f.r.receive(&f.ar.carrier, wire(frame)).unwrap() else {
        panic!("relay stays opaque")
    };
    f.b.receive(&f.rb.carrier, wire(next))
}

pub(super) fn delivered(f: &mut NetmapMesh, frame: PendingFrame) {
    let Received::Packet(packet) = deliver(f, frame).unwrap() else {
        panic!("terminal delivery")
    };
    packet.release(|_| ()).unwrap();
}

/// B's reply through R to A.
pub(super) fn reply(f: &mut NetmapMesh, id: [u8; 32], packet: Packet) -> Result<Received, Error> {
    let frame = f.b.send(id, packet)?;
    let Received::Forward(next) = f.r.receive(&f.rb.carrier, wire(frame)).unwrap() else {
        panic!("relay stays opaque")
    };
    f.a.receive(&f.ar.carrier, wire(next))
}

pub(super) fn sealed(job: pipeline::SendJob) -> PendingFrame {
    pipeline::seal(job).unwrap()
}

/// Send one packet of an existing flow or established circuit.
pub(super) fn send(f: &mut NetmapMesh, packet: Packet) -> PendingFrame {
    let Outbound::Send(job) = f.a.outbound(packet).unwrap() else {
        panic!("established output")
    };
    sealed(job)
}

/// First packet: local decision, handshake, then the queued packet.
pub(super) async fn first(f: &mut NetmapMesh, packet: Packet) -> [u8; 32] {
    let Outbound::Queued { init: Some(init) } = f.a.outbound(packet).unwrap() else {
        panic!("a new flow starts its circuit")
    };
    let id = f.finish_handshake(init).await;
    assert!(
        f.a.flush(0).is_empty(),
        "no output capacity commits no first packet"
    );
    let mut flushed = f.a.flush(1);
    assert_eq!(flushed.len(), 1);
    let Ok(Flushed::Send(job)) = flushed.remove(0) else {
        panic!("queued first packet")
    };
    assert_eq!(job.circuit(), id);
    delivered(f, sealed(job));
    id
}

pub(super) fn icmp(f: &NetmapMesh, reply: bool, sequence: u16) -> Packet {
    let (a, b) = (f.address(0), f.address(2));
    if reply {
        super::packet::echo(b, a, true, 7, sequence, b"ping")
    } else {
        super::packet::echo(a, b, false, 7, sequence, b"ping")
    }
}

#[tokio::test]
async fn dns_circuit_authenticates_the_exit_service_and_never_admits_business_packets() {
    let mut f = NetmapMesh::start().await;
    let grant = CircuitGrant {
        purpose: meshway_dataplane::Purpose::Dns,
        target: f.mesh_target(),
    };
    let path = f.path();
    assert!(
        f.a.start_grant(grant.clone(), vec![path[0], path[2]], &f.ar.carrier)
            .is_err()
    );
    let first = f.a.start_grant(grant, path, &f.ar.carrier).unwrap();
    let circuit = f.finish_handshake(first).await;
    let mut message = hickory_proto::op::Message::new(
        123,
        hickory_proto::op::MessageType::Query,
        hickory_proto::op::OpCode::Query,
    );
    message.add_query(hickory_proto::op::Query::query(
        hickory_proto::rr::Name::from_ascii("dns.example.").unwrap(),
        hickory_proto::rr::RecordType::A,
    ));
    let query = message.to_vec().unwrap();
    message.metadata.message_type = hickory_proto::op::MessageType::Response;
    let response = message.to_vec().unwrap();
    for transport in [
        meshway_configuration::DnsTransport::Udp,
        meshway_configuration::DnsTransport::Tcp,
    ] {
        let id = meshway_model::EventId::new(super::support::request().into_uuid()).unwrap();
        let frame =
            f.a.query_dns(
                circuit,
                id,
                transport,
                &query,
                std::time::Instant::now() + Duration::from_secs(2),
            )
            .unwrap();
        let Received::DnsRequest(request) = deliver(&mut f, frame).unwrap() else {
            panic!("authenticated DNS request");
        };
        assert_eq!(request.bytes().unwrap(), query);
        assert_eq!(request.source(), f.ids()[0]);
        assert_eq!(request.path(), f.path());
        assert_eq!(request.transport(), transport);
        let reply = f.b.answer_dns(request, &response).unwrap();
        let replay = wire(reply);
        let Received::Forward(frame) = f.r.receive(&f.rb.carrier, replay.clone()).unwrap() else {
            panic!("opaque DNS reply");
        };
        assert!(f.r.receive(&f.rb.carrier, replay).is_err());
        let Received::DnsReply(reply) = f.a.receive(&f.ar.carrier, wire(frame)).unwrap() else {
            panic!("correlated DNS reply");
        };
        assert_eq!(reply.id(), id);
        assert_eq!(reply.release(|bytes| bytes).unwrap(), response);
    }
    let packet = f.mesh_udp(false, 42424);
    assert!(f.a.send(circuit, packet).is_err());
    assert_eq!(
        (f.a.flow_count(), f.r.flow_count(), f.b.flow_count()),
        (0, 0, 0)
    );
    let request_id = meshway_model::EventId::new(super::support::request().into_uuid()).unwrap();
    let frame =
        f.a.query_dns(
            circuit,
            request_id,
            meshway_configuration::DnsTransport::Udp,
            &query,
            std::time::Instant::now() + Duration::from_secs(2),
        )
        .unwrap();
    let Received::DnsRequest(pending) = deliver(&mut f, frame).unwrap() else {
        panic!("pending query");
    };
    f.publish(|cfg, _| cfg.access.clear()).await;
    assert!(pending.bytes().is_err());
    assert!(f.b.answer_dns(pending, &response).is_err());
    assert!(f.a.circuit_info(circuit).unwrap().is_none());
    f.close().await;
}

#[tokio::test]
async fn first_packet_is_decided_and_handshaken_locally_without_any_control_call() {
    let mut f = NetmapMesh::start().await;
    let calls = f.sync_calls();
    let id = {
        let packet = f.mesh_udp(false, 42424);
        first(&mut f, packet)
    }
    .await;
    let Received::Packet(packet) = {
        let packet = f.mesh_udp(true, 42424);
        reply(&mut f, id, packet)
    }
    .unwrap() else {
        panic!("reply on the original path")
    };
    packet.release(|_| ()).unwrap();
    // A denied service never starts a circuit; an exit without ready facts is dropped.
    assert!(matches!(
        {
            let packet = f.mesh_udp(false, 42425);
            f.a.outbound(packet)
        }
        .unwrap(),
        Outbound::Denied(UnavailableReason::Unauthorized)
    ));
    assert!(matches!(
        {
            let packet = f.exit_udp(false, 41000);
            f.a.outbound(packet)
        }
        .unwrap(),
        Outbound::Denied(UnavailableReason::ExitUnavailable)
    ));
    // Flow keys name the stable (origin, target) grant.
    let flow = {
        let packet = f.mesh_udp(false, 42424);
        f.a.local_flow(&packet)
    }
    .unwrap()
    .unwrap();
    assert_eq!(flow.grant, f.mesh_target());
    assert_eq!(flow.path, f.path());
    assert_eq!(f.a.circuit_count(), 1);
    assert_eq!(
        f.sync_calls(),
        calls,
        "first packet made a Control round trip"
    );
    f.close().await;
}

#[tokio::test]
async fn missing_adjacency_registers_interest_and_queues_until_the_carrier_exists() {
    let mut f = NetmapMesh::start().await;
    let r = f.ids()[1];
    let carrier = f.ar.carrier.clone();
    f.a.set_carriers(BTreeMap::new()).unwrap();
    assert!(matches!(
        {
            let packet = f.mesh_udp(false, 42424);
            f.a.outbound(packet)
        }
        .unwrap(),
        Outbound::AwaitingAdjacency
    ));
    let interests = f.a.take_adjacency_interests();
    assert_eq!(interests.len(), 1);
    assert_eq!(interests[0].peer, r);
    assert!(f.a.take_adjacency_interests().is_empty());
    // Still no flow: waiting commits nothing.
    assert_eq!(f.a.flow_count(), 0);
    assert!(f.a.flush(32).is_empty());
    f.a.set_carriers(BTreeMap::from([(r, carrier)])).unwrap();
    let mut flushed = f.a.flush(32);
    let Ok(Flushed::Init(init)) = flushed.remove(0) else {
        panic!("handshake once the adjacency exists")
    };
    let id = f.finish_handshake(init).await;
    let mut flushed = f.a.flush(32);
    let Ok(Flushed::Send(job)) = flushed.remove(0) else {
        panic!("queued packet")
    };
    assert_eq!(job.circuit(), id);
    delivered(&mut f, sealed(job));
    // A carrier the netmap does not admit is refused.
    let mut forged = f.ar.carrier.clone();
    forged.listener_key = "other".into();
    assert!(matches!(
        f.a.set_carriers(BTreeMap::from([(r, forged)])),
        Err(Error::Policy)
    ));
    f.close().await;
}

#[tokio::test]
async fn same_content_apply_changes_nothing_and_keeps_outputs() {
    let mut f = NetmapMesh::start().await;
    let id = {
        let packet = f.mesh_udp(false, 42424);
        first(&mut f, packet)
    }
    .await;
    let queued = {
        let packet = f.mesh_udp(false, 42424);
        send(&mut f, packet)
    };
    let before = f.a.circuit_info(id).unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(f.renew().await, [Applied::default(); 3]);
    // Nothing is scanned or retired: the queued output remains valid.
    delivered(&mut f, queued);
    let after = f.a.circuit_info(id).unwrap().unwrap();
    assert_eq!(after.state, CircuitState::Established);
    // Monotonic deadlines: the UTC projection rounds and may step back 1 ms.
    assert!(
        after.expires_at >= before.expires_at,
        "activity, never a lease, bounds a circuit"
    );
    assert_eq!(f.a.flow_count(), 1);
    {
        let packet = f.mesh_udp(false, 42424);
        let frame = send(&mut f, packet);
        delivered(&mut f, frame)
    };
    f.close().await;
}

#[tokio::test]
async fn port_revocation_ends_only_that_flow_and_keeps_the_circuit() {
    let mut f = NetmapMesh::start().await;
    let id = {
        let packet = f.mesh_udp(false, 42424);
        first(&mut f, packet)
    }
    .await;
    {
        let packet = icmp(&f, false, 1);
        let frame = send(&mut f, packet);
        delivered(&mut f, frame)
    };
    assert_eq!(f.a.flow_count(), 2);
    let updates = f
        .publish(|cfg, [a, _, b]| {
            cfg.access[0] = rule(a, &sel(b), &["udp:42424", "tcp:443"]);
        })
        .await;
    for update in [updates[0], updates[2]] {
        assert_eq!(
            update,
            Applied {
                retired_circuits: 0,
                retired_flows: 1
            }
        );
    }
    assert_eq!(f.a.flow_count(), 1);
    // The surviving flow keeps its cipher and nonce windows.
    let frame = {
        let packet = f.mesh_udp(false, 42424);
        send(&mut f, packet)
    };
    assert_eq!(frame.circuit(), id);
    delivered(&mut f, frame);
    assert!(matches!(
        {
            let packet = icmp(&f, false, 2);
            f.a.outbound(packet)
        }
        .unwrap(),
        Outbound::Denied(_)
    ));
    // B judges with its own new content even if A were not updated.
    assert!(
        {
            let packet = icmp(&f, true, 1);
            reply(&mut f, id, packet)
        }
        .is_err()
    );
    f.close().await;
}

#[tokio::test]
async fn path_change_ends_the_circuit_at_every_node() {
    let mut f = NetmapMesh::start().await;
    let id = {
        let packet = f.mesh_udp(false, 42424);
        first(&mut f, packet)
    }
    .await;
    let queued = {
        let packet = f.mesh_udp(false, 42424);
        send(&mut f, packet)
    };
    // A different dial candidate on R changes the path authorization digest.
    let updates = f
        .publish(|cfg, [_, r, _]| {
            let listener = cfg.nodes[&r].listeners["tcp"].clone();
            cfg.nodes
                .get_mut(&r)
                .unwrap()
                .listeners
                .insert("tcp2".into(), listener);
        })
        .await;
    for update in updates {
        assert!(
            matches!(
                update,
                Applied {
                    retired_circuits: 1,
                    ..
                }
            ),
            "{update:?}"
        );
    }
    assert!(queued.release(|_| panic!("old output")).is_err());
    for node in [&f.a, &f.r, &f.b] {
        assert!(node.circuit_info(id).unwrap().is_none());
        assert_eq!(node.dead_circuits(), 1);
    }
    f.close().await;
}

#[tokio::test]
async fn own_identity_change_terminates_everything() {
    let mut f = NetmapMesh::start().await;
    {
        let packet = f.mesh_udp(false, 42424);
        first(&mut f, packet)
    }
    .await;
    let updates = f
        .publish(|cfg, [a, _, _]| {
            cfg.nodes.get_mut(&a).unwrap().addresses.ipv4 = Some("100.64.0.9".parse().unwrap());
        })
        .await;
    assert_eq!(
        updates[0],
        Applied {
            retired_circuits: 1,
            retired_flows: 1,
        }
    );
    assert_eq!(f.a.circuit_count(), 0);
    assert_eq!(f.a.flow_count(), 0);
    // A's address is in every path digest: peers retire the circuit too.
    assert!(matches!(
        updates[1],
        Applied {
            retired_circuits: 1,
            ..
        }
    ));
    f.close().await;
}

#[tokio::test]
async fn withdrawal_drops_every_circuit_flow_and_queued_output() {
    let mut f = NetmapMesh::start().await;
    {
        let packet = f.mesh_udp(false, 42424);
        first(&mut f, packet)
    }
    .await;
    let queued = {
        let packet = f.mesh_udp(false, 42424);
        send(&mut f, packet)
    };
    // Engine's empty apply drops the packet owner; no lease is read here.
    let (desired, keys) = f.content(0);
    let r = f.ids()[1];
    let fresh = meshway_dataplane::secure::SecureNode::new(
        &desired,
        keys,
        Default::default(),
        BTreeMap::from([(r, f.ar.carrier.clone())]),
    )
    .unwrap();
    drop(std::mem::replace(&mut f.a, fresh));
    assert!(queued.release(|_| panic!("withdrawn output")).is_err());
    assert_eq!(f.a.flow_count(), 0);
    f.close().await;
}

#[tokio::test]
async fn origin_without_its_next_hop_registers_interest_and_holds_the_packet() {
    let mut f = NetmapMesh::start().await;
    let id = {
        let packet = f.mesh_udp(false, 42424);
        first(&mut f, packet)
    }
    .await;
    let [_, r, _] = f.ids();
    // The A–R adjacency idled out or failed while the circuit lives.
    f.a.set_carriers(BTreeMap::new()).unwrap();
    let packet = f.mesh_udp(false, 42424);
    assert!(matches!(
        f.a.outbound(packet).unwrap(),
        Outbound::AwaitingAdjacency
    ));
    assert_eq!(
        f.a.take_adjacency_interests(),
        vec![meshway_dataplane::secure::AdjacencyInterest { peer: r }]
    );
    assert!(f.a.flush(32).is_empty(), "held until the carrier is back");
    f.a.set_carriers(BTreeMap::from([(r, f.ar.carrier.clone())]))
        .unwrap();
    let mut flushed = f.a.flush(32);
    let Ok(Flushed::Send(job)) = flushed.remove(0) else {
        panic!("held packet")
    };
    let frame = sealed(job);
    assert_eq!(frame.circuit(), id);
    delivered(&mut f, frame);
    f.close().await;
}

#[tokio::test]
async fn relay_admits_transit_only_on_its_own_content() {
    let mut f = NetmapMesh::start().await;
    // R learns that A has no Access to B; A and B still hold the old content.
    f.recompile(|cfg, _| cfg.access.clear());
    assert!(matches!(f.install_on(1, true).await, Applied { .. }));
    let Outbound::Queued { init: Some(init) } = {
        let packet = f.mesh_udp(false, 42424);
        f.a.outbound(packet)
    }
    .unwrap() else {
        panic!("A still decides locally")
    };
    assert!(matches!(
        f.r.receive(&f.ar.carrier, wire(init)),
        Err(Error::Policy)
    ));
    assert_eq!(f.r.circuit_count(), 0);
    // A grant the relay was never part of cannot be opened through it either.
    let [a, r, b] = f.ids();
    assert!(
        f.b.start_grant(
            CircuitGrant {
                purpose: meshway_dataplane::Purpose::Business,
                target: meshway_dataplane::secure::Target::Mesh { node: a },
            },
            vec![b, r, a],
            &f.rb.carrier.clone(),
        )
        .is_err()
    );
    f.close().await;
}

#[tokio::test]
async fn dead_circuit_record_is_reclaimed_after_its_init_window() {
    let mut f = NetmapMesh::start().await;
    let id = {
        let packet = f.mesh_udp(false, 42424);
        first(&mut f, packet)
    }
    .await;
    f.r.close(id);
    assert_eq!(f.r.circuit_count(), 0);
    assert_eq!(f.r.dead_circuits(), 1);
    // Closing does not count against circuit capacity and is never revived.
    let frame = {
        let packet = f.mesh_udp(false, 42424);
        send(&mut f, packet)
    };
    assert!(f.r.receive(&f.ar.carrier, wire(frame)).is_err());
    tokio::time::sleep(Duration::from_secs(14) + Duration::from_millis(200)).await;
    f.r.maintain().unwrap();
    assert_eq!(f.r.dead_circuits(), 0);
    f.close().await;
}

#[tokio::test]
async fn relay_without_next_hop_registers_interest_and_admits_the_retried_init() {
    let mut f = NetmapMesh::start().await;
    let [a, _, b] = f.ids();
    let (ar, rb) = (f.ar.carrier.clone(), f.rb.carrier.clone());
    f.r.set_carriers(BTreeMap::from([(a, ar.clone())])).unwrap();
    let packet = f.mesh_udp(false, 42424);
    let Outbound::Queued { init: Some(init) } = f.a.outbound(packet).unwrap() else {
        panic!("first packet")
    };
    let id = init.circuit();
    assert!(matches!(
        f.r.receive(&f.ar.carrier, wire(init)),
        Err(Error::Unsupported)
    ));
    let interests = f.r.take_adjacency_interests();
    assert_eq!(interests.len(), 1);
    assert_eq!(interests[0].peer, b);
    // Nothing was consumed: no circuit and no dead record for this CID.
    assert_eq!(f.r.circuit_count(), 0);
    assert_eq!(f.r.dead_circuits(), 0);
    f.r.set_carriers(BTreeMap::from([(a, ar), (b, rb)]))
        .unwrap();
    let due = f.a.handshake_retry_at(id).unwrap().unwrap();
    tokio::time::sleep_until(due.into()).await;
    let retry = f.a.retry_handshake(id).unwrap().unwrap();
    assert_eq!(f.finish_handshake(retry).await, id);
    let mut flushed = f.a.flush(32);
    let Ok(Flushed::Send(job)) = flushed.remove(0) else {
        panic!("queued packet")
    };
    delivered(&mut f, sealed(job));
    f.close().await;
}

#[tokio::test]
async fn tcp_flow_and_its_reply_stay_on_the_original_circuit_through_renewal() {
    let mut f = NetmapMesh::start().await;
    let (a, b) = (f.address(0), f.address(2));
    let syn = super::packet::tcp(a, b, (50000, 443), 1, 0, 0x02);
    let id = first(&mut f, syn).await;
    let Received::Packet(packet) = reply(
        &mut f,
        id,
        super::packet::tcp(b, a, (443, 50000), 7, 2, 0x12),
    )
    .unwrap() else {
        panic!("SYN-ACK on the original path")
    };
    packet.release(|_| ()).unwrap();
    // Same-content renewal continues the flow without a new handshake.
    assert!(
        f.renew()
            .await
            .iter()
            .all(|update| *update == Applied::default())
    );
    let data = super::packet::tcp_with_payload(a, b, (50000, 443), 2, 8, 0x18, b"data");
    let frame = send(&mut f, data);
    assert_eq!(frame.circuit(), id);
    delivered(&mut f, frame);
    // A port outside access never opens a circuit.
    assert!(matches!(
        f.a.outbound(super::packet::tcp(a, b, (50001, 444), 1, 0, 0x02))
            .unwrap(),
        Outbound::Denied(UnavailableReason::Unauthorized)
    ));
    f.close().await;
}
