use super::super::*;

#[derive(Default)]
pub(in crate::manager::tests) struct Faults {
    pub(in crate::manager::tests) drops: Mutex<BTreeSet<u8>>,
    pub(in crate::manager::tests) held_controls: Mutex<BTreeSet<u8>>,
    pub(in crate::manager::tests) blackhole: AtomicBool,
    pub(in crate::manager::tests) block_data: AtomicBool,
    pub(in crate::manager::tests) blocked: AtomicBool,
    pub(in crate::manager::tests) writes: AtomicUsize,
    pub(in crate::manager::tests) waker: AtomicWaker,
    pub(in crate::manager::tests) controls: Mutex<Vec<(u8, SocketAddr, Vec<u8>)>>,
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

pub(in crate::manager::tests) struct Factory {
    pub(in crate::manager::tests) udp: Mutex<BTreeMap<SocketAddr, UdpSocket>>,
    pub(in crate::manager::tests) tcp: Mutex<BTreeMap<SocketAddr, TcpListener>>,
    pub(in crate::manager::tests) faults: Arc<Faults>,
    pub(in crate::manager::tests) blocked_dialers: Mutex<BTreeSet<NodeId>>,
    pub(in crate::manager::tests) acquisitions: AtomicUsize,
}
impl Factory {
    pub(in crate::manager::tests) fn new(
        fixture: &mut support::Fixture,
        faults: Arc<Faults>,
    ) -> Arc<Self> {
        Arc::new(Self {
            udp: Mutex::new(
                [fixture.udp.take(), fixture.reverse_udp.take()]
                    .into_iter()
                    .flatten()
                    .chain(fixture.extra_udp.drain(..))
                    .map(|socket| (socket.local_addr().unwrap(), socket))
                    .collect(),
            ),
            tcp: Mutex::new(
                fixture
                    .tcp
                    .take()
                    .into_iter()
                    .chain(fixture.extra_tcp.drain(..))
                    .map(|socket| (socket.local_addr().unwrap(), socket))
                    .collect(),
            ),
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
            if self.blocked_dialers.lock().unwrap().contains(&cfg.node_id) {
                return Err(Error::Unavailable);
            }
            assert!(!matches!(
                candidate.parameters,
                DialParameters::NativeUdp { .. }
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
            let address =
                super::super::streaming::listener_address(cfg.listeners[key].declaration())?;
            let reserved = self.tcp.lock().unwrap().remove(&address);
            let socket = match reserved {
                Some(socket) => socket,
                None => TcpListener::bind(address).await?,
            };
            assert_eq!(socket.local_addr()?.port(), address.port());
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
