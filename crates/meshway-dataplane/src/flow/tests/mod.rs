mod collection;
mod configuration;
mod continuity;
mod local;
mod tcp_recovery;

#[test]
fn renewed_guard_cannot_revive_a_flow_expired_by_the_old_guard_wall_clock() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
    scene.guard_expires_at = scene.instant + Duration::from_secs(30);
    scene.guard_expires_at_ms = scene.now_ms + 30_000;
    scene
        .process(1, None, 1, &[1, 2, 3], udp(1, 3, 50000, 443))
        .unwrap();
    // Partial forward clock adjustment, followed by a valid same-scope renewal.
    // Monotonic time has not reached the original execution deadline.
    scene.now_ms += 25_000;
    scene.guard_expires_at_ms = scene.now_ms + 60_000;
    scene.guard_expires_at = scene.instant + Duration::from_secs(60);
    scene.advance(Duration::from_secs(6));
    assert!(
        scene
            .process(1, Some(2), 3, &[1, 2, 3], udp(3, 1, 443, 50000))
            .is_err()
    );
    assert_eq!(scene.tables[&node(1)].len(), 0);
}

use super::*;
use crate::{Action, AuthenticatedPeer, Ingress, Packet, fixtures::*};
use configuration::{Scene, node, request};
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
    assert_eq!(route.demand, request());
    assert_eq!(
        route.original_path,
        path.iter().map(|i| node(*i)).collect::<Vec<_>>()
    );
    packet
}

fn forward(scene: &mut Scene, packet: Packet) {
    let packet = unpack(
        scene.process(1, None, 1, &[1, 2, 3], packet).unwrap(),
        Some(2),
        Direction::Forward,
        &[1, 2, 3],
    );
    let packet = unpack(
        scene.process(2, Some(1), 1, &[1, 2, 3], packet).unwrap(),
        Some(3),
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
    let packet = unpack(
        scene.process(2, Some(3), 3, &[1, 2, 3], packet).unwrap(),
        Some(1),
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
            let (sender, previous) = if reply { (3, 3) } else { (1, 1) };
            let facts = packet.facts();
            let packet_result = plaintext.process(2, Some(previous), sender, &[1, 2, 3], packet);
            let facts_result = encrypted.admit_facts(
                2,
                Ingress::Peer {
                    demand: request(),
                    path: vec![node(1), node(2), node(3)],
                    identity: AuthenticatedPeer {
                        origin: node(1),
                        sender: node(sender),
                        previous: node(previous),
                    },
                },
                facts,
            );
            match (packet_result, facts_result) {
                (Ok(Action::Forward { next, route, .. }), Ok(checked)) => {
                    assert_eq!(checked.hop, Hop::Forward(next));
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
                plaintext.tables[&node(2)].len(),
                encrypted.tables[&node(2)].len()
            );
        }
    }
}

#[test]
fn handshake_admission_does_not_establish_reverse_business_flow() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
    assert_eq!(
        meshway_policy::admission::admit_handshake(
            &scene.configs[&node(2)],
            request(),
            &[node(1), node(2), node(3)],
            node(1),
            Some(node(3)),
            true,
            scene.now_ms,
        ),
        Ok(Hop::Forward(node(1)))
    );
    assert!(
        scene
            .process(2, Some(3), 3, &[1, 2, 3], udp(3, 1, 443, 50000))
            .is_err()
    );
    assert_eq!(scene.tables[&node(2)].len(), 0);
}

#[test]
fn compiled_three_node_udp_preserves_full_path_in_both_directions() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
    forward(&mut scene, udp(1, 3, 50000, 443));
    reply(&mut scene, udp(3, 1, 443, 50000));
    for table in scene.tables.values() {
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
    for table in scene.tables.values() {
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
        demand: request(),
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
    for table in scene.tables.values() {
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
fn capacity_rejection_is_transactional_and_idle_expiry_does_not_renew_permits() {
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
    scene.now_ms = 100_000;
    assert!(
        scene
            .process(1, None, 1, &[1, 2, 3], udp(1, 3, 50001, 443))
            .is_err()
    );
    assert_eq!(scene.tables[&node(1)].len(), 0);
    assert_eq!(
        scene.configs[&node(1)].permits[&request()].expires_at_ms,
        100_000
    );
}

#[test]
fn snapshot_bound_retention_and_explicit_revocation_remove_flow_qualification() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
    scene.guard_expires_at = scene.instant + Duration::from_millis(5);
    forward(&mut scene, udp(1, 3, 50000, 443));
    for table in scene.tables.values() {
        assert_eq!(
            table.entries.values().next().unwrap().cap.at,
            scene.guard_expires_at
        );
    }
    scene.advance(Duration::from_millis(5));
    for table in scene.tables.values_mut() {
        table.expire(scene.instant, scene.now_ms);
        assert_eq!(table.len(), 0);
    }
    scene.guard_expires_at = scene.instant + Duration::from_secs(60);
    forward(&mut scene, udp(1, 3, 50000, 443));
    for table in scene.tables.values_mut() {
        table.clear();
    }
    assert!(
        scene
            .process(1, Some(2), 3, &[1, 2, 3], udp(3, 1, 443, 50000))
            .is_err()
    );
}

#[test]
fn live_session_cannot_select_another_permitted_path() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, false);
    let paths = &scene.configs[&node(1)].permits[&request()].paths;
    assert!(
        paths
            .iter()
            .any(|p| p.nodes == vec![node(1), node(2), node(3)])
    );
    assert!(paths.iter().any(|p| p.nodes == vec![node(1), node(3)]));
    scene
        .process(1, None, 1, &[1, 2, 3], udp(1, 3, 50000, 443))
        .unwrap();
    assert!(matches!(
        scene.process(1, None, 1, &[1, 3], udp(1, 3, 50000, 443)),
        Err(Error::Flow)
    ));
    assert_eq!(scene.tables[&node(1)].len(), 1);
}

#[test]
fn renewed_guard_preserves_old_flow_deadlines_until_valid_packet_admission() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
    let old_deadline = scene.instant + Duration::from_secs(2);
    scene.guard_expires_at = old_deadline;
    for source_port in [50000, 50001] {
        scene
            .process(1, None, 1, &[1, 2, 3], udp(1, 3, source_port, 443))
            .unwrap();
    }
    scene.advance(Duration::from_secs(1));
    scene.guard_expires_at = scene.instant + Duration::from_secs(30);
    // Same successful-handoff collection as Engine::refresh, without minting
    // an opaque Control snapshot in private flow tests.
    scene
        .tables
        .get_mut(&node(1))
        .unwrap()
        .expire(scene.instant, scene.now_ms);
    assert_eq!(scene.tables[&node(1)].len(), 2);
    assert!(
        scene.tables[&node(1)]
            .entries
            .values()
            .all(|flow| flow.cap.at == old_deadline)
    );
    // A malformed association cannot turn the new guard into refreshed state.
    assert!(
        scene
            .process(1, Some(2), 3, &[1, 2, 3], udp(3, 1, 444, 50001))
            .is_err()
    );
    assert!(
        scene.tables[&node(1)]
            .entries
            .values()
            .all(|flow| flow.cap.at == old_deadline)
    );
    scene
        .process(1, None, 1, &[1, 2, 3], udp(1, 3, 50000, 443))
        .unwrap();
    assert_eq!(scene.tables[&node(1)].len(), 2);
    scene.advance(Duration::from_secs(1));
    unpack(
        scene
            .process(1, Some(2), 3, &[1, 2, 3], udp(3, 1, 443, 50000))
            .unwrap(),
        None,
        Direction::Reply,
        &[1, 2, 3],
    );
    // The idle flow still expires at its old deadline and cannot be revived
    // by a reverse packet, even though the replacement guard is current.
    assert!(
        scene
            .process(1, Some(2), 3, &[1, 2, 3], udp(3, 1, 443, 50001))
            .is_err()
    );
    assert_eq!(scene.tables[&node(1)].len(), 1);
    assert_eq!(
        scene.tables[&node(1)]
            .entries
            .values()
            .next()
            .unwrap()
            .cap
            .at,
        scene.guard_expires_at
    );
}

#[test]
fn renewed_guard_and_valid_packets_cannot_extend_the_original_permit() {
    let mut scene = Scene::new(BusinessProtocol::Udp, false, true);
    scene.now_ms = 99_000;
    scene.guard_expires_at = scene.instant + Duration::from_secs(5);
    let permit_deadline = scene.instant + Duration::from_secs(1);
    scene
        .process(1, None, 1, &[1, 2, 3], udp(1, 3, 50000, 443))
        .unwrap();
    assert_eq!(
        scene.tables[&node(1)]
            .entries
            .values()
            .next()
            .unwrap()
            .cap
            .at,
        permit_deadline
    );
    scene.advance(Duration::from_millis(500));
    scene.guard_expires_at = scene.instant + Duration::from_secs(30);
    scene
        .tables
        .get_mut(&node(1))
        .unwrap()
        .expire(scene.instant, scene.now_ms);
    scene
        .process(1, None, 1, &[1, 2, 3], udp(1, 3, 50000, 443))
        .unwrap();
    assert_eq!(
        scene.tables[&node(1)]
            .entries
            .values()
            .next()
            .unwrap()
            .cap
            .at,
        permit_deadline
    );
    scene.advance(Duration::from_millis(500));
    assert!(
        scene
            .process(1, Some(2), 3, &[1, 2, 3], udp(3, 1, 443, 50000))
            .is_err()
    );
    assert!(
        scene
            .process(1, None, 1, &[1, 2, 3], udp(1, 3, 50000, 443))
            .is_err()
    );
    assert_eq!(scene.tables[&node(1)].len(), 0);
    assert_eq!(
        scene.configs[&node(1)].permits[&request()].expires_at_ms,
        100_000
    );
}

#[test]
fn compiled_exit_is_exact_and_requires_the_terminal_exit_capability() {
    let mut scene = Scene::new(BusinessProtocol::Udp, true, true);
    let packet = || {
        Packet::parse(datagram(
            address(1),
            "1.1.1.1".parse().unwrap(),
            17,
            udp_body(50000, 443, &[]),
        ))
        .unwrap()
    };
    assert!(scene.process(2, Some(1), 1, &[1, 2], packet()).is_err());
    let action = scene.process(3, Some(2), 1, &[1, 2, 3], packet()).unwrap();
    assert!(matches!(action, Action::Exit { .. }));
    let response = Packet::parse(datagram(
        "1.1.1.1".parse().unwrap(),
        address(1),
        17,
        udp_body(443, 50000, &[]),
    ))
    .unwrap();
    unpack(
        scene.process(3, None, 3, &[1, 2, 3], response).unwrap(),
        Some(2),
        Direction::Reply,
        &[1, 2, 3],
    );
    scene.tables.get_mut(&node(3)).unwrap().clear();
    scene.configs.get_mut(&node(3)).unwrap().exit_enabled = false;
    assert!(matches!(
        scene.process(3, Some(2), 1, &[1, 2, 3], packet()),
        Err(Error::Admission(_))
    ));
    assert_eq!(scene.tables[&node(3)].len(), 0);
}
