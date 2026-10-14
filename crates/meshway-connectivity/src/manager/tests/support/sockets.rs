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
    fn buffers(&self) -> Option<meshway_transport::raw::Buffers> {
        Some(self.inner.buffers())
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
    pub(in crate::manager::tests) traversal_binds: AtomicUsize,
    /// Native UDP listeners bound as coordination sockets.
    pub(in crate::manager::tests) traversal_listeners: AtomicUsize,
    /// State references each traversal socket was rebound to, in order.
    pub(in crate::manager::tests) rebinds:
        Arc<Mutex<Vec<meshway_control::connectivity::SocketReference>>>,
}

/// Coordination authority that grants no target: binding only.
struct Unauthorized(Arc<Mutex<Vec<meshway_control::connectivity::SocketReference>>>);
impl crate::DatagramAuthorization for Unauthorized {
    fn rebind(
        &self,
        reference: &meshway_control::connectivity::SocketReference,
    ) -> Result<(), Error> {
        self.0.lock().unwrap().push(reference.clone());
        Ok(())
    }
    fn authorize_stun<'a>(
        &'a self,
        _context: &'a meshway_control::connectivity::DiscoveryContext,
        _server: &'a str,
    ) -> BoxFuture<'a, Result<Vec<SocketAddr>, Error>> {
        Box::pin(async { Err(Error::Underlay) })
    }
    fn authorize_attempt(
        &self,
        _scope: &meshway_control::connectivity::AttemptScope,
    ) -> Result<(), Error> {
        Err(Error::Underlay)
    }
    fn authorize_learned(
        &self,
        _scope: &meshway_control::connectivity::AttemptScope,
        _address: SocketAddr,
    ) -> Result<(), Error> {
        Err(Error::Underlay)
    }
    fn authorize_channel(
        &self,
        _scope: &meshway_control::connectivity::AttemptScope,
        _lifetime: Arc<dyn meshway_transport::SendLifetime>,
        _address: SocketAddr,
    ) -> Result<(), Error> {
        Err(Error::Underlay)
    }
}
impl Factory {
    pub(in crate::manager::tests) fn endpoint(&self, socket: UdpSocket) -> Box<dyn Datagram> {
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
/// As the Linux factory reports a listener's bind failure.
fn bind_error(key: &str, address: SocketAddr, source: io::Error) -> Error {
    Error::Bind {
        key: key.to_owned(),
        address,
        source,
    }
}
fn listener_address(listener: &Listener) -> Result<SocketAddr, Error> {
    let (ip, port) = match listener {
        Listener::NativeTcp(value) => (value.listen, value.listen_port),
        Listener::Vless(value) => (value.listen, value.listen_port),
        Listener::Trojan(value) => (value.listen, value.listen_port),
        Listener::NativeUdp(_) => return Err(Error::Configuration),
    };
    Ok(SocketAddr::new(ip, port.get()))
}
impl SocketFactory for Factory {
    fn bind_datagram_candidate<'a>(
        &'a self,
        _desired: &'a meshway_engine::Desired,
        _key: &'a str,
        ipv4: bool,
        _reference: meshway_control::connectivity::SocketReference,
    ) -> BoxFuture<'a, Result<crate::TraversalSocket, Error>> {
        Box::pin(async move {
            self.traversal_binds.fetch_add(1, Ordering::SeqCst);
            let local = if ipv4 { "127.0.0.1:0" } else { "[::1]:0" };
            Ok(crate::TraversalSocket {
                endpoint: self.endpoint(UdpSocket::bind(local).await?),
                authorization: Arc::new(Unauthorized(self.rebinds.clone())),
            })
        })
    }
    fn connect_endpoint<'a>(
        &'a self,
        desired: &'a meshway_engine::Desired,
        key: &'a str,
        endpoint: &'a meshway_configuration::execution::DialEndpoint,
    ) -> BoxFuture<'a, Result<AcceptedSocket, Error>> {
        Box::pin(async move {
            self.acquisitions.fetch_add(1, Ordering::SeqCst);
            let cfg = crate::plan::Plan::new(desired)?;
            if self.blocked_dialers.lock().unwrap().contains(&cfg.node) {
                return Err(Error::Unavailable);
            }
            assert!(!matches!(
                cfg.dialers[key].parameters,
                DialParameters::NativeUdp { .. }
            ));
            let stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port.get())).await?;
            Ok(AcceptedSocket {
                local: stream.local_addr()?,
                peer: stream.peer_addr()?,
                stream: Box::new(stream),
            })
        })
    }
    fn datagram_endpoint<'a>(
        &'a self,
        desired: &'a meshway_engine::Desired,
        key: &'a str,
        endpoint: &'a meshway_configuration::execution::DialEndpoint,
    ) -> BoxFuture<'a, Result<Vec<DatagramSocket>, Error>> {
        Box::pin(async move {
            self.acquisitions.fetch_add(1, Ordering::SeqCst);
            let cfg = crate::plan::Plan::new(desired)?;
            if self.blocked_dialers.lock().unwrap().contains(&cfg.node) {
                return Err(Error::Unavailable);
            }
            assert!(matches!(
                cfg.dialers[key].parameters,
                DialParameters::NativeUdp { .. }
            ));
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
    fn listen<'a>(
        &'a self,
        desired: &'a meshway_engine::Desired,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn ListenerSocket>, Error>> {
        Box::pin(async move {
            let cfg = crate::plan::Plan::new(desired)?;
            let address = listener_address(cfg.listeners[key].declaration())?;
            let reserved = self.tcp.lock().unwrap().remove(&address);
            let socket = match reserved {
                Some(socket) => socket,
                None => TcpListener::bind(address)
                    .await
                    .map_err(|source| bind_error(key, address, source))?,
            };
            assert_eq!(socket.local_addr()?.port(), address.port());
            Ok(Box::new(StreamListener(socket)) as Box<dyn ListenerSocket>)
        })
    }
    fn traversal_listener<'a>(
        &'a self,
        desired: &'a meshway_engine::Desired,
        key: &'a str,
        _reference: meshway_control::connectivity::SocketReference,
    ) -> BoxFuture<'a, Result<crate::TraversalSocket, Error>> {
        Box::pin(async move {
            let endpoint = self.datagram_listener(desired, key).await?;
            self.traversal_listeners.fetch_add(1, Ordering::SeqCst);
            Ok(crate::TraversalSocket {
                endpoint,
                authorization: Arc::new(Unauthorized(self.rebinds.clone())),
            })
        })
    }
    fn datagram_listener<'a>(
        &'a self,
        desired: &'a meshway_engine::Desired,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn Datagram>, Error>> {
        Box::pin(async move {
            let cfg = crate::plan::Plan::new(desired)?;
            let Listener::NativeUdp(listener) = cfg.listeners[key].declaration() else {
                return Err(Error::Configuration);
            };
            let address = SocketAddr::new(listener.listen, listener.listen_port.get());
            let reserved = self.udp.lock().unwrap().remove(&address);
            let socket = match reserved {
                Some(socket) => socket,
                None => UdpSocket::bind(address)
                    .await
                    .map_err(|source| bind_error(key, address, source))?,
            };
            assert_eq!(socket.local_addr()?.port(), listener.listen_port.get());
            Ok(self.endpoint(socket))
        })
    }
}
