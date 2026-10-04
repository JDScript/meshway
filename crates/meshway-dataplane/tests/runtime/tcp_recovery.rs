//! Registered Control keys, real Server renewals, production Noise/flow owners
//! and native TCP carrier sessions. Deterministic losses occur at guarded output
//! boundaries, independently of kernel scheduling or benchmark thresholds.
use super::{
    release,
    support::{Node, request, secure::SecureMesh},
};
use meshway_configuration::execution::{BusinessProtocol, BusinessTarget, DemandDecision};
use meshway_dataplane::{
    Direction, Hop, Packet, PendingAction,
    secure::{PendingFrame, Received},
};
use meshway_protocol::{DemandRequest, RenewDemandRequest};

async fn reconcile(fixture: &mut SecureMesh) {
    for (node, secure) in [
        (&mut fixture.mesh.a, &mut fixture.a),
        (&mut fixture.mesh.r, &mut fixture.r),
        (&mut fixture.mesh.b, &mut fixture.b),
    ] {
        let snapshot = node.sync_current().await;
        secure
            .reconcile(node.control.dataplane_identity(snapshot).unwrap())
            .unwrap();
    }
}

async fn renew(fixture: &mut SecureMesh) {
    let lease = fixture.mesh.a.control.renew_instance().await.unwrap();
    fixture
        .mesh
        .a
        .control
        .renew_demand(&RenewDemandRequest {
            instance: fixture.mesh.a.instance(),
            request_id: fixture.mesh.demand,
            expires_at_ms: lease.expires_at_ms,
        })
        .await
        .unwrap();
    reconcile(fixture).await;
}

async fn to_relay(
    fixture: &mut SecureMesh,
    circuit: [u8; 32],
    reply: bool,
    packet: Packet,
) -> PendingFrame {
    let received = if reply {
        let pending = fixture.b.send(circuit, packet).unwrap();
        fixture.rb.listen.send(pending).await;
        let bytes = fixture.rb.dial.recv().await;
        fixture.r.receive(&fixture.rb.carrier, bytes).unwrap()
    } else {
        let pending = fixture.a.send(circuit, packet).unwrap();
        fixture.ar.dial.send(pending).await;
        let bytes = fixture.ar.listen.recv().await;
        fixture.r.receive(&fixture.ar.carrier, bytes).unwrap()
    };
    let Received::Forward(pending) = received else {
        panic!("opaque relay");
    };
    pending
}

async fn to_endpoint(
    fixture: &mut SecureMesh,
    reply: bool,
    pending: PendingFrame,
) -> PendingAction {
    let received = if reply {
        fixture.ar.listen.send(pending).await;
        let bytes = fixture.ar.dial.recv().await;
        fixture.a.receive(&fixture.ar.carrier, bytes).unwrap()
    } else {
        fixture.rb.dial.send(pending).await;
        let bytes = fixture.rb.listen.recv().await;
        fixture.b.receive(&fixture.rb.carrier, bytes).unwrap()
    };
    let Received::Packet(packet) = received else {
        panic!("endpoint business packet");
    };
    packet
}

async fn exchange(fixture: &mut SecureMesh, circuit: [u8; 32], reply: bool, packet: Packet) {
    let expected = packet.as_bytes().to_vec();
    let pending = to_relay(fixture, circuit, reply, packet).await;
    let received = to_endpoint(fixture, reply, pending).await;
    let direction = if reply {
        Direction::Reply
    } else {
        Direction::Forward
    };
    assert_eq!(
        release(&fixture.mesh, received, Hop::Deliver, direction).into_bytes(),
        expected
    );
}

// Wire fixture construction only: no alternate TCP parser, state or admission.
fn packet(
    fixture: &SecureMesh,
    reply: bool,
    seq: u32,
    ack: u32,
    flags: u8,
    payload: &[u8],
) -> Packet {
    fn checksum(bytes: &[u8]) -> u16 {
        let sum = bytes.chunks(2).fold(0u64, |sum, word| {
            sum + (u64::from(word[0]) << 8) + u64::from(*word.get(1).unwrap_or(&0))
        });
        let folded = (sum & 65535) + (sum >> 16);
        !(((folded & 65535) + (folded >> 16)) as u16)
    }
    let address = |node: &Node| {
        node.control
            .execution_snapshot()
            .unwrap()
            .configuration()
            .unwrap()
            .addresses
            .ipv4
            .unwrap()
    };
    let (source, destination, sport, dport) = if reply {
        (
            address(&fixture.mesh.b),
            address(&fixture.mesh.a),
            42424u16,
            50000u16,
        )
    } else {
        (
            address(&fixture.mesh.a),
            address(&fixture.mesh.b),
            50000u16,
            42424u16,
        )
    };
    let mut tcp = vec![0; 20];
    tcp[..2].copy_from_slice(&sport.to_be_bytes());
    tcp[2..4].copy_from_slice(&dport.to_be_bytes());
    tcp[4..8].copy_from_slice(&seq.to_be_bytes());
    tcp[8..12].copy_from_slice(&ack.to_be_bytes());
    tcp[12] = 0x50;
    tcp[13] = flags;
    tcp[14..16].copy_from_slice(&4096u16.to_be_bytes());
    tcp.extend_from_slice(payload);
    let mut pseudo = source.octets().to_vec();
    pseudo.extend(destination.octets());
    pseudo.extend([0, 6]);
    pseudo.extend(u16::try_from(tcp.len()).unwrap().to_be_bytes());
    pseudo.extend(&tcp);
    tcp[16..18].copy_from_slice(&checksum(&pseudo).to_be_bytes());
    let mut ip = vec![0; 20];
    ip[0] = 0x45;
    ip[2..4].copy_from_slice(&u16::try_from(20 + tcp.len()).unwrap().to_be_bytes());
    ip[8] = 64;
    ip[9] = 6;
    ip[12..16].copy_from_slice(&source.octets());
    ip[16..20].copy_from_slice(&destination.octets());
    let check = checksum(&ip);
    ip[10..12].copy_from_slice(&check.to_be_bytes());
    ip.extend(tcp);
    Packet::parse(ip).unwrap()
}

#[tokio::test]
async fn tcp_recovers_cancelled_relay_and_endpoint_outputs_across_real_permit_renewals() {
    let mut fixture = SecureMesh::start().await;
    fixture
        .mesh
        .server
        .configure(&fixture.mesh.owner, |input| {
            input.access[0].ip.push("tcp:42424".into());
        })
        .await;
    let demand = request();
    let decision = fixture
        .mesh
        .a
        .control
        .create_demand(&DemandRequest {
            instance: fixture.mesh.a.instance(),
            request_id: demand,
            target: BusinessTarget::Mesh {
                node: fixture.mesh.b.id,
            },
            protocol: BusinessProtocol::Tcp,
            port: Some(42424.try_into().unwrap()),
            profile: "mesh".into(),
            duration_seconds: 120,
        })
        .await
        .unwrap();
    assert!(matches!(
        decision.decision,
        DemandDecision::Permitted { .. }
    ));
    fixture.mesh.demand = demand;
    reconcile(&mut fixture).await;
    let circuit = fixture.handshake().await;
    for (reply, seq, ack, flags) in [
        (false, 100, 0, 2),
        (true, 900, 101, 0x12),
        (false, 101, 901, 0x10),
    ] {
        let packet = packet(&fixture, reply, seq, ack, flags, &[]);
        exchange(&mut fixture, circuit, reply, packet).await;
    }
    let first = packet(&fixture, false, 101, 901, 0x18, b"abcdefgh");
    let queued_first = to_relay(&mut fixture, circuit, false, first).await;
    let second = packet(&fixture, false, 109, 901, 0x18, b"ijkl");
    let queued_second = to_relay(&mut fixture, circuit, false, second).await;
    renew(&mut fixture).await;
    assert!(
        queued_first
            .release(|_| panic!("cancelled ciphertext escaped"))
            .is_err()
    );
    assert!(
        queued_second
            .release(|_| panic!("cancelled ciphertext escaped"))
            .is_err()
    );
    let delayed = packet(&fixture, true, 901, 101, 0x10, &[]);
    exchange(&mut fixture, circuit, true, delayed).await;
    for (seq, bytes) in [(101, b"abcdef".as_slice()), (107, b"ghijkl".as_slice())] {
        let retransmission = packet(&fixture, false, seq, 901, 0x18, bytes);
        exchange(&mut fixture, circuit, false, retransmission).await;
    }
    // Also lose plaintext after B's real decrypt/admission but before its sink.
    let data = packet(&fixture, false, 113, 901, 0x18, b"mnopqr");
    let queued = to_relay(&mut fixture, circuit, false, data).await;
    let undelivered = to_endpoint(&mut fixture, false, queued).await;
    renew(&mut fixture).await;
    assert!(
        undelivered
            .release(|_| panic!("cancelled plaintext escaped"))
            .is_err()
    );
    let delayed = packet(&fixture, true, 901, 113, 0x10, &[]);
    exchange(&mut fixture, circuit, true, delayed).await;
    let retransmission = packet(&fixture, false, 113, 901, 0x18, b"mnopqr");
    exchange(&mut fixture, circuit, false, retransmission).await;
    let echo = packet(&fixture, true, 901, 119, 0x18, b"recovered");
    exchange(&mut fixture, circuit, true, echo).await;
    for secure in [&fixture.a, &fixture.r, &fixture.b] {
        assert_eq!(secure.retained_flows(), 1);
        let ready = secure.ready_circuits().unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, circuit);
        assert_eq!(ready[0].demand, demand);
        assert_eq!(ready[0].path, fixture.mesh.path);
    }
    fixture.close().await;
}
