use super::*;
use crate::{
    DnsFuture, GatewayError, LocalResolverRoute, OwnerDispatcher, ResolverExchange, Scope,
};
use hickory_proto::{
    op::{Message, MessageType, OpCode, Query as Question},
    rr::{Name, RecordType},
};

struct NoUpstream;
impl ResolverExchange for NoUpstream {
    fn exchange<'a>(
        &'a self,
        _: &'a LocalResolverRoute,
        _: Bytes,
    ) -> DnsFuture<'a, Result<Bytes, GatewayError>> {
        Box::pin(async { panic!("MagicDNS must use the existing local owner") })
    }
}
fn query(id: u16) -> Vec<u8> {
    let mut query = Message::new(id, MessageType::Query, OpCode::Query);
    query.add_query(Question::query(
        Name::from_ascii("n1.perf.internal.").unwrap(),
        RecordType::A,
    ));
    query.to_vec().unwrap()
}
fn positive(bytes: &[u8], id: u16) {
    let answer = Message::from_vec(bytes).unwrap();
    assert_eq!(answer.metadata.id, id);
    assert_eq!(answer.metadata.response_code, ResponseCode::NoError);
    assert_eq!(answer.answers.len(), 1);
    assert_eq!(answer.answers[0].ttl, 0);
    assert!(answer.answers[0].to_string().contains("100.64.0.2"));
}
struct Client {
    device: device::Queue,
    interface: Interface,
    sockets: SocketSet<'static>,
    udp: SocketHandle,
    tcp: SocketHandle,
    started: Instant,
}
impl Client {
    fn new(server: Ipv4Addr) -> Self {
        let mut device = device::Queue {
            input: VecDeque::new(),
            output: VecDeque::new(),
            mtu: 1280,
        };
        let mut config = Config::new(HardwareAddress::Ip);
        config.random_seed = 9;
        let mut interface = Interface::new(config, &mut device, smoltcp::time::Instant::ZERO);
        interface.update_ip_addrs(|addresses| {
            addresses
                .push(IpCidr::new(Ipv4Addr::new(100, 64, 0, 1).into(), 0))
                .unwrap();
        });
        let mut sockets = SocketSet::new(Vec::new());
        let buf = || udp::PacketBuffer::new(vec![udp::PacketMetadata::EMPTY; 8], vec![0; 16384]);
        let mut udp = udp::Socket::new(buf(), buf());
        udp.bind(35001).unwrap();
        let udp = sockets.add(udp);
        let mut tcp = tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0; 16384]),
            tcp::SocketBuffer::new(vec![0; 16384]),
        );
        tcp.set_nagle_enabled(false);
        tcp.connect(interface.context(), (server, 53), 35002)
            .unwrap();
        let tcp = sockets.add(tcp);
        Self {
            device,
            interface,
            sockets,
            udp,
            tcp,
            started: Instant::now(),
        }
    }
    fn poll(&mut self, port: &Port) {
        self.interface.poll(
            smoltcp::time::Instant::from_millis(self.started.elapsed().as_millis() as i64),
            &mut self.device,
            &mut self.sockets,
        );
        while let Some(packet) = self.device.output.pop_front() {
            port.submit(packet);
        }
    }
    async fn advance(&mut self, port: &Port) {
        self.poll(port);
        tokio::select! {
            packet = port.recv() => {
                let packet = packet.unwrap(); packet.ensure_current().unwrap();
                self.device.input.push_back(packet.bytes().to_vec());
            },
            _ = tokio::time::sleep(Duration::from_millis(2)) => {},
        }
    }
    async fn udp_answer(&mut self, port: &Port) -> Vec<u8> {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok((bytes, _)) = self.sockets.get_mut::<udp::Socket>(self.udp).recv() {
                    return bytes.to_vec();
                }
                self.advance(port).await;
            }
        })
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn udp_tcp_and_queued_answer_revocation_use_the_same_owner_dispatcher() {
    let fixture = meshway_desired_fixture::fixture::Chain::new(2).unwrap();
    let desired = fixture.authority.desired(fixture.path[0]).unwrap();
    let (scopes, received) =
        tokio::sync::watch::channel(Some(Scope::netmap(desired.clone(), None)));
    let dispatcher = Arc::new(
        OwnerDispatcher::new(
            received,
            Arc::new(NoUpstream),
            crate::Limits {
                max_pending: 128,
                query_timeout: Duration::from_secs(5),
            },
            1232,
        )
        .unwrap(),
    );
    let address = Ipv4Addr::new(198, 18, 255, 254);
    let (port, transport) = channel(address, 1280).unwrap();
    let stop = CancellationToken::new();
    let running = tokio::spawn(
        Service::new(dispatcher, Default::default())
            .unwrap()
            .run_packets(transport, stop.clone()),
    );
    let mut client = Client::new(address);
    client
        .sockets
        .get_mut::<udp::Socket>(client.udp)
        .send_slice(&query(1), (address, 53))
        .unwrap();
    positive(&client.udp_answer(&port).await, 1);
    tokio::time::timeout(Duration::from_secs(3), async {
        while !client.sockets.get::<tcp::Socket>(client.tcp).can_send() {
            client.advance(&port).await;
        }
    })
    .await
    .unwrap();
    let mut framed = Vec::new();
    for id in [2, 3] {
        let query = query(id);
        framed.extend_from_slice(&(query.len() as u16).to_be_bytes());
        framed.extend(query);
    }
    client
        .sockets
        .get_mut::<tcp::Socket>(client.tcp)
        .send_slice(&framed)
        .unwrap();
    let mut bytes = Vec::new();
    let mut answers = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), async {
        while answers.len() < 2 {
            let socket = client.sockets.get_mut::<tcp::Socket>(client.tcp);
            if socket.can_recv() {
                let start = bytes.len();
                bytes.resize(start + socket.recv_queue(), 0);
                let read = socket.recv_slice(&mut bytes[start..]).unwrap();
                bytes.truncate(start + read);
            }
            while bytes.len() >= 2 {
                let len = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
                if bytes.len() < len + 2 {
                    break;
                }
                answers.push(bytes[2..len + 2].to_vec());
                bytes.drain(..len + 2);
            }
            client.advance(&port).await;
        }
    })
    .await
    .unwrap_or_else(|error| {
        panic!(
            "{error:?}; answers={}, bytes={}, state={:?}, tx={}",
            answers.len(),
            bytes.len(),
            client.sockets.get::<tcp::Socket>(client.tcp).state(),
            client.sockets.get::<tcp::Socket>(client.tcp).send_queue()
        )
    });
    positive(&answers[0], 2);
    positive(&answers[1], 3);
    client
        .sockets
        .get_mut::<udp::Socket>(client.udp)
        .send_slice(&query(4), (address, 53))
        .unwrap();
    let held = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            client.poll(&port);
            if let Some(output) = port.recv().await {
                let ip = Ipv4Packet::new_checked(output.bytes()).unwrap();
                if ip.next_header() == IpProtocol::Udp {
                    break output;
                }
                client.device.input.push_back(output.bytes().to_vec());
            }
        }
    })
    .await
    .unwrap();
    held.ensure_current().unwrap();
    crate::publish_scope(&scopes, None);
    assert!(held.ensure_current().is_err());
    client
        .sockets
        .get_mut::<udp::Socket>(client.udp)
        .send_slice(&query(5), (address, 53))
        .unwrap();
    let failed = Message::from_vec(&client.udp_answer(&port).await).unwrap();
    assert_eq!(failed.metadata.response_code, ResponseCode::ServFail);
    stop.cancel();
    running.await.unwrap().unwrap();
    assert!(port.stopped());
}
