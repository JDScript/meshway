use super::*;

fn established() -> Scene {
    let mut scene = Scene::new(BusinessProtocol::Tcp, false, true);
    forward(&mut scene, tcp(false, 100, 0, 2, 0));
    reply(&mut scene, tcp(true, 900, 101, 0x12, 0));
    forward(&mut scene, tcp(false, 101, 901, 0x10, 0));
    scene
}

fn discard_after_relay(scene: &mut Scene, packet: Packet) {
    let packet = unpack(
        scene.process(1, None, 1, &[1, 2, 3], packet).unwrap(),
        Some(2),
        Direction::Forward,
        &[1, 2, 3],
    );
    // Source and relay committed their one flow, but terminal never receives it.
    drop(scene.process(2, Some(1), 1, &[1, 2, 3], packet).unwrap());
}

#[test]
fn admitted_but_undelivered_data_recovers_with_delayed_ack_and_nonlatest_resegmentation() {
    let mut scene = established();
    discard_after_relay(&mut scene, tcp(false, 101, 901, 0x18, 7));
    // Later bytes reach the terminal before the missing prefix. They must be
    // delivered so its real TCP stack can produce duplicate ACK/SACK feedback.
    forward(&mut scene, tcp(false, 108, 901, 0x18, 3));
    reply(&mut scene, tcp(true, 901, 101, 0x10, 0)); // OS still expects 101.
    forward(&mut scene, tcp(false, 101, 901, 0x18, 4));
    forward(&mut scene, tcp(false, 105, 901, 0x18, 6)); // Coalesce old boundaries.
    reply(&mut scene, tcp(true, 901, 111, 0x18, 5));
    // Reordered ACKs do not roll back either sequence frontier.
    reply(&mut scene, tcp(true, 901, 101, 0x10, 0));
    forward(&mut scene, tcp(false, 111, 901, 0x18, 2));
    reply(&mut scene, tcp(true, 906, 113, 0x10, 0));
    for table in scene.tables.values() {
        assert_eq!(table.len(), 1);
        let key = table.entries.keys().next().unwrap();
        assert_eq!(key.demand, request());
        assert_eq!(key.path, vec![node(1), node(2), node(3)]);
    }
}

#[test]
fn future_data_and_overlap_do_not_authorize_acknowledging_an_unfilled_gap() {
    let mut scene = established();
    forward(&mut scene, tcp(false, 101, 901, 0x18, 10));
    for packet in [
        tcp(false, 100, 901, 0x10, 0), // Before the initial SYN-end floor.
        tcp(false, 106, 901, 0x11, 0), // Cannot invent FIN inside prior data.
        tcp(false, 111, 902, 0x18, 1), // Future peer ACK.
        tcp(false, 111, 900, 0x18, 1), // ACK before peer SYN end.
        tcp(false, 111, 901_u32.wrapping_add(1 << 31), 0x10, 0),
    ] {
        assert!(matches!(
            scene.process(1, None, 1, &[1, 2, 3], packet),
            Err(Error::Flow)
        ));
    }
    forward(&mut scene, tcp(false, 120, 901, 0x18, 3));
    forward(&mut scene, tcp(false, 121, 901, 0x10, 0));
    assert!(matches!(
        scene.process(3, None, 3, &[1, 2, 3], tcp(true, 901, 123, 0x10, 0)),
        Err(Error::Flow)
    ));
    forward(&mut scene, tcp(false, 106, 901, 0x18, 6));
    reply(&mut scene, tcp(true, 901, 112, 0x10, 0));
    forward(&mut scene, tcp(false, 102, 901, 0x18, 8));
    forward(&mut scene, tcp(false, 112, 901, 0x18, 8));
    reply(&mut scene, tcp(true, 901, 123, 0x10, 0));
}

#[test]
fn exact_handshake_retransmits_after_establishment_never_reset_sequence_state() {
    let mut scene = established();
    forward(&mut scene, tcp(false, 101, 901, 0x18, 7));
    reply(&mut scene, tcp(true, 901, 108, 0x18, 5));
    forward(&mut scene, tcp(false, 100, 0, 2, 0));
    reply(&mut scene, tcp(true, 900, 101, 0x12, 0));
    for packet in [
        tcp(false, 99, 0, 2, 0),
        tcp(false, 100, 0, 0x42, 0),
        tcp(false, 100, 0, 2, 1),
    ] {
        assert!(matches!(
            scene.process(1, None, 1, &[1, 2, 3], packet),
            Err(Error::Flow)
        ));
    }
    for packet in [
        tcp(true, 899, 101, 0x12, 0),
        tcp(true, 900, 101, 0x52, 0),
        tcp(true, 900, 102, 0x12, 0),
        tcp(true, 900, 101, 0x12, 1),
    ] {
        assert!(matches!(
            scene.process(3, None, 3, &[1, 2, 3], packet),
            Err(Error::Flow)
        ));
    }
    // The original handshake was accepted without returning either side to ISN.
    forward(&mut scene, tcp(false, 108, 906, 0x18, 2));
    reply(&mut scene, tcp(true, 906, 110, 0x18, 3));
}

#[test]
fn fin_resegmentation_preserves_its_original_sequence_consumption_in_both_directions() {
    let mut scene = established();
    forward(&mut scene, tcp(false, 101, 901, 0x11, 7)); // FIN occupies 108.
    forward(&mut scene, tcp(false, 105, 901, 0x11, 3));
    forward(&mut scene, tcp(false, 108, 901, 0x11, 0));
    forward(&mut scene, tcp(false, 101, 901, 0x18, 7));
    for packet in [
        tcp(false, 109, 901, 0x18, 1),
        tcp(false, 101, 901, 0x18, 8),
        tcp(false, 107, 901, 0x11, 0),
        tcp(false, 109, 901, 0x11, 0),
    ] {
        assert!(matches!(
            scene.process(1, None, 1, &[1, 2, 3], packet),
            Err(Error::Flow)
        ));
    }
    reply(&mut scene, tcp(true, 901, 109, 0x11, 5)); // FIN occupies 906.
    reply(&mut scene, tcp(true, 903, 101, 0x11, 3)); // Older bounded ACK is valid.
    reply(&mut scene, tcp(true, 906, 109, 0x11, 0));
    assert!(matches!(
        scene.process(3, None, 3, &[1, 2, 3], tcp(true, 907, 109, 0x18, 1)),
        Err(Error::Flow)
    ));
    forward(&mut scene, tcp(false, 109, 907, 0x10, 0));
    reply(&mut scene, tcp(true, 907, 109, 0x10, 0));
    for table in scene.tables.values() {
        assert_eq!(table.len(), 1);
    }
}

#[test]
fn established_reset_requires_current_sequence_and_bounded_peer_ack_in_both_directions() {
    for reverse in [false, true] {
        let mut scene = established();
        forward(&mut scene, tcp(false, 101, 901, 0x18, 7));
        reply(&mut scene, tcp(true, 901, 108, 0x18, 5));
        let (local, sequence, ack) = if reverse {
            (3, 906, 108)
        } else {
            (1, 108, 906)
        };
        for packet in [
            tcp(reverse, sequence + 1, ack, 0x14, 0),
            tcp(reverse, sequence - 1, ack, 0x14, 0),
            tcp(reverse, sequence, ack + 1, 0x14, 0),
            tcp(reverse, sequence, 50000, 0x14, 0),
            tcp(reverse, sequence, ack, 0x14, 1),
        ] {
            assert!(matches!(
                scene.process(local, None, local, &[1, 2, 3], packet),
                Err(Error::Flow)
            ));
        }
        if reverse {
            reply(&mut scene, tcp(true, sequence, ack, 0x14, 0));
        } else {
            forward(&mut scene, tcp(false, sequence, ack, 0x14, 0));
        }
        for table in scene.tables.values() {
            assert_eq!(table.len(), 0);
        }
    }
}

#[test]
fn handshake_reset_simultaneous_open_and_fast_open_remain_rejected() {
    let mut scene = Scene::new(BusinessProtocol::Tcp, false, true);
    assert!(matches!(
        scene.process(1, None, 1, &[1, 2, 3], tcp(false, 100, 0, 2, 1)),
        Err(Error::Flow)
    ));
    forward(&mut scene, tcp(false, 100, 0, 2, 0));
    assert!(matches!(
        scene.process(3, None, 3, &[1, 2, 3], tcp(true, 900, 0, 2, 0)),
        Err(Error::Flow)
    ));
    reply(&mut scene, tcp(true, 900, 101, 0x12, 0));
    assert!(matches!(
        scene.process(1, None, 1, &[1, 2, 3], tcp(false, 101, 901, 0x14, 0)),
        Err(Error::Flow)
    ));
    assert!(matches!(
        scene.process(3, None, 3, &[1, 2, 3], tcp(true, 901, 101, 0x14, 0)),
        Err(Error::Flow)
    ));
    forward(&mut scene, tcp(false, 101, 901, 0x10, 0));
    forward(&mut scene, tcp(false, 101, 901, 0x18, 7));
    for table in scene.tables.values() {
        assert_eq!(table.len(), 1);
    }
}
