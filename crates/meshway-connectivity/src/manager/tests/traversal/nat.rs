use super::*;

struct NatEndpoint {
    inner: UdpTransport,
    pub(super) stun: SocketAddr,
    opened: Mutex<BTreeSet<SocketAddr>>,
    pub(super) traffic: Arc<Traffic>,
}
impl Drop for NatEndpoint {
    fn drop(&mut self) {
        self.traffic.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
impl Datagram for NatEndpoint {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
    fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        bytes: &[u8],
        peer: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        let business = !bytes.starts_with(adjacency::MAGIC)
            && !bytes.starts_with(b"MWTRV1\0\0")
            && meshway_stun::classify(bytes).is_none();
        if business && self.traffic.block_data.load(Ordering::SeqCst) {
            self.traffic.waker.register(cx.waker());
            self.traffic.data_blocked.store(true, Ordering::SeqCst);
            return Poll::Pending;
        }
        if bytes.starts_with(b"MWTRV1\0\0") && self.traffic.block_traversal.load(Ordering::SeqCst) {
            self.traffic.waker.register(cx.waker());
            self.traffic
                .blocked
                .lock()
                .unwrap()
                .get_or_insert_with(|| Blocked {
                    local: self.inner.local_addr().unwrap(),
                    remote: peer,
                    digest: bytes[8..40].try_into().expect("owned traversal header"),
                    waker: cx.waker().clone(),
                });
            return Poll::Pending;
        }
        let result = self.inner.poll_send_to(cx, bytes, private(peer));
        if matches!(result, Poll::Ready(Ok(_))) {
            if let Some(timeline) = &self.traffic.timeline {
                timeline.packet("send", self.inner.local_addr()?, peer, bytes);
            }
            if business {
                self.traffic.data_writes.fetch_add(1, Ordering::SeqCst);
            }
            self.opened.lock().unwrap().insert(peer);
            let mut sent = self.traffic.sent.lock().unwrap();
            if sent.len() < 4096 {
                sent.push((self.inner.local_addr()?, peer, bytes.to_vec()));
            }
        }
        result
    }
    fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        bytes: &mut [u8],
    ) -> Poll<io::Result<DatagramRead>> {
        for _ in 0..16 {
            let mut read = match self.inner.poll_recv_from(cx, bytes) {
                Poll::Ready(result) => result?,
                Poll::Pending => return Poll::Pending,
            };
            read.source = public(read.source);
            if let Some(timeline) = &self.traffic.timeline {
                timeline.packet(
                    "recv",
                    self.inner.local_addr()?,
                    read.source,
                    &bytes[..read.copied_len],
                );
            }
            if read.source == self.stun || self.opened.lock().unwrap().contains(&read.source) {
                return Poll::Ready(Ok(read));
            }
        }
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

struct Authorization {
    binding: Mutex<(SocketReference, ExecutionSnapshot)>,
    pub(super) stun: SocketAddr,
    local: SocketAddr,
    pub(super) traffic: Arc<Traffic>,
}
impl DatagramAuthorization for Authorization {
    fn handoff(
        &self,
        snapshot: &ExecutionSnapshot,
        reference: &SocketReference,
        channels: &[(SocketAddr, Arc<dyn meshway_transport::SendLifetime>)],
    ) -> Result<(), Error> {
        let mut binding = self.binding.lock().unwrap();
        snapshot
            .continuation_from(&binding.1)
            .map_err(|_| Error::Authority)?;
        assert_eq!(reference.socket_id, binding.0.socket_id);
        assert_eq!(reference.socket_generation, binding.0.socket_generation);
        assert_eq!(reference.resource, binding.0.resource);
        assert!(channels.iter().all(|(_, scope)| scope.is_current()));
        *binding = (reference.clone(), snapshot.clone());
        Ok(())
    }
    fn authorize_stun<'a>(
        &'a self,
        context: &'a DiscoveryContext,
        server: &'a str,
    ) -> BoxFuture<'a, Result<Vec<SocketAddr>, Error>> {
        Box::pin(async move {
            context.ensure_current().map_err(|_| Error::Authority)?;
            assert!(
                context.servers().iter().any(
                    |value| value.id == server && value.endpoint.port.get() == self.stun.port()
                )
            );
            Ok(vec![self.stun])
        })
    }
    fn authorize_attempt(&self, active: &AttemptScope) -> Result<(), Error> {
        active.ensure_current().map_err(|_| Error::Authority)?;
        assert_eq!(
            active.validated_offer().local().candidate.socket,
            self.binding.lock().unwrap().0
        );
        let mut authorized = self.traffic.authorized.lock().unwrap();
        assert!(authorized.len() < 16);
        authorized.insert(
            (
                self.local,
                active.validated_offer().offer().digest.into_bytes(),
            ),
            active.clone(),
        );
        Ok(())
    }
    fn authorize_channel(
        &self,
        active: &AttemptScope,
        channel: Arc<dyn meshway_transport::SendLifetime>,
    ) -> Result<(), Error> {
        self.authorize_attempt(active)?;
        assert!(channel.is_current());
        let mut channels = self.traffic.channels.lock().unwrap();
        assert!(channels.len() < 16);
        channels.push(ChannelAuthorization {
            local: self.local,
            attempt: active.clone(),
            lifetime: channel,
        });
        Ok(())
    }
}
pub(super) struct NatFactory {
    pub(super) sockets: Mutex<BTreeMap<SocketAddr, UdpSocket>>,
    pub(super) stun: SocketAddr,
    pub(super) traffic: Arc<Traffic>,
}
impl NatFactory {
    fn wrap(
        &self,
        snapshot: &ExecutionSnapshot,
        reference: SocketReference,
        inner: UdpTransport,
    ) -> Result<TraversalSocket, Error> {
        let local = inner.local_addr()?;
        self.traffic.opened.fetch_add(1, Ordering::SeqCst);
        Ok(TraversalSocket {
            endpoint: Box::new(NatEndpoint {
                inner,
                stun: self.stun,
                opened: Mutex::new(BTreeSet::new()),
                traffic: self.traffic.clone(),
            }),
            authorization: Arc::new(Authorization {
                binding: Mutex::new((reference, snapshot.clone())),
                stun: self.stun,
                local,
                traffic: self.traffic.clone(),
            }),
        })
    }
}
impl SocketFactory for NatFactory {
    fn bind_datagram_candidate<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
        ipv4: bool,
        reference: SocketReference,
    ) -> BoxFuture<'a, Result<TraversalSocket, Error>> {
        Box::pin(async move {
            let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
            let DialParameters::NativeUdp { options } = &cfg.dialers[key].parameters else {
                return Err(Error::Configuration);
            };
            assert_eq!(reference.resource, ResourceRef::Dial { key: key.into() });
            let local = if options.local_address.is_empty() {
                if ipv4 { "127.0.0.1" } else { "::1" }.parse().unwrap()
            } else {
                options.local_address.parse().unwrap()
            };
            let socket = UdpSocket::bind(SocketAddr::new(local, 0)).await?;
            let inner = UdpTransport::adopt(socket, &options.socket)?;
            self.wrap(snapshot, reference, inner)
        })
    }
    fn connect<'a>(
        &'a self,
        _: &'a ExecutionSnapshot,
        _: &'a str,
        _: usize,
    ) -> BoxFuture<'a, Result<AcceptedSocket, Error>> {
        Box::pin(async { Err(Error::Configuration) })
    }
    fn listen<'a>(
        &'a self,
        _: &'a ExecutionSnapshot,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn ListenerSocket>, Error>> {
        Box::pin(async { Err(Error::Configuration) })
    }
    fn traversal_listener<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
        reference: SocketReference,
    ) -> BoxFuture<'a, Result<TraversalSocket, Error>> {
        Box::pin(async move {
            let Listener::NativeUdp(declaration) =
                snapshot.configuration().unwrap().listeners[key].declaration()
            else {
                return Err(Error::Configuration);
            };
            assert!(declaration.endpoints.is_empty());
            let local = SocketAddr::new(declaration.listen, declaration.listen_port.get());
            let socket = self.sockets.lock().unwrap().remove(&local);
            let socket = match socket {
                Some(socket) => socket,
                None => UdpSocket::bind(local).await?,
            };
            let inner = UdpTransport::adopt(socket, &declaration.socket)?;
            self.wrap(snapshot, reference, inner)
        })
    }
}
