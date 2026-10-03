//! Positive regressions for the nine independently reproduced review probes.
use super::*;

#[test]
fn lost_initial_receipt_keeps_attempt_token_and_reacknowledges_tcp_retransmission() {
    let (a, b) = plans();
    let syn = tcp(false, 100, 0, 2, 0);
    let first = prepared(&a, &syn);
    a.sent(&first.path, &first.payload).unwrap();
    assert!(b.incoming(&first));
    b.delivered(&first).unwrap();
    // Drop the receipt. An early SYNACK proves delivery itself; no receiver wait.
    assert!(a.incoming(&prepared(&b, &tcp(true, 700, 101, 0x12, 0))));
    let retry = prepared(&a, &syn);
    assert_eq!(&first.payload[5..37], &retry.payload[5..37]);
    a.sent(&retry.path, &retry.payload).unwrap();
    assert!(b.incoming(&retry));
    b.delivered(&retry).unwrap();
    confirm(&a, &b, &retry);
    assert!(a.table().flows.values().all(|f| f.active));
}

#[test]
fn concurrent_udp_before_first_receipt_has_one_stable_attempt() {
    let (a, b) = plans();
    let first = prepared(&a, &udp(false, 40000));
    a.sent(&first.path, &first.payload).unwrap();
    let second = prepared(&a, &udp(false, 40000));
    a.sent(&second.path, &second.payload).unwrap();
    assert_eq!(&first.payload[5..37], &second.payload[5..37]);
    assert!(b.incoming(&first));
    b.delivered(&first).unwrap();
    assert!(b.incoming(&second));
    b.delivered(&second).unwrap();
    assert_eq!(a.table().flows.len(), 1);
    assert!(a.table().flows.values().all(|f| !f.active));
}

#[test]
fn authenticated_reply_overtaking_receipt_carries_matching_proof() {
    let (a, b) = plans();
    let request = prepared(&a, &udp(false, 40000));
    a.sent(&request.path, &request.payload).unwrap();
    assert!(b.incoming(&request));
    b.delivered(&request).unwrap();
    let reply = prepared(&b, &udp(true, 40000));
    let mut forged = ReceivedPacket {
        source: reply.source.clone(),
        path: reply.path.clone(),
        payload: reply.payload.clone(),
    };
    forged.payload[21] ^= 1;
    assert!(!a.incoming(&forged));
    assert!(a.incoming(&reply));
    a.delivered(&reply).unwrap();
    assert!(a.table().flows.values().all(|f| f.active));
}

fn handshake(a: &StatefulIpv4Plan, b: &StatefulIpv4Plan) {
    deliver(a, b, &tcp(false, 100, 0, 2, 0));
    deliver(b, a, &tcp(true, 700, 101, 0x12, 0));
    deliver(a, b, &tcp(false, 101, 701, 0x10, 0));
}

#[test]
fn negotiated_window_allows_large_pipeline_reorder_and_retransmission() {
    fn syn(reply: bool, seq: u32, ack: u32, flags: u8) -> Vec<u8> {
        let mut bytes = tcp(reply, seq, ack, flags, 0);
        bytes.extend_from_slice(&[3, 3, 7, 1]);
        bytes[32] = 0x60;
        bytes[2..4].copy_from_slice(&44_u16.to_be_bytes());
        bytes[10..12].fill(0);
        let ip_sum = sum(&bytes[..20], 0);
        bytes[10..12].copy_from_slice(&ip_sum.to_be_bytes());
        bytes[36..38].fill(0);
        let pseudo = 2 * (100 * 256 + 64) + 3 + 6 + 24;
        let tcp_sum = sum(&bytes[20..], pseudo);
        bytes[36..38].copy_from_slice(&tcp_sum.to_be_bytes());
        bytes
    }
    let (a, b) = plans();
    deliver(&a, &b, &syn(false, 100, 0, 2));
    deliver(&b, &a, &syn(true, 700, 101, 0x12));
    deliver(&a, &b, &tcp(false, 101, 701, 0x10, 0));
    deliver(&b, &a, &tcp(true, 701, 101, 0x10, 0));
    let first = prepared(&a, &tcp(false, 101, 701, 0x18, 1000));
    a.sent(&first.path, &first.payload).unwrap();
    let mut last = None;
    for i in 1..2000 {
        let packet = prepared(&a, &tcp(false, 101 + i * 1000, 701, 0x18, 1000));
        a.sent(&packet.path, &packet.payload).unwrap();
        last = Some(packet);
    }
    let last = last.unwrap();
    assert!(b.incoming(&last));
    b.delivered(&last).unwrap();
    assert!(b.incoming(&first));
    b.delivered(&first).unwrap();
    deliver(&b, &a, &tcp(true, 701, 1101, 0x10, 0));
    assert!(a.outgoing(&tcp(false, 101, 701, 0x18, 1000)).is_some());
}

#[test]
fn echo_completion_does_not_destroy_an_inflight_reservation_or_delivery() {
    let (a, b) = plans();
    deliver(&a, &b, &echo(false, 42));
    let path = paths();
    let pending = a.encapsulate(&path, &echo(false, 42)).unwrap();
    let reply = prepared(&b, &echo(true, 42));
    assert!(a.incoming(&reply));
    a.delivered(&reply).unwrap();
    a.sent(&path, &pending).unwrap();
    assert!(a.table().flows.is_empty());

    let (a, b) = plans();
    deliver(&a, &b, &echo(false, 43));
    let again = prepared(&a, &echo(false, 43));
    a.sent(&again.path, &again.payload).unwrap();
    assert!(b.incoming(&again));
    let reply = prepared(&b, &echo(true, 43));
    b.sent(&reply.path, &reply.payload).unwrap();
    b.delivered(&again).unwrap();
    assert!(b.outgoing(&echo(true, 43)).is_some());
}

#[test]
fn tcp_pipeline_and_cumulative_ack_do_not_wait_for_per_packet_receipts() {
    let (a, b) = plans();
    handshake(&a, &b);
    let first = prepared(&a, &tcp(false, 101, 701, 0x18, 10));
    a.sent(&first.path, &first.payload).unwrap();
    let second = prepared(&a, &tcp(false, 111, 701, 0x18, 10));
    a.sent(&second.path, &second.payload).unwrap();
    // Network reorder is legitimate inside the bounded sequence window.
    assert!(b.incoming(&second));
    b.delivered(&second).unwrap();
    assert!(b.incoming(&first));
    b.delivered(&first).unwrap();
    let ack = prepared(&b, &tcp(true, 701, 121, 0x10, 0));
    b.sent(&ack.path, &ack.payload).unwrap();
    assert!(a.incoming(&ack));
    a.delivered(&ack).unwrap();
    assert!(a.outgoing(&tcp(false, 121, 701, 0x18, 10)).is_some());
    assert!(a.next_control().is_none());
}

#[test]
fn cancelled_carrier_send_preserves_provisional_pipeline_without_terminal_commit() {
    let (a, b) = plans();
    handshake(&a, &b);
    let path = paths();
    let first = a
        .encapsulate(&path, &tcp(false, 101, 701, 0x18, 10))
        .unwrap();
    a.cancelled(&path, &first);
    let next = prepared(&a, &tcp(false, 111, 701, 0x18, 10));
    a.sent(&next.path, &next.payload).unwrap();
    assert!(b.incoming(&next));
    b.delivered(&next).unwrap();
    assert!(a.outgoing(&tcp(false, 101, 701, 0x18, 10)).is_some());
    let fin = a
        .encapsulate(&path, &tcp(false, 121, 701, 0x11, 0))
        .unwrap();
    a.cancelled(&path, &fin);
    assert!(a.table().flows.values().all(|f| !f.tcp.unwrap().terminal()));
}

#[test]
fn more_than_1024_unconfirmed_packets_on_one_flow_consume_one_slot() {
    let (a, b) = plans();
    deliver(&a, &b, &udp(false, 40000));
    for _ in 0..CAPACITY + 1 {
        let packet = prepared(&a, &udp(false, 40000));
        a.sent(&packet.path, &packet.payload).unwrap();
    }
    assert_eq!(a.table().flows.len(), 1);
    assert_eq!(a.statistics().capacity_drops, 0);
}

#[tokio::test(start_paused = true)]
async fn continuous_tcp_has_no_absolute_five_minute_retirement() {
    let (a, b) = plans();
    handshake(&a, &b);
    for i in 0..5 {
        tokio::time::advance(Duration::from_secs(59)).await;
        deliver(&a, &b, &tcp(false, 101 + i, 701, 0x18, 1));
    }
    tokio::time::advance(Duration::from_secs(6)).await;
    deliver(&a, &b, &tcp(false, 106, 701, 0x18, 1));
    deliver(&b, &a, &tcp(true, 701, 107, 0x10, 0));
}

#[tokio::test(start_paused = true)]
async fn one_fin_keeps_opposite_half_open_past_six_seconds() {
    let (a, b) = plans();
    handshake(&a, &b);
    deliver(&a, &b, &tcp(false, 101, 701, 0x11, 0));
    tokio::time::advance(Duration::from_secs(6)).await;
    deliver(&b, &a, &tcp(true, 701, 102, 0x18, 1));
    deliver(&b, &a, &tcp(true, 702, 102, 0x11, 0));
    deliver(&a, &b, &tcp(false, 102, 703, 0x10, 0));
    tokio::time::advance(Duration::from_secs(6)).await;
    assert!(a.outgoing(&tcp(false, 102, 703, 0x18, 1)).is_none());
}

#[tokio::test(start_paused = true)]
async fn live_udp_reply_stays_on_original_path_after_five_minutes() {
    let (mut a, mut b) = plans();
    let other = vec!["b".into(), "other-via".into(), "a".into()];
    b.outgoing
        .insert("100.64.0.1".parse().unwrap(), other.clone());
    a.incoming.insert(other, "100.64.0.2".parse().unwrap());
    deliver(&a, &b, &udp(false, 40000));
    for _ in 0..15 {
        tokio::time::advance(Duration::from_secs(19)).await;
        deliver(&a, &b, &udp(false, 40000));
    }
    tokio::time::advance(Duration::from_secs(16)).await;
    let reply = prepared(&b, &udp(true, 40000));
    assert_eq!(reply.path, vec!["b", "via", "a"]);
    assert_eq!(reply.payload[4], REPLY);
    assert!(a.incoming(&reply));
    a.delivered(&reply).unwrap();
}

#[test]
fn reverse_grant_cannot_relabel_live_reply_tuple_with_fresh_peer_token() {
    let (mut a, b) = plans();
    let other = vec!["b".into(), "other-via".into(), "a".into()];
    a.incoming.insert(other.clone(), b.local);
    deliver(&a, &b, &udp(false, 40000));
    // Model a peer bypassing its honest outgoing classifier and declaring an
    // independently authorized initiation with the old response's exact tuple.
    let independent = StatefulIpv4Plan::new(
        b.local,
        1200,
        BTreeMap::from([(a.local, other)]),
        BTreeMap::new(),
    );
    let forged = prepared(&independent, &udp(true, 40000));
    assert_eq!(forged.payload[4], FORWARD);
    assert!(!a.incoming(&forged));
    assert!(a.incoming(&prepared(&b, &udp(true, 40000))));
    assert!(
        a.incoming(&prepared(&independent, &udp(true, 40001))),
        "a genuinely distinct authorized flow remains permitted"
    );
}

#[test]
fn independently_authorized_echo_requests_with_equal_ids_are_not_replies() {
    fn swap(mut bytes: Vec<u8>) -> Vec<u8> {
        let source: [u8; 4] = bytes[12..16].try_into().unwrap();
        let destination: [u8; 4] = bytes[16..20].try_into().unwrap();
        bytes[12..16].copy_from_slice(&destination);
        bytes[16..20].copy_from_slice(&source);
        bytes
    }
    let (mut a, mut b) = plans();
    let other: Vec<String> = vec!["b".into(), "other-via".into(), "a".into()];
    a.incoming.insert(other.clone(), b.local);
    a.returns
        .insert(b.local, other.iter().rev().cloned().collect());
    b.outgoing.insert(a.local, other.clone());
    let first = prepared(&a, &echo(false, 42));
    a.sent(&first.path, &first.payload).unwrap();
    assert!(b.incoming(&first));
    b.delivered(&first).unwrap();
    let independent = prepared(&b, &swap(echo(false, 42)));
    assert_eq!(independent.path, other);
    b.sent(&independent.path, &independent.payload).unwrap();
    assert!(a.incoming(&independent));
    a.delivered(&independent).unwrap();
    deliver(&b, &a, &echo(true, 42));
    deliver(&a, &b, &swap(echo(true, 42)));
}

#[tokio::test]
async fn encrypted_three_hop_capacity_drop_precedes_enqueue_and_preserves_other_peer() {
    use crate::packet_io::{self, PacketBatch};
    use tokio::sync::watch;
    let runtimes = encrypted_nodes().await;
    let (mut a, _) = plans();
    let good_ip = "100.64.0.9".parse().unwrap();
    a.outgoing.insert(good_ip, vec!["a".into(), "via".into()]);
    let observer = a.clone();
    let good = StatefulIpv4Plan::new(
        good_ip,
        1200,
        BTreeMap::new(),
        BTreeMap::from([(vec!["a".into(), "via".into()], a.local)]),
    );
    let (device, endpoint) = packet_io::channel(8).unwrap();
    let (gd, ge) = packet_io::channel(8).unwrap();
    let (lease, deadline) = watch::channel(Some(Instant::now() + Duration::from_secs(30)));
    let pump = tokio::spawn(packet_io::run(
        device,
        runtimes[0].clone(),
        a,
        deadline.clone(),
    ));
    let gp = tokio::spawn(packet_io::run(gd, runtimes[1].clone(), good, deadline));
    let mut remote = runtimes[2].take_packet_receiver().unwrap();
    fn good_packet(reply: bool) -> Vec<u8> {
        let mut p = udp(reply, 60000);
        p[if reply { 15 } else { 19 }] = 9;
        p[10..12].fill(0);
        let checksum = sum(&p[..20], 0);
        p[10..12].copy_from_slice(&checksum.to_be_bytes());
        p
    }
    async fn exchange(endpoint: &packet_io::ChannelEndpoint, good: &packet_io::ChannelEndpoint) {
        endpoint
            .try_feed(PacketBatch::new(vec![good_packet(false)]).unwrap())
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), good.recv_batch())
            .await
            .unwrap()
            .unwrap();
        good.try_feed(PacketBatch::new(vec![good_packet(true)]).unwrap())
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), endpoint.recv_batch())
                .await
                .unwrap()
                .unwrap()
                .into_packets(),
            vec![good_packet(true)]
        );
    }
    exchange(&endpoint, &ge).await;
    // Withhold all B receipts, including the first. Same-flow traffic remains usable.
    for _ in 0..CAPACITY + 1 {
        endpoint
            .try_feed(PacketBatch::new(vec![udp(false, 40000)]).unwrap())
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), remote.recv())
            .await
            .unwrap()
            .unwrap();
    }
    for port in 1000..1000 + (PEER_CAPACITY - 1) as u16 {
        endpoint
            .try_feed(PacketBatch::new(vec![udp(false, port)]).unwrap())
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), remote.recv())
            .await
            .unwrap()
            .unwrap();
    }
    endpoint
        .try_feed(PacketBatch::new(vec![udp(false, 999)]).unwrap())
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while observer.statistics().capacity_drops == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(30), remote.recv())
            .await
            .is_err()
    );
    assert!(!pump.is_finished());
    exchange(&endpoint, &ge).await;
    lease.send_replace(None);
    pump.await.unwrap().unwrap();
    gp.await.unwrap().unwrap();
    for runtime in runtimes {
        runtime.shutdown().await;
    }
}
