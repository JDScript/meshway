use super::*;

fn packet(v6: bool, tcp: bool, index: usize, size: usize) -> Vec<u8> {
    let ip = if v6 { 40 } else { 20 };
    let header = ip + if tcp { 20 } else { 8 };
    let mut packet = vec![0; header + size];
    packet[0] = if v6 { 0x60 } else { 0x45 };
    if v6 {
        packet[6] = if tcp { 6 } else { 17 };
        packet[7] = 64;
        packet[23] = 1;
        packet[39] = 2;
    } else {
        put(&mut packet, 4, index as u16);
        packet[8] = 64;
        packet[9] = if tcp { 6 } else { 17 };
        packet[15] = 1;
        packet[19] = 2;
    }
    put(&mut packet, ip, 1234);
    put(&mut packet, ip + 2, 5678);
    if tcp {
        packet[ip + 12] = 0x50;
        packet[ip + 13] = 0x10;
        packet[ip + 4..ip + 8].copy_from_slice(
            &(u32::MAX - 100)
                .wrapping_add(index as u32 * 1000)
                .to_be_bytes(),
        );
    }
    packet[header..].fill(index as u8 + 1);
    Layout::parse(&packet).unwrap().finish(&mut packet);
    packet
}

#[test]
fn tcp_udp_ipv4_ipv6_round_trip_with_short_tail_and_sequence_wrap() {
    for v6 in [false, true] {
        for tcp in [false, true] {
            let mut packets = vec![
                packet(v6, tcp, 0, 1000),
                packet(v6, tcp, 1, 1000),
                packet(v6, tcp, 2, 301),
            ];
            if tcp {
                let last = &mut packets[2];
                let layout = Layout::parse(last).unwrap();
                last[layout.ip + 13] |= 8;
                layout.finish(last);
            }
            let refs: Vec<_> = packets.iter().map(Vec::as_slice).collect();
            let (aggregate, count) = merge(&refs, true);
            assert_eq!(count, 3);
            let decoded = split(&aggregate, 1280).unwrap();
            assert_eq!(decoded, packets);
            for packet in decoded {
                assert!(Layout::parse(&packet).unwrap().valid(&packet));
            }
        }
    }
}

#[test]
fn checksum_known_vector_and_partial_packet() {
    assert_eq!(
        fold(sum(&[0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7])),
        0x220d
    );
    for v6 in [false, true] {
        for tcp in [false, true] {
            let packet = packet(v6, tcp, 0, 301);
            let layout = Layout::parse(&packet).unwrap();
            let mut input = vec![0; HEADER];
            input.extend_from_slice(&packet);
            input[0] = 1;
            input[6..8].copy_from_slice(&(layout.ip as u16).to_ne_bytes());
            input[8..10].copy_from_slice(&((layout.checksum - layout.ip) as u16).to_ne_bytes());
            put(
                &mut input[HEADER..],
                layout.checksum,
                !fold(layout.pseudo(&packet)),
            );
            assert_eq!(split(&input, 1280).unwrap(), vec![packet]);
        }
    }
}

#[test]
fn refuses_corruption_and_incompatible_sequences_or_flows() {
    let first = packet(false, true, 0, 1000);
    let mut next = packet(false, true, 1, 1000);
    next[50] ^= 1;
    assert_eq!(merge(&[&first, &next], true).1, 1);
    let mut next = packet(false, true, 1, 1000);
    next[33] &= !0x10;
    Layout::parse(&next).unwrap().finish(&mut next);
    assert_eq!(merge(&[&first, &next], true).1, 1);
    let mut next = packet(false, true, 1, 1000);
    next[27] ^= 1;
    Layout::parse(&next).unwrap().finish(&mut next);
    assert_eq!(merge(&[&first, &next], true).1, 1);
    let udp = packet(false, false, 0, 1000);
    assert_eq!(merge(&[&udp, &udp], false).1, 1);
    for len in 0..10 {
        assert!(split(&vec![0; len], 1280).is_err());
    }
    let (mut aggregate, _) = merge(&[&first, &packet(false, true, 1, 1000)], true);
    aggregate[4..6].fill(0);
    assert!(split(&aggregate, 1280).is_err());
}
