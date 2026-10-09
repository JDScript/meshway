//! Differential admission: one configuration (A may reach B on udp:42424
//! through R) executed by the legacy per-demand path and by the netmap path,
//! with the same packet sequence on real SecureNodes A→R→B. Every forward,
//! reply and terminal receipt must reach the same admission conclusion.
use super::support::{
    netmap::{NetmapMesh, rule, sel},
    packet,
    secure::SecureMesh,
};
use meshway_dataplane::{
    Packet,
    secure::{PendingFrame, Received, SecureNode},
};
use std::net::IpAddr;

/// One step: the producer's send result and, if sent, the receiver's.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    SendRejected,
    ReceiveRejected,
    Delivered,
}

struct Nodes<'a> {
    a: &'a mut SecureNode,
    r: &'a mut SecureNode,
    b: &'a mut SecureNode,
    ar: &'a meshway_dataplane::secure::Carrier,
    rb: &'a meshway_dataplane::secure::Carrier,
}

fn wire(frame: PendingFrame) -> Vec<u8> {
    frame.release(|bytes| bytes.into_vec()).unwrap()
}

fn step(nodes: &mut Nodes<'_>, id: [u8; 32], reply: bool, packet: Packet) -> Outcome {
    let (from, to, first, second) = if reply {
        (&mut *nodes.b, &mut *nodes.a, nodes.rb, nodes.ar)
    } else {
        (&mut *nodes.a, &mut *nodes.b, nodes.ar, nodes.rb)
    };
    let Ok(frame) = from.send(id, packet) else {
        return Outcome::SendRejected;
    };
    let Received::Forward(next) = nodes.r.receive(first, wire(frame)).unwrap() else {
        panic!("relay stays opaque")
    };
    match to.receive(second, wire(next)) {
        Ok(Received::Packet(packet)) => {
            packet.release(|_| ()).unwrap();
            Outcome::Delivered
        }
        _ => Outcome::ReceiveRejected,
    }
}

/// The shared packet sequence over Mesh addresses `a` and `b`.
fn sequence(a: IpAddr, b: IpAddr) -> Vec<(bool, Packet)> {
    vec![
        (false, packet::udp(a, b, 50000, 42424, b"granted forward")),
        (true, packet::udp(b, a, 42424, 50000, b"exact reply")),
        (false, packet::udp(a, b, 50000, 42425, b"other port")),
        (
            true,
            packet::udp(b, a, 42424, 50001, b"unsolicited reverse"),
        ),
        (false, packet::echo(a, b, false, 9, 1, b"no icmp grant")),
        (false, packet::tcp(a, b, (40000, 443), 100, 0, 2)),
        (
            false,
            packet::udp(
                a,
                "100.64.0.77".parse().unwrap(),
                50000,
                42424,
                b"unassigned",
            ),
        ),
        (true, packet::udp(b, a, 42425, 50000, b"wrong source port")),
        (false, packet::udp(a, b, 50000, 42424, b"granted again")),
        (true, packet::udp(b, a, 42424, 50000, b"exact reply again")),
    ]
}

fn run(nodes: &mut Nodes<'_>, id: [u8; 32], a: IpAddr, b: IpAddr) -> Vec<Outcome> {
    sequence(a, b)
        .into_iter()
        .map(|(reply, packet)| step(nodes, id, reply, packet))
        .collect()
}

fn address(node: &super::support::Node) -> IpAddr {
    let snapshot = node.control.execution_snapshot().unwrap();
    snapshot
        .configuration()
        .unwrap()
        .addresses
        .ipv4
        .unwrap()
        .into()
}

#[tokio::test]
async fn legacy_and_netmap_paths_reach_the_same_admission_conclusions() {
    let mut legacy = SecureMesh::start().await;
    let id = legacy.handshake().await;
    let (a, b) = (address(&legacy.mesh.a), address(&legacy.mesh.b));
    let SecureMesh {
        a: la,
        r: lr,
        b: lb,
        ar,
        rb,
        ..
    } = &mut legacy;
    let expected = run(
        &mut Nodes {
            a: la,
            r: lr,
            b: lb,
            ar: &ar.carrier,
            rb: &rb.carrier,
        },
        id,
        a,
        b,
    );
    legacy.close().await;
    assert_eq!(expected[0], Outcome::Delivered);
    assert_eq!(expected[1], Outcome::Delivered);
    assert!(
        expected[2..8]
            .iter()
            .all(|outcome| *outcome != Outcome::Delivered)
    );

    let mut netmap = NetmapMesh::with(super::support::netmap::DAY_MS, |cfg, [a, _, b]| {
        cfg.access = vec![rule(a, &sel(b), &["udp:42424"])];
        cfg.routing.egress.assignments.clear();
        cfg.routing.egress.profiles.clear();
    })
    .await;
    let target = netmap.mesh_target();
    let id = netmap.handshake(target).await;
    let (a, b): (IpAddr, IpAddr) = (netmap.address(0), netmap.address(2));
    let NetmapMesh {
        a: na,
        r: nr,
        b: nb,
        ar,
        rb,
        ..
    } = &mut netmap;
    let actual = run(
        &mut Nodes {
            a: na,
            r: nr,
            b: nb,
            ar: &ar.carrier,
            rb: &rb.carrier,
        },
        id,
        a,
        b,
    );
    netmap.close().await;
    assert_eq!(actual, expected);
}
