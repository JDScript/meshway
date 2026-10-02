use hickory_proto::{
    op::{Message, MessageType, OpCode, Query, ResponseCode},
    rr::{Name, RecordType},
};
use meshway_agent_dns::{Error, Guard, Server, Zone, validate_listen};
use std::{collections::BTreeMap, net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    time::{Instant, timeout},
};

fn name(value: &str) -> Name {
    Name::from_ascii(value).unwrap()
}

fn active() -> Guard {
    let guard = Guard::default();
    assert!(guard.publish_snapshot(
        Zone {
            name: name("meshway.test."),
            ttl: 30,
            forward: BTreeMap::from([(
                name("demo-sha.meshway.test."),
                ("100.64.0.2".parse().unwrap(), "fd00::2".parse().unwrap()),
            )]),
            reverse: BTreeMap::from([(
                name("2.0.64.100.in-addr.arpa."),
                name("demo-sha.meshway.test."),
            )]),
        },
        Instant::now() + Duration::from_secs(45),
        1,
        true,
        None,
        None,
    ));
    guard
}

fn query(owner: &str, kind: RecordType) -> Message {
    let mut query = Message::new(12345, MessageType::Query, OpCode::Query);
    query.metadata.recursion_desired = true;
    query.add_query(Query::query(name(owner), kind));
    query
}

async fn bind() -> (Server, SocketAddr) {
    // The public bind API takes a fixed port. Retry only local reservation races.
    for _ in 0..16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        if let Ok(server) = Server::bind(address).await {
            return (server, address);
        }
    }
    panic!("could not reserve a loopback port for both DNS protocols");
}

#[tokio::test]
async fn actual_udp_tcp_queries_withdrawal_and_socket_cleanup() {
    let (server, address) = bind().await;
    let guard = active();
    let service = tokio::spawn(server.serve(guard.clone()));
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
        let mut buffer = [0; 4096];
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
    oversized.write_u16(4097).await.unwrap();
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
    let mut buffer = [0; 4096];
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
    drop(Server::bind(address).await.unwrap());
}

#[tokio::test]
async fn either_bind_failure_closes_partial_startup_and_ipv6_loopback_serves() {
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = tcp.local_addr().unwrap();
    assert!(matches!(Server::bind(address).await, Err(Error::Io(_))));
    let udp = UdpSocket::bind(address).await.unwrap();
    drop(tcp);
    assert!(matches!(Server::bind(address).await, Err(Error::Io(_))));
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
    let mut buffer = [0; 4096];
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
        assert!(matches!(
            validate_listen(address.parse().unwrap()),
            Err(Error::InvalidListen)
        ));
    }
    for address in ["127.0.0.1:5353", "[::1]:5353"] {
        validate_listen(address.parse().unwrap()).unwrap();
    }
}
