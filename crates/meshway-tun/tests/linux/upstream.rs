//! Black-box coverage of the pinned tun-rs codec, with independent checksums.
use super::*;
use tun_rs::{GROTable, VIRTIO_NET_HDR_LEN, VirtioNetHdr, gso_split};

const OFFSET: usize = VIRTIO_NET_HDR_LEN;

fn finish(packet: &mut [u8]) {
    let v6 = packet[0] >> 4 == 6;
    let ip = if v6 { 40 } else { 20 };
    let protocol = packet[if v6 { 6 } else { 9 }];
    let at = ip + if protocol == 6 { 16 } else { 6 };
    packet[at..at + 2].fill(0);
    if !v6 {
        packet[10..12].fill(0);
        let value = checksum(&packet[..ip]);
        packet[10..12].copy_from_slice(&value.to_be_bytes());
    }
    let mut pseudo = if v6 {
        packet[8..40].to_vec()
    } else {
        packet[12..20].to_vec()
    };
    pseudo.extend_from_slice(&[0, protocol]);
    pseudo.extend_from_slice(&((packet.len() - ip) as u16).to_be_bytes());
    pseudo.extend_from_slice(&packet[ip..]);
    let value = checksum(&pseudo);
    packet[at..at + 2].copy_from_slice(&value.to_be_bytes());
}

pub(super) fn assert_checksums(packet: &[u8]) {
    let mut checked = packet.to_vec();
    finish(&mut checked);
    assert_eq!(packet, checked, "independent IP/transport checksum");
}

fn packet(v6: bool, tcp: bool, index: usize, size: usize) -> Vec<u8> {
    let (local, remote) = if v6 {
        (LOCAL_V6.into(), REMOTE_V6.into())
    } else {
        (LOCAL_V4.into(), REMOTE_V4.into())
    };
    let payload = vec![index as u8 + 1; size];
    let mut packet = if tcp {
        tcp_reply(
            local,
            remote,
            5678,
            7,
            false,
            (index as u32 * 1000, &payload),
        )
    } else {
        udp_packet(local, remote, 1234, 5678, &payload)
    };
    if !v6 {
        packet[4..6].copy_from_slice(&(index as u16).to_be_bytes());
    }
    if tcp {
        let ip = if v6 { 40 } else { 20 };
        packet[ip + 4..ip + 8].copy_from_slice(
            &(u32::MAX - 100)
                .wrapping_add(index as u32 * 1000)
                .to_be_bytes(),
        );
    }
    finish(&mut packet);
    packet
}

fn gro(packets: &[Vec<u8>], udp: bool) -> Vec<Vec<u8>> {
    let mut buffers: Vec<_> = packets
        .iter()
        .map(|packet| {
            let mut buffer = Vec::with_capacity(65535 + 2 * OFFSET);
            buffer.resize(OFFSET, 0);
            buffer.extend_from_slice(packet);
            buffer
        })
        .collect();
    GROTable::new()
        .apply_gro(&mut buffers, OFFSET, udp)
        .unwrap();
    buffers
}

fn segments(buffer: &[u8]) -> usize {
    let header = VirtioNetHdr::decode(buffer).unwrap();
    if header.gso_type == 0 {
        1
    } else {
        (buffer.len() - OFFSET - usize::from(header.hdr_len)).div_ceil(usize::from(header.gso_size))
    }
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
                packets[2][if v6 { 53 } else { 33 }] |= 8;
                finish(&mut packets[2]);
            }
            let mut buffers = gro(&packets, true);
            assert_eq!(segments(&buffers[0]), 3);
            let header = VirtioNetHdr::decode(&buffers[0]).unwrap();
            if tcp {
                // FIN is not a GRO candidate, but may occur on kernel GSO input.
                buffers[0][OFFSET + if v6 { 53 } else { 33 }] |= 1;
                packets[2][if v6 { 53 } else { 33 }] |= 1;
                finish(&mut packets[2]);
            }
            let mut out = vec![vec![0xa5; MTU + 17]; 3];
            let mut sizes = [0; 3];
            assert_eq!(
                gso_split(
                    &mut buffers[0][OFFSET..],
                    header,
                    &mut out,
                    &mut sizes,
                    17,
                    v6
                )
                .unwrap(),
                3
            );
            for (i, output) in out.iter_mut().enumerate() {
                assert_eq!(&output[..17], &[0xa5; 17]);
                assert_eq!(output[17..17 + sizes[i]], packets[i]);
                assert_checksums(&output[17..17 + sizes[i]]);
            }
        }
    }
}

#[test]
fn refuses_corruption_and_incompatible_sequences_or_flows() {
    let first = packet(false, true, 0, 1000);
    for mutation in [0, 1, 2, 3] {
        let mut next = packet(false, true, 1, 1000);
        match mutation {
            0 => next[50] ^= 1,
            1 => next[33] &= !0x10,
            2 => next[27] ^= 1,
            _ => next[23] ^= 1,
        }
        if mutation != 0 {
            finish(&mut next);
        }
        assert_eq!(segments(&gro(&[first.clone(), next], true)[0]), 1);
    }
    let udp = packet(false, false, 0, 1000);
    assert_eq!(segments(&gro(&[udp.clone(), udp], false)[0]), 1);
}

#[test]
fn upstream_merge_boundaries_and_prepend() {
    for v6 in [false, true] {
        let first = packet(v6, true, 0, 1000);
        let next = packet(v6, true, 1, 1000);
        assert_eq!(segments(&gro(&[next.clone(), first.clone()], true)[0]), 2);
        let ip = if v6 { 40 } else { 20 };
        for flags in [0x18, 0x11, 0x12, 0x14, 0x30, 0x50, 0x90] {
            let mut changed = first.clone();
            changed[ip + 13] = flags;
            finish(&mut changed);
            assert_eq!(segments(&gro(&[changed, next.clone()], true)[0]), 1);
        }
        // IPv4 ID continuity and IPv6 flow-label equality are not GRO gates.
        let mut changed = next.clone();
        if v6 {
            changed[3] = 1;
        } else {
            changed[4..6].fill(99);
        }
        finish(&mut changed);
        assert_eq!(segments(&gro(&[first.clone(), changed], true)[0]), 2);
        // TCP receive-window differences are also accepted by upstream.
        let mut changed = next.clone();
        changed[ip + 14] ^= 1;
        finish(&mut changed);
        assert_eq!(segments(&gro(&[first.clone(), changed], true)[0]), 2);
        let mut changed = next;
        changed[if v6 { 7 } else { 8 }] -= 1;
        finish(&mut changed);
        assert_eq!(segments(&gro(&[first, changed], true)[0]), 1);
    }
    let mut options = packet(false, true, 0, 1000);
    options.splice(20..20, [1, 1, 1, 1]);
    options[0] = 0x46;
    let len = options.len() as u16;
    options[2..4].copy_from_slice(&len.to_be_bytes());
    options[10..12].fill(0);
    let value = checksum(&options[..24]);
    options[10..12].copy_from_slice(&value.to_be_bytes());
    assert_eq!(segments(&gro(&[options.clone(), options], true)[0]), 1);
}

#[test]
fn invalid_headers_and_output_bounds() {
    assert_eq!(
        checksum(&[0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7]),
        0x220d
    );
    for len in 0..OFFSET {
        assert!(VirtioNetHdr::decode(&vec![0; len]).is_err());
    }
    let packets = vec![packet(false, true, 0, 1000), packet(false, true, 1, 1000)];
    let aggregate = gro(&packets, true).remove(0);
    let header = VirtioNetHdr::decode(&aggregate).unwrap();
    for mutation in 0..5 {
        let mut bad = header;
        match mutation {
            0 => bad.gso_size = 0,
            1 => bad.csum_start = 1,
            2 => bad.hdr_len = 19,
            3 => bad.hdr_len = 60000,
            _ => bad.csum_offset = 10000,
        }
        let mut input = aggregate[OFFSET..].to_vec();
        assert!(
            gso_split(
                &mut input,
                bad,
                &mut [vec![0; MTU], vec![0; MTU]],
                &mut [0; 2],
                0,
                false
            )
            .is_err()
        );
    }
    let mut input = aggregate[OFFSET..].to_vec();
    assert!(gso_split(&mut input, header, &mut [vec![0; MTU]], &mut [0], 0, false).is_err());
    let mut input = aggregate[OFFSET..].to_vec();
    assert!(
        gso_split(
            &mut input,
            header,
            &mut [vec![0; 20], vec![0; 20]],
            &mut [0; 2],
            0,
            false
        )
        .is_err()
    );
}

pub(super) fn verify_codec() {
    tcp_udp_ipv4_ipv6_round_trip_with_short_tail_and_sequence_wrap();
    refuses_corruption_and_incompatible_sequences_or_flows();
    upstream_merge_boundaries_and_prepend();
    invalid_headers_and_output_bounds();
}

pub(super) async fn reject_invalid_kernel_headers() {
    let device = tun_rs::DeviceBuilder::new()
        .offload(true)
        .build_async()
        .unwrap();
    for len in 0..OFFSET {
        assert!(device.send(&vec![0; len]).await.is_err());
    }
    let packets = vec![packet(false, true, 0, 1000), packet(false, true, 1, 1000)];
    let mut aggregate = gro(&packets, true).remove(0);
    aggregate[1] = 0x7f;
    assert!(
        device.send(&aggregate).await.is_err(),
        "kernel rejects unknown GSO type"
    );
}
