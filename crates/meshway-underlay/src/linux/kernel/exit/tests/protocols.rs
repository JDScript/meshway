use super::*;

pub(super) async fn round_trips(
    backend: &mut backend::Backend,
    tun: &meshway_tun::Tun,
    tuple: OriginalTuple,
    external_source: &str,
) {
    let scope = ExitScope {
        binding: Digest::from_bytes([11; 32]),
        revoked: CancellationToken::new(),
    };
    let echo = OriginalTuple {
        transport: ExitTransport::IcmpEcho {
            identifier: 17102,
            sequence: 1,
        },
        ..tuple
    };
    backend
        .install(id(102), echo, cutoff(30_000), scope.clone())
        .await
        .unwrap();
    let mut body = vec![8, 0, 0, 0, 0x42, 0xce, 0, 1];
    body.extend_from_slice(b"x3-echo");
    let sum = checksum(&body);
    body[2..4].copy_from_slice(&sum.to_be_bytes());
    tun.send(&ip(echo, &body)).await.unwrap();
    assert!(matches!(
        backend.confirm(id(102)).await.unwrap(),
        CtConfirmation::Confirmed { .. }
    ));
    let mut packet = vec![0; 1280];
    let size = tokio::time::timeout(Duration::from_secs(2), recv_ipv4(tun, &mut packet))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(packet[20], 0);
    assert_eq!(&packet[24..size], &body[4..]);
    let shared = OriginalTuple {
        transport: ExitTransport::IcmpEcho {
            identifier: 17102,
            sequence: 2,
        },
        ..echo
    };
    backend
        .install(id(104), shared, cutoff(30_000), scope.clone())
        .await
        .unwrap();
    let old_ct = conntrack::lookup_owned(&mut backend.commands, echo, backend.ledger.program.mark)
        .await
        .unwrap()
        .unwrap();
    let mut second = body.clone();
    second[6..8].copy_from_slice(&2u16.to_be_bytes());
    second[2..4].fill(0);
    let sum = checksum(&second);
    second[2..4].copy_from_slice(&sum.to_be_bytes());
    tun.send(&ip(shared, &second)).await.unwrap();
    assert!(
        matches!(backend.confirm(id(104)).await.unwrap(), CtConfirmation::Confirmed { conntrack_id, .. } if conntrack_id == old_ct.id)
    );
    tokio::time::timeout(Duration::from_secs(2), recv_ipv4(tun, &mut packet))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&packet[26..28], &2u16.to_be_bytes());
    backend.retire(id(102)).await.unwrap();
    assert_eq!(
        conntrack::lookup_owned(&mut backend.commands, shared, backend.ledger.program.mark)
            .await
            .unwrap()
            .unwrap()
            .id,
        old_ct.id
    );
    tun.send(&ip(echo, &body)).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), recv_ipv4(tun, &mut packet))
            .await
            .is_err()
    );
    tun.send(&ip(shared, &second)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), recv_ipv4(tun, &mut packet))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&packet[26..28], &2u16.to_be_bytes());
    backend.retire(id(104)).await.unwrap();
    assert!(
        conntrack::lookup_owned(&mut backend.commands, shared, backend.ledger.program.mark)
            .await
            .unwrap()
            .is_none()
    );
    println!(
        "X3C_ICMP=identifier-and-sequence,shared-ct-retained,retired-sequence-drops,last-retirement-clears-ct"
    );

    let tcp = OriginalTuple {
        transport: ExitTransport::Tcp {
            source_port: 17103,
            destination_port: tuple.fields().2,
        },
        ..tuple
    };
    backend
        .install(id(103), tcp, cutoff(30_000), scope)
        .await
        .unwrap();
    tun.send(&segment(tcp, 1000, 0, 2, &[])).await.unwrap();
    assert!(matches!(
        backend.confirm(id(103)).await.unwrap(),
        CtConfirmation::Confirmed { .. }
    ));
    tokio::time::timeout(Duration::from_secs(2), recv_ipv4(tun, &mut packet))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(packet[33] & 0x12, 0x12);
    let ack = u32::from_be_bytes(packet[24..28].try_into().unwrap()).wrapping_add(1);
    tun.send(&segment(tcp, 1001, ack, 0x10, &[])).await.unwrap();
    tun.send(&segment(tcp, 1001, ack, 0x18, b"x3-tcp"))
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let size = recv_ipv4(tun, &mut packet).await.unwrap();
            let offset = 20 + usize::from(packet[32] >> 4) * 4;
            if size > offset {
                break packet[offset..size].to_vec();
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        response,
        format!("from={external_source}|x3-tcp").as_bytes()
    );
    backend.retire(id(103)).await.unwrap();
    assert!(
        conntrack::lookup_owned(&mut backend.commands, tcp, backend.ledger.program.mark)
            .await
            .unwrap()
            .is_none()
    );
    // ACK/FIN may already have crossed the final gate before retirement. Drain
    // only this exact old TCP tuple, then test traffic submitted after the ACK.
    for index in 0..64 {
        match tokio::time::timeout(Duration::from_millis(50), recv_ipv4(tun, &mut packet)).await {
            Err(_) => break,
            Ok(Ok(size)) => {
                assert!(index < 63 && size >= 40 && packet[9] == 6);
                assert_eq!(u16::from_be_bytes([packet[22], packet[23]]), 17103);
            }
            Ok(Err(error)) => panic!("TUN read: {error}"),
        }
    }
    tun.send(&segment(tcp, 2000, 0, 2, &[])).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), recv_ipv4(tun, &mut packet))
            .await
            .is_err()
    );
    println!("X3_PROTOCOLS=icmp-echo,tcp-handshake-and-payload,tcp-exact-retirement");
}

fn checksum(bytes: &[u8]) -> u16 {
    let (pairs, rest) = bytes.as_chunks::<2>();
    let mut sum: u32 = pairs
        .iter()
        .map(|pair| u32::from(u16::from_be_bytes(*pair)))
        .sum();
    if let Some(byte) = rest.first() {
        sum += u32::from(*byte) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}
fn ip(tuple: OriginalTuple, body: &[u8]) -> Vec<u8> {
    let mut packet = vec![0; 20];
    packet[0] = 0x45;
    packet[2..4].copy_from_slice(&u16::try_from(20 + body.len()).unwrap().to_be_bytes());
    packet[8] = 64;
    packet[9] = tuple.fields().0;
    packet[12..16].copy_from_slice(&tuple.source.octets());
    packet[16..20].copy_from_slice(&tuple.destination.octets());
    let sum = checksum(&packet);
    packet[10..12].copy_from_slice(&sum.to_be_bytes());
    packet.extend_from_slice(body);
    packet
}
fn segment(tuple: OriginalTuple, sequence: u32, ack: u32, flags: u8, payload: &[u8]) -> Vec<u8> {
    let mut body = vec![0; 20];
    body[0..2].copy_from_slice(&tuple.fields().1.to_be_bytes());
    body[2..4].copy_from_slice(&tuple.fields().2.to_be_bytes());
    body[4..8].copy_from_slice(&sequence.to_be_bytes());
    body[8..12].copy_from_slice(&ack.to_be_bytes());
    body[12] = 5 << 4;
    body[13] = flags;
    body[14..16].copy_from_slice(&4096u16.to_be_bytes());
    body.extend_from_slice(payload);
    let mut pseudo = Vec::new();
    pseudo.extend_from_slice(&tuple.source.octets());
    pseudo.extend_from_slice(&tuple.destination.octets());
    pseudo.extend_from_slice(&[0, 6]);
    pseudo.extend_from_slice(&u16::try_from(body.len()).unwrap().to_be_bytes());
    pseudo.extend_from_slice(&body);
    body[16..18].copy_from_slice(&checksum(&pseudo).to_be_bytes());
    ip(tuple, &body)
}
