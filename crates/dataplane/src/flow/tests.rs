use super::*;

fn paths() -> Vec<String> {
    vec!["a".into(), "via".into(), "b".into()]
}
fn plans() -> (StatefulIpv4Plan, StatefulIpv4Plan) {
    let a = "100.64.0.1".parse().unwrap();
    let b = "100.64.0.2".parse().unwrap();
    (
        StatefulIpv4Plan::new(a, 1200, BTreeMap::from([(b, paths())]), BTreeMap::new()),
        StatefulIpv4Plan::new(b, 1200, BTreeMap::new(), BTreeMap::from([(paths(), a)])),
    )
}

fn sum(bytes: &[u8], mut initial: u32) -> u16 {
    let (words, tail) = bytes.as_chunks::<2>();
    initial += words
        .iter()
        .map(|word| u32::from(u16::from_be_bytes(*word)))
        .sum::<u32>();
    if let Some(byte) = tail.first() {
        initial += u32::from(*byte) << 8;
    }
    while initial > 0xffff {
        initial = (initial & 0xffff) + (initial >> 16);
    }
    !(initial as u16)
}

fn ipv4(protocol: u8, reply: bool, body: Vec<u8>) -> Vec<u8> {
    let mut packet = vec![0; 20];
    packet[0] = 0x45;
    packet[6] = 0x40;
    packet[8] = 64;
    packet[9] = protocol;
    packet[12..16].copy_from_slice(&[100, 64, 0, if reply { 2 } else { 1 }]);
    packet[16..20].copy_from_slice(&[100, 64, 0, if reply { 1 } else { 2 }]);
    packet[2..4].copy_from_slice(&((20 + body.len()) as u16).to_be_bytes());
    let checksum = sum(&packet, 0);
    packet[10..12].copy_from_slice(&checksum.to_be_bytes());
    packet.extend(body);
    packet
}

fn udp(reply: bool, port: u16) -> Vec<u8> {
    let mut body = vec![0; 8];
    body[..2].copy_from_slice(&(if reply { 53 } else { port }).to_be_bytes());
    body[2..4].copy_from_slice(&(if reply { port } else { 53 }).to_be_bytes());
    body[4..6].copy_from_slice(&8_u16.to_be_bytes());
    ipv4(17, reply, body)
}

fn tcp(reply: bool, seq: u32, ack: u32, flags: u8, data: usize) -> Vec<u8> {
    let mut body = vec![0; 20 + data];
    body[..2].copy_from_slice(&(if reply { 443_u16 } else { 40000 }).to_be_bytes());
    body[2..4].copy_from_slice(&(if reply { 40000_u16 } else { 443 }).to_be_bytes());
    body[4..8].copy_from_slice(&seq.to_be_bytes());
    body[8..12].copy_from_slice(&ack.to_be_bytes());
    body[12] = 0x50;
    body[13] = flags;
    body[14..16].copy_from_slice(&65535_u16.to_be_bytes());
    let pseudo = 2 * (100 * 256 + 64) + 3 + 6 + body.len() as u32;
    let checksum = sum(&body, pseudo);
    body[16..18].copy_from_slice(&checksum.to_be_bytes());
    ipv4(6, reply, body)
}

fn echo(reply: bool, seq: u16) -> Vec<u8> {
    let mut body = vec![0; 8];
    body[0] = if reply { 0 } else { 8 };
    body[4..6].copy_from_slice(&99_u16.to_be_bytes());
    body[6..8].copy_from_slice(&seq.to_be_bytes());
    let checksum = sum(&body, 0);
    body[2..4].copy_from_slice(&checksum.to_be_bytes());
    ipv4(1, reply, body)
}

fn prepared(plan: &StatefulIpv4Plan, bytes: &[u8]) -> ReceivedPacket {
    let path = plan.outgoing(bytes).expect("authorized packet").clone();
    ReceivedPacket {
        source: path[0].clone(),
        payload: plan.encapsulate(&path, bytes).unwrap(),
        path,
    }
}

fn deliver(from: &StatefulIpv4Plan, to: &StatefulIpv4Plan, bytes: &[u8]) {
    let packet = prepared(from, bytes);
    from.sent(&packet.path, &packet.payload).unwrap();
    assert!(to.incoming(&packet));
    assert_eq!(to.plaintext(&packet).unwrap(), bytes);
    to.delivered(&packet).unwrap();
    confirm(from, to, &packet);
}

fn confirm(from: &StatefulIpv4Plan, to: &StatefulIpv4Plan, packet: &ReceivedPacket) {
    let path: Vec<_> = packet.path.iter().rev().cloned().collect();
    assert!(
        from.control(&ReceivedPacket {
            source: path[0].clone(),
            path,
            payload: to.receipt(packet).unwrap(),
        })
        .unwrap()
    );
}

#[test]
fn udp_return_requires_successful_boundaries_and_exact_tuple_token_and_path() {
    let (a, b) = plans();
    let request = udp(false, 40000);
    let reply = udp(true, 40000);
    assert!(b.outgoing(&reply).is_none());
    let packet = prepared(&a, &request);
    assert!(a.table().is_empty());
    assert!(b.incoming(&packet));
    assert!(
        b.table().is_empty(),
        "admission/preparation is not a successful delivery"
    );
    assert!(b.outgoing(&reply).is_none());
    b.delivered(&packet).unwrap();
    let returning = prepared(&b, &reply);
    assert_eq!(returning.path, vec!["b", "via", "a"]);
    assert!(
        !a.incoming(&returning),
        "failed or cancelled source send creates no eligibility"
    );
    a.sent(&packet.path, &packet.payload).unwrap();
    assert!(
        !a.incoming(&returning),
        "queue acceptance is not delivery evidence"
    );
    confirm(&a, &b, &packet);
    assert!(a.incoming(&returning));
    let mut forged = returning;
    forged.payload[5] ^= 1;
    assert!(!a.incoming(&forged));
    forged.payload[5] ^= 1;
    forged.path.remove(1);
    assert!(!a.incoming(&forged));
    assert!(b.outgoing(&udp(true, 40001)).is_none());
    assert!(
        b.outgoing(&udp(false, 40000)).is_none(),
        "spoofed local source denied"
    );
    let (fresh, _) = plans();
    assert!(fresh.table().is_empty());
}

#[test]
fn tcp_requires_syn_synack_and_ack_then_bounds_sequence_and_acknowledgement() {
    let (a, b) = plans();
    assert!(a.outgoing(&tcp(false, 123, 0, 0x10, 0)).is_none());
    assert!(b.outgoing(&tcp(true, 99, 0, 2, 0)).is_none());
    deliver(&a, &b, &tcp(false, 100, 0, 2, 0));
    assert!(b.outgoing(&tcp(true, 700, 999, 0x12, 0)).is_none());
    assert!(b.outgoing(&tcp(true, 700, 101, 0x10, 0)).is_none());
    deliver(&b, &a, &tcp(true, 700, 101, 0x12, 0));
    assert!(a.outgoing(&tcp(false, 101, 999, 0x10, 0)).is_none());
    deliver(&a, &b, &tcp(false, 101, 701, 0x10, 0));
    deliver(&a, &b, &tcp(false, 101, 701, 0x18, 10));
    deliver(&b, &a, &tcp(true, 701, 111, 0x18, 10));
    assert!(b.outgoing(&tcp(true, 999, 111, 0x18, 1)).is_none());
    assert!(b.outgoing(&tcp(true, 711, 999, 0x18, 1)).is_none());
    assert!(b.outgoing(&tcp(true, 711, 111, 2, 0)).is_none());
}

#[test]
fn icmp_echo_exact_sequence_is_single_use_and_unsolicited_echo_reply_cannot_open() {
    let (a, b) = plans();
    deliver(&a, &b, &echo(false, 10));
    assert!(b.outgoing(&echo(true, 11)).is_none());
    deliver(&b, &a, &echo(true, 10));
    assert!(b.outgoing(&echo(true, 10)).is_none());
    assert!(a.outgoing(&echo(true, 10)).is_none());
}

#[test]
fn return_uses_established_forward_waypoints_not_reverse_initiation_route() {
    let (a, mut b) = plans();
    let reverse_initiation = vec!["b".into(), "other-via".into(), "a".into()];
    b.outgoing
        .insert("100.64.0.1".parse().unwrap(), reverse_initiation.clone());
    deliver(&a, &b, &udp(false, 40000));
    assert_eq!(
        b.outgoing(&udp(true, 40000)),
        Some(&vec!["b".into(), "via".into(), "a".into()])
    );
    assert_eq!(b.outgoing(&udp(true, 40001)), Some(&reverse_initiation));
    let packet = prepared(&b, &udp(true, 40000));
    assert!(a.incoming(&packet));
    let wrong_path = ReceivedPacket {
        path: reverse_initiation,
        ..packet
    };
    assert!(!a.incoming(&wrong_path));
}

#[tokio::test(start_paused = true)]
async fn expiry_capacity_restart_and_fragmentation_fail_closed() {
    let (a, b) = plans();
    for port in 1000..1000 + CAPACITY as u16 {
        deliver(&a, &b, &udp(false, port));
    }
    assert!(a.encapsulate(&paths(), &udp(false, 999)).is_err());
    assert_eq!(a.table().len(), CAPACITY);
    let packet = prepared(&b, &udp(true, 1000));
    let (fresh, _) = plans();
    assert!(!fresh.incoming(&packet));
    tokio::time::advance(Duration::from_secs(31)).await;
    assert!(!a.incoming(&packet));
    assert!(b.outgoing(&udp(true, 1000)).is_none());
    assert!(
        a.outgoing(&udp(false, 999)).is_some(),
        "capacity recovers after bounded expiry"
    );
    let mut fragment = udp(false, 999);
    fragment[6] = 0x20;
    fragment[10..12].fill(0);
    let check = sum(&fragment[..20], 0);
    fragment[10..12].copy_from_slice(&check.to_be_bytes());
    assert!(a.outgoing(&fragment).is_none());
}

#[tokio::test(start_paused = true)]
async fn enqueue_forged_or_expired_receipts_and_other_revision_never_create_return_eligibility() {
    let (a, b) = plans();
    let packet = prepared(&a, &udp(false, 40000));
    a.sent(&packet.path, &packet.payload).unwrap();
    assert!(a.table().is_empty());
    let reverse: Vec<_> = packet.path.iter().rev().cloned().collect();
    let mut receipt = ReceivedPacket {
        source: reverse[0].clone(),
        path: reverse,
        payload: b.receipt(&packet).unwrap(),
    };
    receipt.payload[21] ^= 1;
    assert!(a.control(&receipt).unwrap());
    assert!(a.table().is_empty());
    receipt.payload[21] ^= 1;
    receipt.path.remove(1);
    assert!(a.control(&receipt).unwrap());
    assert!(a.table().is_empty());
    receipt.path.insert(1, "via".into());
    tokio::time::advance(Duration::from_secs(6)).await;
    assert!(a.control(&receipt).unwrap());
    assert!(a.table().is_empty());
    assert!(!b.fresh().scoped([1; 32]).incoming(&packet));
    assert!(a.fresh().scoped([1; 32]).control(&receipt).is_ok());
}

#[tokio::test]
async fn encrypted_three_hop_pump_returns_one_way_udp_and_echo_without_reverse_initiation() {
    use crate::packet_io::{self, PacketBatch};
    use crate::{
        Identity, LabConfig, Limits, PeerConfig, PrivateKey, Runtime, StreamRole, TransportKind,
    };
    use tokio::sync::watch;
    let keys: Vec<_> = (0..3).map(|_| Identity::generate().unwrap()).collect();
    let ids = ["a", "via", "b"];
    let path = paths();
    let reverse: Vec<_> = path.iter().rev().cloned().collect();
    let mut runtimes = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        let config = LabConfig {
            node_id: (*id).into(),
            private_key: PrivateKey::from_hex(&keys[i].private_key_hex()).unwrap(),
            public_key: keys[i].public_key_hex(),
            trusted_peers: ids
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(j, id)| PeerConfig {
                    node_id: (*id).into(),
                    public_key: keys[j].public_key_hex(),
                    transports: vec![TransportKind::NativeTcp],
                })
                .collect(),
            allowed_paths: vec![path.clone(), reverse.clone()],
            listeners: vec![],
            outgoing: vec![],
            limits: Limits::default(),
        };
        runtimes.push(Arc::new(Runtime::start(config).await.unwrap()));
    }
    for i in 0..2 {
        let (left, right) = tokio::io::duplex(65536);
        let (a, b) = tokio::join!(
            runtimes[i].attach_stream(
                left,
                StreamRole::Dialer {
                    peer: ids[i + 1].into()
                },
                TransportKind::NativeTcp
            ),
            runtimes[i + 1].attach_stream(right, StreamRole::Acceptor, TransportKind::NativeTcp)
        );
        a.unwrap();
        b.unwrap();
    }
    let (a, b) = plans();
    let observer = a.clone();
    let mut relay = runtimes[1].take_packet_receiver().unwrap();
    let (ad, ac) = packet_io::channel(8).unwrap();
    let (bd, bc) = packet_io::channel(8).unwrap();
    let (lease, deadline) = watch::channel(Some(Instant::now() + Duration::from_secs(30)));
    let ap = tokio::spawn(packet_io::run(ad, runtimes[0].clone(), a, deadline.clone()));
    let bp = tokio::spawn(packet_io::run(bd, runtimes[2].clone(), b, deadline));
    for (request, response) in [
        (udp(false, 40000), udp(true, 40000)),
        (echo(false, 20), echo(true, 20)),
    ] {
        ac.try_feed(PacketBatch::new(vec![request.clone()]).unwrap())
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), bc.recv_batch())
                .await
                .unwrap()
                .unwrap()
                .into_packets(),
            vec![request]
        );
        // Wait for the authenticated delivery receipt, not just the TUN writer.
        tokio::time::timeout(Duration::from_secs(3), async {
            while observer.table().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        bc.try_feed(PacketBatch::new(vec![response.clone()]).unwrap())
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), ac.recv_batch())
                .await
                .unwrap()
                .unwrap()
                .into_packets(),
            vec![response]
        );
    }
    bc.try_feed(PacketBatch::new(vec![udp(true, 55555)]).unwrap())
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), ac.recv_batch())
            .await
            .is_err()
    );
    assert!(
        relay.try_recv().is_err(),
        "relay must never receive business plaintext"
    );
    lease.send_replace(None);
    ap.await.unwrap().unwrap();
    bp.await.unwrap().unwrap();
    for runtime in runtimes {
        runtime.shutdown().await;
    }
}
