use super::*;
use crate::{AcceptedSocket, DatagramSocket, ListenerSocket};
use futures_util::{future::BoxFuture, task::AtomicWaker};
use meshway_control::ExecutionSnapshot;
use meshway_transport::raw::{Datagram, DatagramRead};
use meshway_transport_spec::{DialParameters, Listener};
use meshway_transport_udp::UdpTransport;
use std::{
    collections::BTreeSet,
    io,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    task::{Context, Poll},
    time::Duration,
};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

mod reattachment;
mod regression;
pub(super) mod support;
mod traversal;

#[derive(Default)]
struct Faults {
    drops: Mutex<BTreeSet<u8>>,
    held_controls: Mutex<BTreeSet<u8>>,
    blackhole: AtomicBool,
    block_data: AtomicBool,
    blocked: AtomicBool,
    writes: AtomicUsize,
    waker: AtomicWaker,
    controls: Mutex<Vec<(u8, SocketAddr, Vec<u8>)>>,
}

struct Endpoint {
    inner: UdpTransport,
    faults: Arc<Faults>,
}
impl Datagram for Endpoint {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
    fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        bytes: &mut [u8],
    ) -> Poll<io::Result<DatagramRead>> {
        self.inner.poll_recv_from(cx, bytes)
    }
    fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        bytes: &[u8],
        peer: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        let control = bytes.starts_with(adjacency::MAGIC);
        if !control && self.faults.block_data.load(Ordering::SeqCst) {
            self.faults.waker.register(cx.waker());
            self.faults.blocked.store(true, Ordering::SeqCst);
            return Poll::Pending;
        }
        if self.faults.blackhole.load(Ordering::SeqCst) {
            return Poll::Ready(Ok(bytes.len()));
        }
        if control {
            let kind = bytes[8];
            let mut recorded = self.faults.controls.lock().unwrap();
            if recorded.len() < 128 {
                recorded.push((kind, peer, bytes.to_vec()));
            }
            if self.faults.held_controls.lock().unwrap().contains(&kind) {
                return Poll::Ready(Ok(bytes.len()));
            }
            if self.faults.drops.lock().unwrap().remove(&kind) {
                return Poll::Ready(Ok(bytes.len()));
            }
        }
        let result = self.inner.poll_send_to(cx, bytes, peer);
        if !control && matches!(result, Poll::Ready(Ok(_))) {
            self.faults.writes.fetch_add(1, Ordering::SeqCst);
        }
        result
    }
}

struct Factory {
    udp: Mutex<BTreeMap<SocketAddr, UdpSocket>>,
    tcp: Mutex<Option<TcpListener>>,
    faults: Arc<Faults>,
    blocked_dialers: Mutex<BTreeSet<NodeId>>,
    acquisitions: AtomicUsize,
}
impl Factory {
    fn new(fixture: &mut support::Fixture, faults: Arc<Faults>) -> Arc<Self> {
        Arc::new(Self {
            udp: Mutex::new(
                [fixture.udp.take(), fixture.reverse_udp.take()]
                    .into_iter()
                    .flatten()
                    .chain(fixture.extra_udp.drain(..))
                    .map(|socket| (socket.local_addr().unwrap(), socket))
                    .collect(),
            ),
            tcp: Mutex::new(fixture.tcp.take()),
            faults,
            blocked_dialers: Mutex::new(BTreeSet::new()),
            acquisitions: AtomicUsize::new(0),
        })
    }
    fn endpoint(&self, socket: UdpSocket) -> Box<dyn Datagram> {
        Box::new(Endpoint {
            inner: UdpTransport::adopt(socket, &Default::default()).unwrap(),
            faults: self.faults.clone(),
        })
    }
}
struct StreamListener(TcpListener);
impl ListenerSocket for StreamListener {
    fn local_addr(&self) -> Result<SocketAddr, Error> {
        Ok(self.0.local_addr()?)
    }
    fn accept(&self) -> BoxFuture<'_, Result<AcceptedSocket, Error>> {
        Box::pin(async {
            let (stream, peer) = self.0.accept().await?;
            Ok(AcceptedSocket {
                local: stream.local_addr()?,
                peer,
                stream: Box::new(stream),
            })
        })
    }
}
impl SocketFactory for Factory {
    fn connect<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
        index: usize,
    ) -> BoxFuture<'a, Result<AcceptedSocket, Error>> {
        Box::pin(async move {
            self.acquisitions.fetch_add(1, Ordering::SeqCst);
            let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
            let candidate = &cfg.dialers[key];
            assert!(matches!(
                candidate.parameters,
                DialParameters::NativeTcp { .. }
            ));
            let endpoint = &candidate.endpoints[index];
            let stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port.get())).await?;
            Ok(AcceptedSocket {
                local: stream.local_addr()?,
                peer: stream.peer_addr()?,
                stream: Box::new(stream),
            })
        })
    }
    fn listen<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn ListenerSocket>, Error>> {
        Box::pin(async move {
            let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
            let Listener::NativeTcp(listener) = cfg.listeners[key].declaration() else {
                return Err(Error::Configuration);
            };
            let socket = self.tcp.lock().unwrap().take().ok_or(Error::Unavailable)?;
            assert_eq!(socket.local_addr()?.port(), listener.listen_port.get());
            Ok(Box::new(StreamListener(socket)) as Box<dyn ListenerSocket>)
        })
    }
    fn datagram_listener<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn Datagram>, Error>> {
        Box::pin(async move {
            let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
            let Listener::NativeUdp(listener) = cfg.listeners[key].declaration() else {
                return Err(Error::Configuration);
            };
            let address = SocketAddr::new(listener.listen, listener.listen_port.get());
            let reserved = self.udp.lock().unwrap().remove(&address);
            let socket = match reserved {
                Some(socket) => socket,
                None => UdpSocket::bind(address).await?,
            };
            assert_eq!(socket.local_addr()?.port(), listener.listen_port.get());
            Ok(self.endpoint(socket))
        })
    }
    fn datagram_candidate<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
        index: usize,
    ) -> BoxFuture<'a, Result<Vec<DatagramSocket>, Error>> {
        Box::pin(async move {
            self.acquisitions.fetch_add(1, Ordering::SeqCst);
            let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
            if self.blocked_dialers.lock().unwrap().contains(&cfg.node_id) {
                return Err(Error::Unavailable);
            }
            let candidate = &cfg.dialers[key];
            assert!(matches!(
                candidate.parameters,
                DialParameters::NativeUdp { .. }
            ));
            let endpoint = &candidate.endpoints[index];
            let peer = SocketAddr::new(endpoint.host.parse().unwrap(), endpoint.port.get());
            let local = if peer.is_ipv4() {
                "127.0.0.1:0"
            } else {
                "[::1]:0"
            };
            Ok(vec![DatagramSocket {
                endpoint: self.endpoint(UdpSocket::bind(local).await?),
                targets: vec![peer],
            }])
        })
    }
}

async fn linked(manager: &Manager, count: usize, protocol: TransportProtocol) {
    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            let facts = manager.snapshot().unwrap();
            if facts.links.len() == count
                && facts
                    .links
                    .iter()
                    .all(|link| link.carrier.protocol == protocol)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("bounded actual peer handshake/failover");
}

async fn frame(manager: &mut Manager) -> (LinkFact, Frame) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match manager.recv().await {
                Event::Frame { link, frame } => return (*link, frame),
                Event::Changed => {}
                Event::Closed => panic!("manager closed"),
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn real_udp_four_message_loss_shared_listener_and_unpolled_output_fence() {
    let mut fixture = support::Fixture::new(false, false).await;
    let ids = fixture.ids();
    let faults = Arc::new(Faults::default());
    *faults.drops.lock().unwrap() = BTreeSet::from([1, 2, 3, 4]);
    let factory = Factory::new(&mut fixture, faults.clone());
    let limits = Limits {
        session: meshway_transport::Limits::new(4096, 16, 65_536, 16, 16).unwrap(),
        ..Default::default()
    };
    let mut b = Manager::start(fixture.identities[1].clone(), factory.clone(), limits)
        .await
        .unwrap();
    let mut a = Manager::start(
        fixture.identities[0].clone(),
        factory.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    linked(&a, 1, TransportProtocol::NativeUdp).await;
    let mut c = Manager::start(fixture.identities[2].clone(), factory, Default::default())
        .await
        .unwrap();
    linked(&c, 1, TransportProtocol::NativeUdp).await;
    linked(&b, 2, TransportProtocol::NativeUdp).await;
    let facts = b.snapshot().unwrap();
    assert_eq!(facts.links[0].socket_id, facts.links[1].socket_id);
    assert_eq!(facts.listeners[0].socket_id, facts.links[0].socket_id);
    assert!(faults.drops.lock().unwrap().is_empty());

    // Unassociated sources cannot inject Mesh frames or replay a genuine ACK
    // into a different source tuple. Empty/oversize datagrams do not kill peers.
    let stranger = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = facts.listeners[0].local;
    let ack = faults
        .controls
        .lock()
        .unwrap()
        .iter()
        .find(|(kind, _, _)| *kind == 3)
        .unwrap()
        .2
        .clone();
    // Above the adjacency bound while still below macOS's UDP send limit.
    let mut oversized = vec![0; adjacency::MAX_MESSAGE + 1];
    oversized[..8].copy_from_slice(b"MWADJv1\0");
    for bytes in [vec![], b"MWCIR-invalid".to_vec(), ack, oversized] {
        stranger.send_to(&bytes, address).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(b.snapshot().unwrap().links.len(), 2);

    let mut secure = meshway_dataplane::secure::SecureNode::new(
        fixture.identities[0].clone(),
        Default::default(),
        a.carriers().unwrap(),
    )
    .unwrap();
    let pending = secure
        .start(fixture.demands[0], vec![ids[0], ids[1]])
        .unwrap();
    faults.block_data.store(true, Ordering::SeqCst);
    let before = faults.writes.load(Ordering::SeqCst);
    let mut send = Box::pin(a.send(pending));
    assert!(futures_util::poll!(send.as_mut()).is_pending());
    assert!(faults.blocked.load(Ordering::SeqCst));
    let scope = a
        .shared
        .links
        .lock()
        .unwrap()
        .get(&ids[1])
        .unwrap()
        .scope
        .clone();
    scope.cancellation.cancel();
    faults.block_data.store(false, Ordering::SeqCst);
    faults.waker.wake();
    // Deliberately keep `send` alive without polling it again.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(faults.writes.load(Ordering::SeqCst), before);
    drop(send);

    // The other peer still uses the same listener descriptor and its own guard.
    let mut secure_c = meshway_dataplane::secure::SecureNode::new(
        fixture.identities[2].clone(),
        Default::default(),
        c.carriers().unwrap(),
    )
    .unwrap();
    let pending = secure_c
        .start(fixture.demands[1], vec![ids[2], ids[1]])
        .unwrap();
    c.send(pending).await.unwrap();
    let (link, received) = frame(&mut b).await;
    assert_eq!(link.peer, ids[2]);
    let mut secure_b = meshway_dataplane::secure::SecureNode::new(
        fixture.identities[1].clone(),
        Default::default(),
        b.carriers().unwrap(),
    )
    .unwrap();
    assert!(matches!(
        secure_b.receive(&link.carrier, received.payload).unwrap(),
        meshway_dataplane::secure::Received::Established { .. }
    ));
    for manager in [&mut a, &mut b, &mut c] {
        manager.shutdown().await.unwrap();
    }
    fixture.close().await;
}

#[tokio::test]
async fn real_ipv6_udp_blackhole_selects_allowed_stream_without_replacing_secure_owner() {
    let mut fixture = support::Fixture::new(true, true).await;
    let ids = fixture.ids();
    let faults = Arc::new(Faults::default());
    let factory = Factory::new(&mut fixture, faults.clone());
    let mut b = Manager::start(
        fixture.identities[1].clone(),
        factory.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    let mut a = Manager::start(fixture.identities[0].clone(), factory, Default::default())
        .await
        .unwrap();
    linked(&a, 1, TransportProtocol::NativeUdp).await;
    linked(&b, 1, TransportProtocol::NativeUdp).await;
    let mut secure_a = meshway_dataplane::secure::SecureNode::new(
        fixture.identities[0].clone(),
        Default::default(),
        a.carriers().unwrap(),
    )
    .unwrap();
    let mut secure_b = meshway_dataplane::secure::SecureNode::new(
        fixture.identities[1].clone(),
        Default::default(),
        b.carriers().unwrap(),
    )
    .unwrap();
    let pending = secure_a
        .start(fixture.demands[0], vec![ids[0], ids[1]])
        .unwrap();
    let circuit = pending.circuit();
    a.send(pending).await.unwrap();
    let (link, received) = frame(&mut b).await;
    let meshway_dataplane::secure::Received::Established {
        response: Some(response),
        ..
    } = secure_b.receive(&link.carrier, received.payload).unwrap()
    else {
        panic!("end-to-end handshake");
    };
    b.send(response).await.unwrap();
    let (link, received) = frame(&mut a).await;
    assert!(matches!(
        secure_a.receive(&link.carrier, received.payload).unwrap(),
        meshway_dataplane::secure::Received::Established { .. }
    ));
    faults.blackhole.store(true, Ordering::SeqCst);
    linked(&a, 1, TransportProtocol::NativeTcp).await;
    linked(&b, 1, TransportProtocol::NativeTcp).await;
    secure_a.set_carriers(a.carriers().unwrap()).unwrap();
    secure_b.set_carriers(b.carriers().unwrap()).unwrap();
    assert!(
        secure_a
            .ready_circuits()
            .unwrap()
            .iter()
            .any(|ready| ready.id == circuit)
    );
    faults.blackhole.store(false, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        a.snapshot().unwrap().links[0].carrier.protocol,
        TransportProtocol::NativeTcp
    );
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn simultaneous_configured_initiators_converge_on_one_logical_carrier() {
    let mut fixture = support::Fixture::reverse().await;
    let faults = Arc::new(Faults::default());
    let factory = Factory::new(&mut fixture, faults);
    let (a, b) = tokio::join!(
        Manager::start(
            fixture.identities[0].clone(),
            factory.clone(),
            Default::default()
        ),
        Manager::start(fixture.identities[1].clone(), factory, Default::default()),
    );
    let (mut a, mut b) = (a.unwrap(), b.unwrap());
    linked(&a, 1, TransportProtocol::NativeUdp).await;
    linked(&b, 1, TransportProtocol::NativeUdp).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let a = a.snapshot().unwrap();
            let b = b.snapshot().unwrap();
            if a.links.len() == 1 && b.links.len() == 1 && a.links[0].id == b.links[0].id {
                assert_eq!(a.links[0].carrier, b.links[0].carrier);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let id = a.snapshot().unwrap().links[0].id;
    tokio::time::sleep(Duration::from_secs(7)).await;
    assert_eq!(a.snapshot().unwrap().links[0].id, id);
    assert_eq!(b.snapshot().unwrap().links[0].id, id);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}
