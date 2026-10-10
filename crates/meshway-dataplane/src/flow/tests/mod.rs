mod collection;
mod configuration;
mod local;
mod tcp_recovery;

use super::*;
use crate::{Action, AuthenticatedPeer, Ingress, Packet, fixtures::*};
use configuration::{Scene, node};
use std::time::Duration;

fn unpack(action: Action, next: Option<u8>, direction: Direction, path: &[u8]) -> Packet {
    let (actual, packet, route) = match action {
        Action::Forward {
            next,
            packet,
            route,
        } => (Some(next), packet, route),
        Action::Deliver { packet, route } => (None, packet, route),
        Action::Exit { .. } => panic!("unexpected exit"),
    };
    assert_eq!(actual, next.map(node));
    assert_eq!(route.direction, direction);
    assert_eq!(route.origin, node(1));
    assert_eq!(
        route.grant,
        meshway_policy::compile::netmap::Target::Mesh { node: node(3) }
    );
    assert_eq!(
        route.original_path,
        path.iter().map(|i| node(*i)).collect::<Vec<_>>()
    );
    packet
}

/// Business flows are admitted at the two endpoints only; the relay
/// forwards the circuit's session without a flow entry.
fn forward(scene: &mut Scene, packet: Packet) {
    let packet = unpack(
        scene.process(1, None, 1, &[1, 2, 3], packet).unwrap(),
        Some(2),
        Direction::Forward,
        &[1, 2, 3],
    );
    unpack(
        scene.process(3, Some(2), 1, &[1, 2, 3], packet).unwrap(),
        None,
        Direction::Forward,
        &[1, 2, 3],
    );
}

fn reply(scene: &mut Scene, packet: Packet) {
    let packet = unpack(
        scene.process(3, None, 3, &[1, 2, 3], packet).unwrap(),
        Some(2),
        Direction::Reply,
        &[1, 2, 3],
    );
    unpack(
        scene.process(1, Some(2), 3, &[1, 2, 3], packet).unwrap(),
        None,
        Direction::Reply,
        &[1, 2, 3],
    );
}

#[test]
fn compiled_three_node_udp_preserves_full_path_in_both_directions() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
    forward(&mut scene, udp(1, 3, 50000, 443));
    reply(&mut scene, udp(3, 1, 443, 50000));
    for table in [&scene.tables[&node(1)], &scene.tables[&node(3)]] {
        assert_eq!(table.len(), 1);
    }
    // Arrival peer remains the adjacent relay; endpoint identity stays node 3.
    assert!(matches!(
        scene.process(1, Some(3), 3, &[1, 2, 3], udp(3, 1, 443, 50000)),
        Err(Error::Admission(_))
    ));
    assert!(matches!(
        scene.process(1, Some(2), 2, &[1, 2, 3], udp(3, 1, 443, 50000)),
        Err(Error::Sender)
    ));
}

#[test]
fn compiled_three_node_ipv6_uses_the_same_parser_policy_and_flow_owner() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
    let packet = Packet::parse(datagram(
        "fd00::1".parse().unwrap(),
        "fd00::3".parse().unwrap(),
        17,
        udp_body(50000, 443, b"IPv6 request"),
    ))
    .unwrap();
    forward(&mut scene, packet);
    let packet = Packet::parse(datagram(
        "fd00::3".parse().unwrap(),
        "fd00::1".parse().unwrap(),
        17,
        udp_body(443, 50000, b"IPv6 reply"),
    ))
    .unwrap();
    reply(&mut scene, packet);
    for table in [&scene.tables[&node(1)], &scene.tables[&node(3)]] {
        assert_eq!(table.len(), 1);
    }
}

#[test]
fn forged_forward_address_port_protocol_and_hop_cannot_create_entries() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
    for packet in [
        udp(2, 3, 50000, 443),
        udp(1, 2, 50000, 443),
        udp(1, 3, 50000, 444),
        tcp(false, 100, 0, 2, 0),
    ] {
        assert!(scene.process(3, Some(2), 1, &[1, 2, 3], packet).is_err());
        assert_eq!(scene.tables[&node(3)].len(), 0);
    }
    assert!(matches!(
        scene.process(3, Some(1), 1, &[1, 2, 3], udp(1, 3, 50000, 443)),
        Err(Error::Admission(_))
    ));
    assert!(matches!(
        scene.process(3, Some(1), 1, &[1, 3], udp(1, 3, 50000, 443)),
        Err(Error::Admission(_))
    ));
    assert!(matches!(
        scene.process(3, Some(2), 2, &[1, 2, 3], udp(1, 3, 50000, 443)),
        Err(Error::Sender)
    ));
    let ingress = Ingress::Peer {
        target: scene.target.clone(),
        path: vec![node(1), node(2), node(3)],
        identity: AuthenticatedPeer {
            origin: node(2),
            sender: node(1),
            previous: node(2),
        },
    };
    assert!(matches!(
        scene.admit(3, ingress, udp(1, 3, 50000, 443)),
        Err(Error::Admission(_))
    ));
    assert_eq!(scene.tables[&node(3)].len(), 0);
}

#[test]
fn unsolicited_and_forged_reverse_tuples_do_not_qualify_as_replies() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
    assert!(
        scene
            .process(1, Some(2), 3, &[1, 2, 3], udp(3, 1, 443, 50000))
            .is_err()
    );
    assert_eq!(scene.tables[&node(1)].len(), 0);
    forward(&mut scene, udp(1, 3, 50000, 443));
    for packet in [
        udp(3, 1, 444, 50000),
        udp(3, 1, 443, 50001),
        udp(2, 1, 443, 50000),
        udp(3, 2, 443, 50000),
    ] {
        assert!(scene.process(1, Some(2), 3, &[1, 2, 3], packet).is_err());
        assert_eq!(scene.tables[&node(1)].len(), 1);
    }
    reply(&mut scene, udp(3, 1, 443, 50000));
}

#[test]
fn echo_identifier_sequence_and_message_role_are_stateful() {
    let mut scene = Scene::new(BusinessProtocol::IcmpEcho, false, true);
    assert!(
        scene
            .process(1, Some(2), 3, &[1, 2, 3], echo(true, 12, 34))
            .is_err()
    );
    forward(&mut scene, echo(false, 12, 34));
    for packet in [echo(true, 13, 34), echo(true, 12, 35)] {
        assert!(scene.process(1, Some(2), 3, &[1, 2, 3], packet).is_err());
    }
    let request_body = vec![8, 0, 0, 0, 0, 12, 0, 34];
    let backwards_request =
        Packet::parse(datagram(address(3), address(1), 1, request_body)).unwrap();
    assert!(matches!(
        scene.process(1, Some(2), 3, &[1, 2, 3], backwards_request),
        Err(Error::Flow)
    ));
    reply(&mut scene, echo(true, 12, 34));
    assert_eq!(scene.tables[&node(1)].len(), 1);
}

#[test]
fn tcp_requires_handshake_and_observed_peer_acknowledgement() {
    let mut scene = Scene::new(BusinessProtocol::Tcp, false, true);
    for packet in [tcp(true, 900, 101, 0x12, 0), tcp(false, 101, 901, 0x10, 0)] {
        assert!(scene.process(1, None, 1, &[1, 2, 3], packet).is_err());
        assert_eq!(scene.tables[&node(1)].len(), 0);
    }
    forward(&mut scene, tcp(false, 100, 0, 2, 0));
    // Rejected SYN/ACK and reverse SYN do not mutate the pending handshake.
    for packet in [
        tcp(true, 900, 102, 0x12, 0),
        tcp(true, 900, 0, 2, 0),
        tcp(true, 900, 101, 0x10, 1),
    ] {
        assert!(matches!(
            scene.process(3, None, 3, &[1, 2, 3], packet),
            Err(Error::Flow)
        ));
    }
    reply(&mut scene, tcp(true, 900, 101, 0x12, 0));
    reply(&mut scene, tcp(true, 900, 101, 0x12, 0)); // SYN/ACK retransmit
    assert!(matches!(
        scene.process(1, None, 1, &[1, 2, 3], tcp(false, 101, 902, 0x10, 0)),
        Err(Error::Flow)
    ));
    forward(&mut scene, tcp(false, 101, 901, 0x10, 0));
    forward(&mut scene, tcp(false, 101, 901, 0x18, 7));
    forward(&mut scene, tcp(false, 101, 901, 0x18, 7)); // latest segment retransmit
    assert!(matches!(
        scene.process(3, None, 3, &[1, 2, 3], tcp(true, 902, 109, 0x18, 5)),
        Err(Error::Flow)
    ));
    reply(&mut scene, tcp(true, 901, 108, 0x18, 5));
    assert!(matches!(
        scene.process(3, None, 3, &[1, 2, 3], tcp(true, 1234, 0, 2, 0)),
        Err(Error::Flow)
    ));
    forward(&mut scene, tcp(false, 108, 906, 0x14, 0)); // established reset
    for table in [&scene.tables[&node(1)], &scene.tables[&node(3)]] {
        assert_eq!(table.len(), 0);
    }
    assert!(
        scene
            .process(1, Some(2), 3, &[1, 2, 3], tcp(true, 906, 108, 0x10, 0))
            .is_err()
    );
}

#[test]
fn tcp_sequence_wrap_and_half_close_remain_bounded() {
    let mut scene = Scene::new(BusinessProtocol::Tcp, false, true);
    forward(&mut scene, tcp(false, u32::MAX, 0, 2, 0));
    reply(&mut scene, tcp(true, u32::MAX, 0, 0x12, 0));
    forward(&mut scene, tcp(false, 0, 0, 0x10, 0));
    forward(&mut scene, tcp(false, 0, 0, 0x11, 0));
    reply(&mut scene, tcp(true, 0, 1, 0x10, 0));
    assert!(matches!(
        scene.process(1, None, 1, &[1, 2, 3], tcp(false, 1, 0, 0x18, 1)),
        Err(Error::Flow)
    ));
    reply(&mut scene, tcp(true, 0, 1, 0x11, 0));
    forward(&mut scene, tcp(false, 1, 1, 0x10, 0));
    // Keep the closed tuple as a bounded tombstone until idle expiration.
    assert!(matches!(
        scene.process(1, None, 1, &[1, 2, 3], tcp(false, 50, 0, 2, 0)),
        Err(Error::Flow)
    ));
}

#[test]
fn signed_facts_and_parsed_packets_share_protocol_state_and_rejections() {
    for (protocol, packets) in [
        (
            BusinessProtocol::Udp,
            vec![
                (false, udp(1, 3, 50000, 443)),
                (true, udp(3, 1, 444, 50000)),
                (true, udp(3, 1, 443, 50000)),
            ],
        ),
        (
            BusinessProtocol::IcmpEcho,
            vec![
                (false, echo(false, 12, 34)),
                (true, echo(true, 12, 35)),
                (true, echo(true, 12, 34)),
            ],
        ),
        (
            BusinessProtocol::Tcp,
            vec![
                (false, tcp(false, 100, 0, 2, 0)),
                (true, tcp(true, 900, 102, 0x12, 0)),
                (true, tcp(true, 900, 101, 0x12, 0)),
                (false, tcp(false, 101, 901, 0x10, 0)),
                (false, tcp(false, 101, 901, 0x18, 7)),
                (true, tcp(true, 901, 108, 0x18, 5)),
            ],
        ),
    ] {
        let mut plaintext = Scene::new(protocol, false, true);
        let mut encrypted = Scene::new(protocol, false, true);
        for (reply, packet) in packets {
            // The terminal admits forward packets from the relay and its own
            // replies locally.
            let facts = packet.facts();
            let path = vec![node(1), node(2), node(3)];
            let (packet_result, ingress) = if reply {
                (
                    plaintext.process(3, None, 3, &[1, 2, 3], packet),
                    Ingress::Local {
                        target: plaintext.target.clone(),
                        path,
                    },
                )
            } else {
                (
                    plaintext.process(3, Some(2), 1, &[1, 2, 3], packet),
                    Ingress::Peer {
                        target: plaintext.target.clone(),
                        path,
                        identity: AuthenticatedPeer {
                            origin: node(1),
                            sender: node(1),
                            previous: node(2),
                        },
                    },
                )
            };
            let facts_result = encrypted.admit_facts(3, ingress, facts);
            match (packet_result, facts_result) {
                (Ok(Action::Forward { next, route, .. }), Ok(checked)) => {
                    assert_eq!(checked.hop, Hop::Forward(next));
                    assert_eq!(checked.route.direction, route.direction);
                    assert_eq!(checked.route.original_path, route.original_path);
                }
                (Ok(Action::Deliver { route, .. }), Ok(checked)) => {
                    assert_eq!(checked.hop, Hop::Deliver);
                    assert_eq!(checked.route.direction, route.direction);
                    assert_eq!(checked.route.original_path, route.original_path);
                }
                (Err(packet), Err(facts)) => assert_eq!(
                    std::mem::discriminant(&packet),
                    std::mem::discriminant(&facts)
                ),
                result => panic!("packet/facts admission diverged: {result:?}"),
            }
            assert_eq!(
                plaintext.tables[&node(3)].len(),
                encrypted.tables[&node(3)].len()
            );
        }
    }
}

#[test]
fn capacity_rejection_is_transactional_and_flows_end_with_idle_expiry() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
    scene.tables.get_mut(&node(1)).unwrap().limits.max_flows = 1;
    scene.tables.get_mut(&node(1)).unwrap().limits.idle_timeout = Duration::from_secs(1);
    scene
        .process(1, None, 1, &[1, 2, 3], udp(1, 3, 50000, 443))
        .unwrap();
    let before = scene.tables[&node(1)]
        .entries
        .values()
        .next()
        .unwrap()
        .cap
        .at;
    scene.advance(Duration::from_millis(900));
    assert!(matches!(
        scene.process(1, None, 1, &[1, 2, 3], udp(1, 3, 50001, 443)),
        Err(Error::Capacity)
    ));
    assert_eq!(
        scene.tables[&node(1)]
            .entries
            .values()
            .next()
            .unwrap()
            .cap
            .at,
        before
    );
    scene.advance(Duration::from_millis(100));
    assert!(
        scene
            .process(1, Some(2), 3, &[1, 2, 3], udp(3, 1, 443, 50000))
            .is_err()
    );
    assert_eq!(scene.tables[&node(1)].len(), 0);
    scene
        .process(1, None, 1, &[1, 2, 3], udp(1, 3, 50001, 443))
        .unwrap();
}
