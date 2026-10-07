use super::*;

fn state(header: VirtioNetHdr, packet: &[u8]) -> ReadState {
    let mut scratch = vec![0; VIRTIO_NET_HDR_LEN];
    header.encode(&mut scratch).unwrap();
    scratch.extend_from_slice(packet);
    ReadState {
        scratch,
        buffers: Vec::new(),
        sizes: Vec::new(),
        pending: 0..0,
    }
}

fn udp() -> (VirtioNetHdr, Vec<u8>) {
    let mut packet = vec![0; 32];
    packet[0] = 0x45;
    packet[3] = 32;
    packet[8] = 64;
    packet[9] = 17;
    packet[15] = 1;
    packet[19] = 2;
    packet[21] = 1;
    packet[23] = 2;
    packet[25] = 12;
    // The first two-byte payload makes a complete UDP checksum exactly zero:
    // address words 1+2, ports 1+2, UDP length 10 twice, protocol 17 = 43.
    packet[28..30].copy_from_slice(&(!43_u16).to_be_bytes());
    (
        VirtioNetHdr {
            flags: 1,
            gso_type: VIRTIO_NET_HDR_GSO_UDP_L4,
            hdr_len: 32, // deliberately Linux FORWARD's first-packet length
            gso_size: 2,
            csum_start: 20,
            csum_offset: 6,
        },
        packet,
    )
}

#[test]
fn normalization_reuses_high_water_slots_across_small_and_large_aggregates() {
    let (mut header, mut packet) = udp();
    header.gso_size = 1;
    packet.resize(28 + MAX_SEGMENTS, 0);
    let mut read = state(header, &packet);
    read.normalize(read.scratch.len(), 1280).unwrap();
    let pointers: Vec<_> = read.buffers.iter().map(Vec::as_ptr).collect();
    let capacity: usize = read.buffers.iter().map(Vec::capacity).sum();
    assert_eq!(read.pending.len(), MAX_SEGMENTS);
    assert!(capacity <= 2 * MAX_SUPER);
    for _ in 0..8 {
        let (small_header, small_packet) = udp();
        small_header.encode(&mut read.scratch).unwrap();
        read.scratch[VIRTIO_NET_HDR_LEN..VIRTIO_NET_HDR_LEN + small_packet.len()]
            .copy_from_slice(&small_packet);
        read.normalize(VIRTIO_NET_HDR_LEN + small_packet.len(), 1280)
            .unwrap();
        assert_eq!(read.pending.len(), 2);
        assert_eq!(read.buffers.len(), MAX_SEGMENTS);
        header.encode(&mut read.scratch).unwrap();
        read.scratch[VIRTIO_NET_HDR_LEN..].copy_from_slice(&packet);
        read.normalize(read.scratch.len(), 1280).unwrap();
        assert_eq!(read.pending.len(), MAX_SEGMENTS);
        // The first two slots grew by one byte for the small aggregate. All
        // remaining allocations survived; they were not freed/recreated.
        assert_eq!(
            read.buffers[2..]
                .iter()
                .map(Vec::as_ptr)
                .collect::<Vec<_>>(),
            pointers[2..]
        );
        assert!(read.buffers.iter().map(Vec::capacity).sum::<usize>() <= capacity + 64);
    }
}

#[test]
fn normalization_preserves_zero_checksum_encoding_and_ignores_kernel_hdr_len() {
    let (header, packet) = udp();
    let mut state = state(header, &packet);
    state.normalize(state.scratch.len(), 1280).unwrap();
    assert_eq!(state.pending, 0..2);
    assert_eq!(state.sizes, [30, 30]);
    assert_eq!(&state.buffers[0][26..28], &[0xff, 0xff]);
}

#[test]
fn partial_checksum_uses_seed_and_rejects_invalid_bounds() {
    let (mut header, mut packet) = udp();
    header.gso_type = 0;
    packet.truncate(30);
    packet[3] = 30;
    packet[25] = 10;
    packet[26..28].copy_from_slice(&30_u16.to_be_bytes()); // addresses + proto + length
    let mut valid = state(header, &packet);
    valid.normalize(valid.scratch.len(), 1280).unwrap();
    assert_eq!(&valid.buffers[0][26..28], &[0xff, 0xff]);
    for offset in [10, u16::MAX] {
        header.csum_offset = offset;
        let mut invalid = state(header, &packet);
        assert!(invalid.normalize(invalid.scratch.len(), 1280).is_err());
    }
}

#[test]
fn malformed_aggregates_are_errors_without_publishing_segments() {
    for mutation in 0..9 {
        let (mut header, mut packet) = udp();
        match mutation {
            0 => header.flags = 4,
            1 => header.flags = 0,
            2 => header.gso_type = 0x7f,
            3 => header.gso_size = 0,
            4 => header.gso_size = 1280,
            5 => header.csum_start = 19,
            6 => header.csum_offset = u16::MAX,
            7 => packet[6] = 0x20,
            _ => {
                packet.truncate(28);
            }
        }
        let mut invalid = state(header, &packet);
        assert!(
            invalid.normalize(invalid.scratch.len(), 1280).is_err(),
            "mutation {mutation}"
        );
        assert!(invalid.pending.is_empty());
    }
    let (mut header, mut packet) = udp();
    header.gso_size = 1;
    packet.resize(28 + MAX_SEGMENTS + 1, 0);
    let mut invalid = state(header, &packet);
    assert!(invalid.normalize(invalid.scratch.len(), 1280).is_err());
    assert!(invalid.pending.is_empty());
}

#[test]
fn gro_does_not_repair_bad_ipv4_checksum_or_udp_length() {
    let (_, mut packet) = udp();
    let ip_checksum = !checksum(&packet[..20], 0);
    packet[10..12].copy_from_slice(&ip_checksum.to_be_bytes());
    let value = !checksum(&packet[20..], 3 + 17 + 12);
    packet[26..28].copy_from_slice(&value.to_be_bytes());
    assert!(gro_admissible(&packet));
    for at in [10, 25] {
        let mut invalid = packet.clone();
        invalid[at] ^= 1;
        assert!(!gro_admissible(&invalid));
        let mut write = WriteState::default();
        assert_eq!(write.prepare(&[&invalid, &invalid], true).unwrap(), 1);
        assert_eq!(&write.buffers[0][VIRTIO_NET_HDR_LEN..], invalid);
        assert_eq!(VirtioNetHdr::decode(&write.buffers[0]).unwrap().gso_type, 0);
    }
}

#[test]
fn clearing_noninitial_cwr_keeps_transport_checksum_valid() {
    let (mut header, mut packet) = udp();
    packet.resize(44, 0);
    packet[3] = 44;
    packet[9] = 6;
    packet[24..].fill(0);
    packet[32] = 0x50;
    packet[33] = 0x90;
    header.gso_type = VIRTIO_NET_HDR_GSO_TCPV4;
    header.csum_offset = 16;
    let mut state = state(header, &packet);
    state.normalize(state.scratch.len(), 1280).unwrap();
    assert_eq!(state.pending, 0..2);
    assert_eq!(state.buffers[0][33], 0x90);
    assert_eq!(state.buffers[1][33], 0x10);
    for packet in state.buffers {
        assert_eq!(checksum(&packet[20..], 3 + 6 + 22), u16::MAX);
    }
}
