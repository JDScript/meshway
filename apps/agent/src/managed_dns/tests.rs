use super::*;
use hickory_proto::op::Query;
use tokio::io::AsyncWriteExt;

fn name(value: &str) -> Name {
    Name::from_ascii(value).unwrap()
}

fn active() -> Guard {
    let guard = Guard::default();
    guard.publish(
        Zone {
            name: name("meshway.test."),
            ttl: 30,
            forward: BTreeMap::from([
                (
                    name("demo-sha.meshway.test."),
                    (Ipv4Addr::new(100, 64, 0, 2), "fd00::2".parse().unwrap()),
                ),
                (
                    name("id.aaaa.bbbb.meshway.test."),
                    (Ipv4Addr::new(100, 64, 0, 2), "fd00::2".parse().unwrap()),
                ),
            ]),
            reverse: BTreeMap::from([
                (
                    name("2.0.64.100.in-addr.arpa."),
                    name("id.aaaa.bbbb.meshway.test."),
                ),
                (
                    Name::from("fd00::2".parse::<Ipv6Addr>().unwrap()),
                    name("id.aaaa.bbbb.meshway.test."),
                ),
            ]),
        },
        Instant::now() + Duration::from_secs(45),
        1,
    );
    guard
}

fn query(owner: &str, kind: RecordType) -> Message {
    let mut query = Message::new(12345, MessageType::Query, OpCode::Query);
    query.metadata.recursion_desired = true;
    query.add_query(Query::query(name(owner), kind));
    query
}

fn response(guard: &Guard, query: &Message) -> Message {
    Message::from_vec(&guard.respond(&query.to_vec().unwrap(), false).unwrap()).unwrap()
}

#[test]
fn authoritative_bounds_case_reverse_and_nodata() {
    let guard = active();
    for owner in [
        "demo-sha.meshway.test",
        "DEMO-SHA.MESHWAY.TEST.",
        "id.aaaa.bbbb.meshway.test.",
    ] {
        for kind in [RecordType::A, RecordType::AAAA] {
            let request = query(owner, kind);
            let reply = response(&guard, &request);
            assert_eq!(reply.metadata.id, request.metadata.id);
            assert_eq!(
                reply.queries,
                Message::from_vec(&request.to_vec().unwrap())
                    .unwrap()
                    .queries
            );
            assert!(!reply.metadata.recursion_available);
            assert!(reply.metadata.authoritative);
            assert_eq!(reply.answers.len(), 1);
            assert_eq!(reply.answers[0].record_type(), kind);
            assert_eq!(reply.answers[0].ttl, 30);
        }
    }
    for owner in [
        "2.0.64.100.in-addr.arpa.".to_owned(),
        Name::from("fd00::2".parse::<Ipv6Addr>().unwrap()).to_string(),
    ] {
        let reply = response(&guard, &query(&owner, RecordType::PTR));
        assert_eq!(
            reply.answers[0].data,
            RData::PTR(PTR(name("id.aaaa.bbbb.meshway.test.")))
        );
    }
    for owner in [
        "3.0.64.100.in-addr.arpa.",
        "example.org.",
        "badmeshway.test.",
        "meshway.test.example.org.",
    ] {
        let reply = response(&guard, &query(owner, RecordType::A));
        assert_eq!(reply.metadata.response_code, ResponseCode::Refused);
        assert!(reply.answers.is_empty());
        assert!(!reply.metadata.authoritative);
    }
    assert_eq!(
        response(&guard, &query("unknown.meshway.test.", RecordType::A))
            .metadata
            .response_code,
        ResponseCode::NXDomain
    );
    let nodata = response(&guard, &query("demo-sha.meshway.test.", RecordType::TXT));
    assert_eq!(nodata.metadata.response_code, ResponseCode::NoError);
    assert!(nodata.answers.is_empty());
    for kind in [RecordType::AXFR, RecordType::IXFR, RecordType::ANY] {
        assert_eq!(
            response(&guard, &query("meshway.test.", kind))
                .metadata
                .response_code,
            ResponseCode::Refused
        );
    }
}

#[test]
fn expired_withdrawn_disabled_and_fractional_lease_never_answer() {
    let guard = active();
    let request = query("demo-sha.meshway.test.", RecordType::A);
    let start = Instant::now();
    let soon = guard.answer(&request, start + Duration::from_secs(44));
    assert!(soon.answers.is_empty()); // Less than one second cannot announce TTL 1.
    let expired = guard.answer(&request, start + Duration::from_secs(46));
    assert_eq!(expired.metadata.response_code, ResponseCode::ServFail);
    guard.withdraw();
    assert!(response(&guard, &request).answers.is_empty());
    let guard = active();
    guard.disable();
    assert_eq!(
        response(&guard, &request).metadata.response_code,
        ResponseCode::Refused
    );
}

#[cfg(unix)]
fn leased_disabled() -> Guard {
    let guard = Guard::default();
    guard.listener_bound("127.0.0.1:5353".parse().unwrap(), None);
    assert!(guard.publish_snapshot(
        Zone {
            name: name("meshway.test."),
            ttl: 30,
            forward: BTreeMap::new(),
            reverse: BTreeMap::new(),
        },
        Instant::now() + Duration::from_secs(45),
        1,
        false,
        None,
        None,
    ));
    guard
}

#[cfg(unix)]
#[tokio::test(start_paused = true)]
async fn disabled_snapshot_newer_revision_withdraws_status_answers_and_generation() {
    use crate::diagnostics::dns::State as DnsState;
    let guard = leased_disabled();
    let before = guard.status();
    let generation = guard.0.read().unwrap().generation.clone();
    assert_eq!(before.state, DnsState::Disabled);
    assert_eq!(before.active_revision, None);
    assert!(before.lease_remaining_ms > 0);
    let request = query("demo-sha.meshway.test.", RecordType::A);
    assert_eq!(
        response(&guard, &request).metadata.response_code,
        ResponseCode::Refused
    );
    guard.observe_revision(2);
    let withdrawn = guard.status();
    assert_eq!(withdrawn.state, DnsState::Unavailable);
    assert_eq!(withdrawn.revision_floor, 2);
    assert!(withdrawn.generation > before.generation);
    assert!(!Arc::ptr_eq(
        &generation,
        &guard.0.read().unwrap().generation
    ));
    assert_eq!(withdrawn.lease_remaining_ms, 0);
    assert_eq!(withdrawn.zone, before.zone);
    assert_eq!(
        response(&guard, &request).metadata.response_code,
        ResponseCode::ServFail
    );
}

#[cfg(unix)]
#[tokio::test(start_paused = true)]
async fn disabled_snapshot_expires_before_watchdog_and_can_recover_only_with_fresh_publication() {
    use crate::diagnostics::dns::State as DnsState;
    let guard = leased_disabled();
    let before = guard.status();
    let generation = guard.0.read().unwrap().generation.clone();
    let request = query("demo-sha.meshway.test.", RecordType::A);
    // Match the whole-second positive/disabled answer fence before the watchdog runs.
    tokio::time::advance(Duration::from_millis(44_500)).await;
    assert_eq!(guard.status().state, DnsState::Expired);
    assert_eq!(
        response(&guard, &request).metadata.response_code,
        ResponseCode::ServFail
    );
    assert!(guard.expire(Instant::now()));
    let expired = guard.status();
    assert_eq!(expired.state, DnsState::Expired);
    assert_eq!(expired.active_revision, None);
    assert_eq!(expired.lease_remaining_ms, 0);
    assert!(expired.generation > before.generation);
    assert!(!Arc::ptr_eq(
        &generation,
        &guard.0.read().unwrap().generation
    ));
    assert!(!guard.expire(Instant::now() + Duration::from_secs(1)));
    assert!(guard.publish_snapshot(
        Zone {
            name: name("meshway.test."),
            ttl: 30,
            forward: BTreeMap::new(),
            reverse: BTreeMap::new(),
        },
        Instant::now() + Duration::from_secs(10),
        2,
        false,
        None,
        None,
    ));
    assert_eq!(guard.status().state, DnsState::Disabled);
    assert_eq!(guard.status().active_revision, None);
    assert_eq!(
        response(&guard, &request).metadata.response_code,
        ResponseCode::Refused
    );
}

#[test]
fn malformed_compression_opcodes_multiple_questions_edns_and_tc() {
    let guard = active();
    let mut request = query("demo-sha.meshway.test.", RecordType::A);
    request.add_query(Query::query(name("example.org."), RecordType::A));
    assert_eq!(
        response(&guard, &request).metadata.response_code,
        ResponseCode::FormErr
    );
    request.queries.truncate(1);
    request.metadata.op_code = OpCode::Update;
    assert_eq!(
        response(&guard, &request).metadata.response_code,
        ResponseCode::NotImp
    );
    request.metadata.op_code = OpCode::Query;
    let mut edns = Edns::new();
    edns.set_max_payload(65535);
    request.edns = Some(edns);
    assert_eq!(
        response(&guard, &request).edns.unwrap().max_payload(),
        MAX_UDP as u16
    );
    let valid = request.to_vec().unwrap();
    assert!(guard.respond(&valid[..valid.len() - 1], false).is_none());
    let mut extra = valid;
    extra.push(0);
    assert!(guard.respond(&extra, false).is_none());
    assert!(guard.respond(&vec![0; MAX_UDP + 1], false).is_none());
    // Hickory rejects a QNAME compression pointer back to itself.
    assert!(
        guard
            .respond(
                &[0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0xc0, 0x0c, 0, 1, 0, 1],
                false
            )
            .is_none()
    );
    let mut large = Message::response(5, OpCode::Query);
    large.add_query(Query::query(name("demo-sha.meshway.test."), RecordType::A));
    for _ in 0..60 {
        large.add_answer(Record::from_rdata(
            name("demo-sha.meshway.test."),
            30,
            RData::A(A(Ipv4Addr::LOCALHOST)),
        ));
    }
    let limited = encode_bounded(large, 512).unwrap();
    assert!(limited.len() <= 512);
    let decoded = Message::from_vec(&limited).unwrap();
    assert!(decoded.metadata.truncation);
    assert!(decoded.answers.is_empty());
    assert_eq!(decoded.metadata.id, 5);
    assert_eq!(decoded.queries.len(), 1);
}

async fn bind() -> Server {
    // Reserve a TCP port then retry binding both protocols if another local test
    // raced us. No system DNS or route configuration is involved.
    loop {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        if let Ok(server) = Server::bind(address).await {
            return server;
        }
    }
}

#[tokio::test]
async fn actual_udp_tcp_queries_withdrawal_and_socket_cleanup() {
    let server = bind().await;
    let address = server.udp.local_addr().unwrap();
    let guard = active();
    let service_guard = guard.clone();
    let service = tokio::spawn(server.serve(service_guard));
    let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    udp.connect(address).await.unwrap();
    for kind in [RecordType::A, RecordType::AAAA, RecordType::PTR] {
        let owner = if kind == RecordType::PTR {
            "2.0.64.100.in-addr.arpa."
        } else {
            "DEMO-SHA.MESHWAY.TEST."
        };
        let wire = query(owner, kind).to_vec().unwrap();
        udp.send(&wire).await.unwrap();
        let mut buffer = [0; MAX_UDP];
        let length = timeout(Duration::from_secs(2), udp.recv(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            Message::from_vec(&buffer[..length]).unwrap().answers.len(),
            1
        );
        let mut tcp = TcpStream::connect(address).await.unwrap();
        tcp.write_u16(wire.len() as u16).await.unwrap();
        tcp.write_all(&wire).await.unwrap();
        let length = timeout(Duration::from_secs(2), tcp.read_u16())
            .await
            .unwrap()
            .unwrap();
        let mut buffer = vec![0; usize::from(length)];
        tcp.read_exact(&mut buffer).await.unwrap();
        assert_eq!(Message::from_vec(&buffer).unwrap().answers.len(), 1);
    }
    let mut oversized = TcpStream::connect(address).await.unwrap();
    oversized.write_u16((MAX_REQUEST + 1) as u16).await.unwrap();
    assert_eq!(
        timeout(Duration::from_secs(2), oversized.read_u8())
            .await
            .unwrap()
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    guard.withdraw();
    udp.send(
        &query("demo-sha.meshway.test.", RecordType::A)
            .to_vec()
            .unwrap(),
    )
    .await
    .unwrap();
    let mut buffer = [0; MAX_UDP];
    let length = timeout(Duration::from_secs(2), udp.recv(&mut buffer))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Message::from_vec(&buffer[..length])
            .unwrap()
            .metadata
            .response_code,
        ResponseCode::ServFail
    );
    service.abort();
    assert!(service.await.unwrap_err().is_cancelled());
    let rebound = Server::bind(address).await.unwrap();
    drop(rebound);
}

#[tokio::test]
async fn either_bind_failure_closes_partial_startup_and_ipv6_loopback_serves() {
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = tcp.local_addr().unwrap();
    assert!(Server::bind(address).await.is_err());
    let udp = UdpSocket::bind(address).await.unwrap();
    drop(tcp);
    assert!(Server::bind(address).await.is_err());
    let tcp = TcpListener::bind(address).await.unwrap();
    drop((tcp, udp));

    let reservation = TcpListener::bind("[::1]:0").await.unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    let server = Server::bind(address).await.unwrap();
    let service = tokio::spawn(server.serve(active()));
    let udp = UdpSocket::bind("[::1]:0").await.unwrap();
    udp.send_to(
        &query("demo-sha.meshway.test.", RecordType::A)
            .to_vec()
            .unwrap(),
        address,
    )
    .await
    .unwrap();
    let mut buffer = [0; MAX_UDP];
    let (length, _) = timeout(Duration::from_secs(2), udp.recv_from(&mut buffer))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Message::from_vec(&buffer[..length]).unwrap().answers.len(),
        1
    );
    service.abort();
    assert!(service.await.unwrap_err().is_cancelled());
}

#[test]
fn only_unprivileged_loopback_listeners() {
    for address in [
        "0.0.0.0:5353",
        "192.0.2.1:5353",
        "127.0.0.1:53",
        "[::]:5353",
    ] {
        assert!(validate_listen(address.parse().unwrap()).is_err());
    }
    for address in ["127.0.0.1:5353", "[::1]:5353"] {
        validate_listen(address.parse().unwrap()).unwrap();
    }
}
