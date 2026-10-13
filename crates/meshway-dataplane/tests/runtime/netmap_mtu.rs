//! A local packet larger than the inner MTU of the channel to its next hop
//! is dropped before any nonce is used and reported on the sender's own TUN
//! as fragmentation needed, in either direction of the flow; the limit
//! follows the channel facts.
use super::netmap_flow::{delivered, first, reply, send};
use super::support::netmap::NetmapMesh;
use meshway_dataplane::{
    Action, Hop, Packet, Transport,
    secure::{Error, Outbound, Received},
};
use std::collections::BTreeMap;

fn reported(errors: Vec<meshway_dataplane::PendingAction>, about: &Packet, mtu: u16) {
    assert_eq!(errors.len(), 1, "{errors:?}");
    let error = errors.into_iter().next().unwrap();
    assert_eq!(error.hop(), Hop::Deliver);
    let packet = error
        .release(|action| match action {
            Action::Deliver { packet, .. } => packet,
            other => panic!("{other:?}"),
        })
        .unwrap();
    let Transport::Error(icmp) = packet.transport() else {
        panic!("{packet:?}")
    };
    assert_eq!((icmp.kind, icmp.code), (3, 4));
    assert_eq!(icmp.quoted.traffic, about.traffic());
    assert_eq!(packet.traffic().destination, about.traffic().source);
    let bytes = packet.as_bytes();
    assert_eq!(u16::from_be_bytes([bytes[26], bytes[27]]), mtu);
}

#[tokio::test]
async fn a_packet_beyond_its_next_hop_mtu_is_reported_locally_both_ways() {
    let mut f = NetmapMesh::start().await;
    let [_, r, _] = f.ids();
    let (a, b) = (f.address(0), f.address(2));
    let id = first(&mut f, super::packet::tcp(a, b, (50000, 443), 1, 0, 0x02)).await;
    let Received::Packet(packet) = reply(
        &mut f,
        id,
        super::packet::tcp(b, a, (443, 50000), 7, 2, 0x12),
    )
    .unwrap() else {
        panic!("SYN-ACK")
    };
    packet.release(|_| ()).unwrap();

    // A's channel to R carries 1,200-byte inner packets.
    f.a.set_path_limits(BTreeMap::from([(r, 1200)]));
    let big = |seq| super::packet::tcp_with_payload(a, b, (50000, 443), seq, 8, 0x18, &[0; 1200]);
    let oversized = big(2);
    let facts = oversized.facts();
    assert!(matches!(f.a.outbound(oversized), Err(Error::TooBig)));
    let quoted = super::packet::tcp_with_payload(a, b, (50000, 443), 2, 8, 0x18, &[0; 1200]);
    assert_eq!(quoted.facts(), facts);
    reported(f.a.take_local_errors(), &quoted, 1200);
    assert!(f.a.take_local_errors().is_empty());
    // A packet that fits continues on the same circuit.
    let small =
        super::packet::tcp_with_payload(a, b, (50000, 443), 2, 8, 0x18, &[0; 1200 - 40 - 1]);
    let frame = send(&mut f, small);
    assert_eq!(frame.circuit(), id);
    delivered(&mut f, frame);

    // B's replies on its channel to R: the error reaches B's own TUN.
    f.b.set_path_limits(BTreeMap::from([(r, 1100)]));
    let response = super::packet::tcp_with_payload(b, a, (443, 50000), 8, 1161, 0x18, &[0; 1100]);
    let copy = super::packet::tcp_with_payload(b, a, (443, 50000), 8, 1161, 0x18, &[0; 1100]);
    assert!(matches!(f.b.send(id, response), Err(Error::TooBig)));
    reported(f.b.take_local_errors(), &copy, 1100);

    // The channel changed (a relay without a known limit): no limit applies.
    f.a.set_path_limits(BTreeMap::new());
    let Outbound::Send(job) = f.a.outbound(big(1161)).unwrap() else {
        panic!("established output")
    };
    delivered(
        &mut f,
        meshway_dataplane::secure::pipeline::seal(job).unwrap(),
    );
    assert!(f.a.take_local_errors().is_empty());
    f.close().await;
}

#[tokio::test]
async fn the_mss_clamp_follows_the_channel_limits() {
    let mut f = NetmapMesh::start().await;
    let [_, r, b_id] = f.ids();
    let (r_address, b) = (f.address(1), f.address(2));
    assert_eq!(f.a.clamps(), Default::default());
    f.a.set_path_limits(BTreeMap::from([(r, 1300)]));
    let clamps = f.a.clamps();
    // R has A's channel; B is reached through a relay: the smallest limit.
    assert_eq!(clamps.peers.get(&r_address), Some(&1300));
    assert_eq!(clamps.peers.get(&b), Some(&1300));
    assert!(!clamps.peers.keys().any(|address| *address == f.address(0)));
    let _ = b_id;
    f.close().await;
}
