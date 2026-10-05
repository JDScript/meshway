//! Real Server/Control + real UDP descriptors with an isolated userspace NAT
//! address/filter adapter. Host routing and DNS are never modified.
use super::*;
use crate::{
    DatagramAuthorization, TraversalSocket,
    traversal::{Operation, Reply, TraversalGateway},
};
use meshway_control::{
    Control,
    connectivity::{AttemptScope, DiscoveryContext},
};
use meshway_model::{Sequence, state::*};
use meshway_protocol::connectivity::{CandidateReference, SocketReference};

fn public(address: SocketAddr) -> SocketAddr {
    SocketAddr::new(
        if address.is_ipv4() {
            "198.51.100.1"
        } else {
            "2001:db8::1"
        }
        .parse()
        .unwrap(),
        address.port(),
    )
}
fn private(address: SocketAddr) -> SocketAddr {
    SocketAddr::new(
        if address.is_ipv4() {
            "127.0.0.1"
        } else {
            "::1"
        }
        .parse()
        .unwrap(),
        address.port(),
    )
}

fn listener_socket(manager: &Manager) -> SocketFact {
    manager
        .snapshot()
        .unwrap()
        .sockets
        .into_iter()
        .find(|socket| socket.reference.resource == (ResourceRef::Listener { key: "udp".into() }))
        .unwrap()
}

#[derive(Default)]
struct Traffic {
    sent: Mutex<Vec<(SocketAddr, SocketAddr, Vec<u8>)>>,
    block_traversal: AtomicBool,
    blocked: Mutex<Option<Blocked>>,
    authorized: Mutex<BTreeMap<(SocketAddr, [u8; 32]), AttemptScope>>,
    channels: Mutex<Vec<ChannelAuthorization>>,
    waker: AtomicWaker,
    block_data: AtomicBool,
    data_blocked: AtomicBool,
    data_writes: AtomicUsize,
    opened: AtomicUsize,
    dropped: AtomicUsize,
}
struct ChannelAuthorization {
    local: SocketAddr,
    attempt: AttemptScope,
    lifetime: Arc<dyn meshway_transport::SendLifetime>,
}
#[derive(Clone)]
struct Blocked {
    local: SocketAddr,
    remote: SocketAddr,
    digest: [u8; 32],
    waker: std::task::Waker,
}
struct NatEndpoint {
    inner: UdpTransport,
    stun: SocketAddr,
    opened: Mutex<BTreeSet<SocketAddr>>,
    traffic: Arc<Traffic>,
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
    stun: SocketAddr,
    local: SocketAddr,
    traffic: Arc<Traffic>,
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
struct NatFactory {
    sockets: Mutex<BTreeMap<SocketAddr, UdpSocket>>,
    stun: SocketAddr,
    traffic: Arc<Traffic>,
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

struct Gateway {
    control: AsyncMutex<Control>,
    acknowledged: Mutex<Vec<CandidateReference>>,
}

struct OfflineGateway;
impl TraversalGateway for OfflineGateway {
    fn request(&self, _: Operation) -> BoxFuture<'_, Result<Reply, Error>> {
        Box::pin(std::future::pending())
    }
    fn published(&self, _: &CandidateReference) -> bool {
        false
    }
}

#[tokio::test]
async fn incompatible_listener_binds_exact_dial_family_and_capacity_failure_releases_sockets() {
    use meshway_configuration::Input;
    use meshway_transport_spec::{DialerSet, NativeUdpDialer};
    let stun = "198.51.100.1:3478".parse().unwrap();
    let mut fixture = support::Fixture::traversal(stun).await;
    let ids = fixture.ids();
    fixture
        .configure(|cfg| {
            cfg.nodes.get_mut(&ids[0]).unwrap().dialers = Input::Value(DialerSet {
                native_udp: Some(NativeUdpDialer {
                    local_address: "::1".into(),
                    ..Default::default()
                }),
                native_tcp: None,
                vless: None,
                trojan: None,
            });
        })
        .await;
    let identity = fixture.refresh(0).await;
    let traffic = Arc::new(Traffic::default());
    let socket = fixture.reverse_udp.take().unwrap();
    let factory = Arc::new(NatFactory {
        sockets: Mutex::new(BTreeMap::from([(socket.local_addr().unwrap(), socket)])),
        stun,
        traffic: traffic.clone(),
    });
    let generation = Sequence::MIN.checked_next().unwrap();
    let mut manager = Manager::start_with_traversal(
        identity,
        factory.clone(),
        Default::default(),
        Arc::new(OfflineGateway),
        generation,
    )
    .await
    .unwrap();
    let facts = manager.snapshot().unwrap();
    assert_eq!(facts.sockets.len(), 2);
    assert!(listener_socket(&manager).bound_address.is_ipv4());
    let dial = facts
        .sockets
        .iter()
        .find(|socket| matches!(socket.reference.resource, ResourceRef::Dial { .. }))
        .unwrap();
    assert_eq!(
        dial.bound_address.ip(),
        "::1".parse::<std::net::IpAddr>().unwrap()
    );
    assert_ne!(dial.bound_address.port(), 0);
    manager.shutdown().await.unwrap();

    fixture
        .configure(|cfg| {
            let node = cfg.nodes.get_mut(&ids[0]).unwrap();
            let Input::Value(dialers) = &mut node.dialers else {
                panic!("explicit dialer")
            };
            dialers.native_udp.as_mut().unwrap().local_address.clear();
            let Listener::NativeUdp(listener) = node.listeners.get_mut("udp").unwrap() else {
                panic!("UDP listener")
            };
            listener.socket.receive_buffer = 65_536;
        })
        .await;
    let identity = fixture.refresh(0).await;
    let result = Manager::start_with_traversal(
        identity,
        factory,
        Limits {
            candidates: 1,
            ..Default::default()
        },
        Arc::new(OfflineGateway),
        generation,
    )
    .await;
    assert!(
        matches!(result, Err(Error::Capacity)),
        "both families are uncovered; do not silently skip the second socket"
    );
    assert_eq!(
        traffic.opened.load(Ordering::SeqCst),
        traffic.dropped.load(Ordering::SeqCst)
    );
    fixture.close().await;
}

#[tokio::test]
async fn uncovered_dial_resources_beyond_four_are_all_bound() {
    let stun = "198.51.100.1:3478".parse().unwrap();
    let mut fixture = support::Fixture::traversal(stun).await;
    let ids = fixture.ids();
    let mut reserved = Vec::new();
    for _ in 0..5 {
        reserved.push(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    }
    fixture
        .configure(|cfg| {
            let listeners = &mut cfg.nodes.get_mut(&ids[1]).unwrap().listeners;
            let Listener::NativeUdp(template) = listeners["udp"].clone() else {
                panic!("UDP listener")
            };
            for (index, socket) in reserved.iter().enumerate() {
                let mut declaration = template.clone();
                declaration.listen_port = socket.local_addr().unwrap().port().try_into().unwrap();
                listeners.insert(format!("udp-{index}"), Listener::NativeUdp(declaration));
            }
        })
        .await;
    let identity = fixture.refresh(0).await;
    let keys: Vec<_> = identity
        .snapshot()
        .configuration()
        .unwrap()
        .dialers
        .keys()
        .cloned()
        .collect();
    assert_eq!(keys.len(), 6);
    let socket = fixture.reverse_udp.take().unwrap();
    let traffic = Arc::new(Traffic::default());
    let factory = Arc::new(NatFactory {
        sockets: Mutex::new(BTreeMap::from([(socket.local_addr().unwrap(), socket)])),
        stun,
        traffic: traffic.clone(),
    });
    let mut manager = Manager::start_with_traversal(
        identity,
        factory,
        Default::default(),
        Arc::new(OfflineGateway),
        Sequence::MIN.checked_next().unwrap(),
    )
    .await
    .unwrap();
    let facts = manager.snapshot().unwrap();
    assert_eq!(
        facts.sockets.len(),
        7,
        "one reusable IPv4 listener and all six missing IPv6 dial resources"
    );
    for key in keys {
        assert!(facts.sockets.iter().any(|socket| {
            socket.bound_address.is_ipv6()
                && socket.reference.resource == (ResourceRef::Dial { key: key.clone() })
        }));
    }
    manager.shutdown().await.unwrap();
    assert_eq!(
        traffic.opened.load(Ordering::SeqCst),
        traffic.dropped.load(Ordering::SeqCst)
    );
    fixture.close().await;
}
impl TraversalGateway for Gateway {
    fn request(&self, operation: Operation) -> BoxFuture<'_, Result<Reply, Error>> {
        Box::pin(async move {
            let mut control = self.control.lock().await;
            let result: Result<Reply, meshway_control::Error> = async {
                Ok(match operation {
                    Operation::Query(snapshot) => {
                        Reply::Query(control.query_udp_traversal(&snapshot).await?)
                    }
                    Operation::Peers { context, peers } => {
                        Reply::Peers(control.query_connectivity_peers(&context, &peers).await?)
                    }
                    Operation::Prepare { context, request } => {
                        Reply::Prepare(control.prepare_udp_traversal(&context, &request).await?)
                    }
                    Operation::Accept(offer) => {
                        Reply::Accept(control.accept_udp_traversal(&offer).await?)
                    }
                    Operation::Cancel(offer) => {
                        control.cancel_udp_traversal(&offer).await?;
                        Reply::Cancelled
                    }
                })
            }
            .await;
            result.map_err(|_| Error::Unavailable)
        })
    }
    fn published(&self, reference: &CandidateReference) -> bool {
        self.acknowledged.lock().unwrap().contains(reference)
    }
}
impl Gateway {
    async fn report_changes(&self, snapshot: Snapshot) {
        let candidates: Vec<_> = snapshot
            .sockets
            .iter()
            .flat_map(|socket| {
                socket
                    .endpoints
                    .iter()
                    .map(|observation| socket.candidate(observation).unwrap())
            })
            .collect();
        let changed = *self.acknowledged.lock().unwrap() != candidates;
        if changed {
            self.report(snapshot).await;
        }
    }
    async fn synchronize(
        &self,
        manager: &mut Manager,
        secure: &mut meshway_dataplane::secure::SecureNode,
    ) -> meshway_control::DataPlaneIdentity {
        self.install(manager, secure, true).await
    }
    async fn install(
        &self,
        manager: &mut Manager,
        secure: &mut meshway_dataplane::secure::SecureNode,
        continuous: bool,
    ) -> meshway_control::DataPlaneIdentity {
        let identity = {
            let mut control = self.control.lock().await;
            control.authenticate().await.unwrap();
            control.refresh_configuration().await.unwrap();
            control.acknowledge_saved().await.unwrap();
            control
                .dataplane_identity(control.execution_snapshot().unwrap())
                .unwrap()
        };
        assert!(
            !continuous || manager.can_reconcile(&identity),
            "unchanged physical resources must accept the direct Control handoff: stopped={}, lineage={}, original_bounds={}, resources={:?}, health={:?}",
            manager.shared.stop.is_cancelled(),
            identity
                .snapshot()
                .continuation_from(manager.shared.identity.snapshot())
                .is_ok(),
            manager
                .shared
                .identity
                .snapshot()
                .retention_deadline()
                .is_ok(),
            identity
                .snapshot()
                .continuation_from(manager.shared.identity.snapshot())
                .ok()
                .map(|old| {
                    let cfg = identity.snapshot().configuration().unwrap();
                    (
                        old.listeners == cfg.listeners,
                        old.dialers == cfg.dialers,
                        old.peers == cfg.peers,
                    )
                }),
            manager
                .shared
                .links
                .lock()
                .unwrap()
                .values()
                .map(|link| (
                    link.scope.health.as_ref().map(|health| health
                        .lock()
                        .unwrap()
                        .saturating_duration_since(tokio::time::Instant::now())),
                    admit_adjacency(
                        identity.snapshot().configuration().unwrap(),
                        link.fact.peer,
                        &link.fact.carrier,
                        identity.snapshot().now_ms().unwrap()
                    )
                    .is_ok()
                ))
                .collect::<Vec<_>>()
        );
        secure.reconcile(identity.clone()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), manager.reconcile(identity.clone()))
            .await
            .expect("handoff without acquisition")
            .unwrap();
        secure.set_carriers(manager.carriers().unwrap()).unwrap();
        {
            let mut control = self.control.lock().await;
            let ticket = control.begin_application().await.unwrap();
            control
                .finish_application(&ticket, Ok(()), chrono::Utc::now())
                .await
                .unwrap();
        }
        self.report(manager.snapshot().unwrap()).await;
        identity
    }

    async fn report(&self, snapshot: Snapshot) {
        let mut control = self.control.lock().await;
        let identity = control.execution_snapshot().unwrap();
        let cfg = identity.configuration().unwrap();
        let reference = identity.reference();
        let mut state = NodeState {
            system: SystemState {
                hostname: cfg.hostname.clone(),
                os: "test".into(),
                arch: "test".into(),
                agent_version: "test".into(),
            },
            capabilities: cfg.self_keys.capabilities.clone(),
            configuration: ConfigurationState {
                saved: Some(reference.clone()),
                applied: Some(reference),
                progress: None,
            },
            listeners: BTreeMap::new(),
            sockets: BTreeMap::new(),
            links: BTreeMap::new(),
            paths: BTreeMap::new(),
            selection: Default::default(),
            probes: Vec::new(),
            problems: Vec::new(),
        };
        let mut candidates = Vec::new();
        for socket in snapshot.sockets {
            for observation in &socket.endpoints {
                candidates.push(socket.candidate(observation).unwrap());
            }
            if let ResourceRef::Listener { key } = &socket.reference.resource {
                state.listeners.insert(
                    key.clone(),
                    ListenerState {
                        phase: ListenerPhase::Listening,
                        socket_ids: vec![socket.reference.socket_id.clone()],
                        loaded_certificate_sha256: None,
                        problem: None,
                        configuration: Some(socket.reference.configuration.clone()),
                    },
                );
            }
            state.sockets.insert(
                socket.reference.socket_id,
                SocketState {
                    resource: socket.reference.resource,
                    configuration: Some(socket.reference.configuration),
                    generation: socket.reference.socket_generation,
                    network: socket.network,
                    bound_address: socket.bound_address,
                    endpoints: socket.endpoints,
                },
            );
        }
        control
            .report_runtime_state(state, chrono::Utc::now())
            .await
            .unwrap();
        *self.acknowledged.lock().unwrap() = candidates;
    }
}

#[tokio::test]
async fn empty_published_listeners_use_same_fd_stun_bilateral_punch_and_existing_noise_owner() {
    let stun = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let stun_address = public(stun.local_addr().unwrap());
    let stop = CancellationToken::new();
    let stopped = stop.clone();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let observers = observed.clone();
    let service = tokio::spawn(async move {
        let mut bytes = [0; meshway_stun::MAX_DATAGRAM_SIZE];
        loop {
            let (len, source) = tokio::select! { _ = stopped.cancelled() => break, result = stun.recv_from(&mut bytes) => result.unwrap() };
            let request = meshway_stun::parse_request(&bytes[..len]).unwrap();
            observers.lock().unwrap().push(source);
            let response = meshway_stun::encode_response(&request, public(source));
            stun.send_to(&response, source).await.unwrap();
            // A replay must not manufacture another observation.
            stun.send_to(&response, source).await.unwrap();
        }
    });
    let mut fixture = support::Fixture::traversal(stun_address).await;
    let ids = fixture.ids();
    let traffic = Arc::new(Traffic::default());
    let sockets = [
        fixture.udp.take().unwrap(),
        fixture.reverse_udp.take().unwrap(),
    ]
    .into_iter()
    .map(|socket| (socket.local_addr().unwrap(), socket))
    .collect();
    let factory = Arc::new(NatFactory {
        sockets: Mutex::new(sockets),
        stun: stun_address,
        traffic: traffic.clone(),
    });
    let mut gateways = Vec::new();
    for mut control in fixture.controls.drain(..2) {
        control.acknowledge_saved().await.unwrap();
        let ticket = control.begin_application().await.unwrap();
        control
            .finish_application(&ticket, Ok(()), chrono::Utc::now())
            .await
            .unwrap();
        gateways.push(Arc::new(Gateway {
            control: AsyncMutex::new(control),
            acknowledged: Mutex::new(Vec::new()),
        }));
    }
    let generation = Sequence::MIN.checked_next().unwrap();
    let mut a = Manager::start_with_traversal(
        fixture.identities[0].clone(),
        factory.clone(),
        Default::default(),
        gateways[0].clone(),
        generation,
    )
    .await
    .unwrap();
    let mut b = Manager::start_with_traversal(
        fixture.identities[1].clone(),
        factory.clone(),
        Default::default(),
        gateways[1].clone(),
        generation,
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if a.snapshot()
                .unwrap()
                .sockets
                .iter()
                .filter(|socket| socket.bound_address.is_ipv4())
                .all(|socket| !socket.endpoints.is_empty())
                && b.snapshot()
                    .unwrap()
                    .sockets
                    .iter()
                    .filter(|socket| socket.bound_address.is_ipv4())
                    .all(|socket| !socket.endpoints.is_empty())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let opened = traffic.opened.load(Ordering::SeqCst);
    for manager in [&a, &b] {
        assert!(manager.snapshot().unwrap().sockets.iter().any(|socket| {
            socket.bound_address.is_ipv6()
                && matches!(socket.reference.resource, ResourceRef::Dial { .. })
        }));
    }
    let initial = listener_socket(&a);
    assert_eq!(
        initial.endpoints[0].origin,
        EndpointSource::Stun {
            observer: stun_address.to_string()
        }
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(initial, listener_socket(&a));
    assert!(a.snapshot().unwrap().links.is_empty());
    assert!(b.snapshot().unwrap().links.is_empty());
    traffic.block_traversal.store(true, Ordering::SeqCst);
    // Only actual report acknowledgement releases prepare on either Node.
    gateways[0].report(a.snapshot().unwrap()).await;
    gateways[1].report(b.snapshot().unwrap()).await;
    {
        use meshway_control::connectivity::{Outcome, prepare_request};
        let mut control = gateways[0].control.lock().await;
        let snapshot = control.execution_snapshot().unwrap();
        let Outcome::Current(discovery) = control.query_udp_traversal(&snapshot).await.unwrap()
        else {
            panic!("metadata");
        };
        let local = listener_socket(&a);
        let remote = listener_socket(&b);
        let carrier = support::carriers(&fixture.identities[0])[&ids[1]].clone();
        let mut request = prepare_request(
            &discovery.context,
            carrier,
            local.candidate(&local.endpoints[0]).unwrap(),
            remote.candidate(&remote.endpoints[0]).unwrap(),
        )
        .unwrap();
        request.peer_candidate.observation_digest = [42; 32].into();
        assert!(matches!(
            control
                .prepare_udp_traversal(&discovery.context, &request)
                .await
                .unwrap(),
            Outcome::Unavailable(_)
        ));
        assert!(snapshot.ensure_current().is_ok());
        assert!(control.status().authenticated);
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while traffic.blocked.lock().unwrap().is_none() {
            gateways[0].report_changes(a.snapshot().unwrap()).await;
            gateways[1].report_changes(b.snapshot().unwrap()).await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let blocked = traffic.blocked.lock().unwrap().clone().unwrap();
    let cancelled_digest = {
        let index = if a.snapshot().unwrap().listeners[0].local == blocked.local {
            0
        } else {
            assert_eq!(b.snapshot().unwrap().listeners[0].local, blocked.local);
            1
        };
        let active = traffic
            .authorized
            .lock()
            .unwrap()
            .get(&(blocked.local, blocked.digest))
            .expect("blocked frame must have an exact locally authorized scope")
            .clone();
        assert_eq!(
            active.validated_offer().peer().address,
            blocked.remote.to_string()
        );
        active
            .ensure_current()
            .expect("cancel a live original attempt, not an already expired observation");
        let mut control = gateways[index].control.lock().await;
        let digest = active.validated_offer().offer().digest;
        control
            .cancel_udp_traversal(active.validated_offer())
            .await
            .unwrap();
        assert!(active.ensure_current().is_err());
        assert!(a.snapshot().unwrap().links.is_empty());
        assert!(b.snapshot().unwrap().links.is_empty());
        digest
    };
    traffic.block_traversal.store(false, Ordering::SeqCst);
    blocked.waker.wake_by_ref();
    traffic.waker.wake();
    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            let left = a.snapshot().unwrap();
            let right = b.snapshot().unwrap();
            if left.links.len() == 1
                && right.links.len() == 1
                && left.links[0].id == right.links[0].id
            {
                break;
            }
            gateways[0].report_changes(left).await;
            gateways[1].report_changes(right).await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("actual candidate acknowledgements must keep running during acquisition");
    let a_link = a.snapshot().unwrap().links[0].clone();
    let b_link = b.snapshot().unwrap().links[0].clone();
    assert_eq!(a_link.id, b_link.id);
    assert_eq!(
        a_link.socket_id,
        a.snapshot().unwrap().listeners[0].socket_id
    );
    assert_eq!(
        b_link.socket_id,
        b.snapshot().unwrap().listeners[0].socket_id
    );
    assert!(observed.lock().unwrap().contains(&a_link.local));
    assert!(observed.lock().unwrap().contains(&b_link.local));
    {
        let sent = traffic.sent.lock().unwrap();
        assert!(!sent.iter().any(|(local, _, bytes)| *local == blocked.local
            && bytes.starts_with(b"MWTRV1\0\0")
            && bytes.get(8..40) == Some(cancelled_digest.as_bytes().as_slice())));
        for link in [&a_link, &b_link] {
            assert!(
                sent.iter()
                    .any(|(local, remote, bytes)| *local == link.local
                        && *remote == stun_address
                        && meshway_stun::classify(bytes).is_some())
            );
            assert!(
                sent.iter()
                    .any(|(local, remote, bytes)| *local == link.local
                        && *remote == link.remote
                        && bytes.starts_with(b"MWTRV1\0\0"))
            );
        }
    }
    {
        let active = traffic
            .channels
            .lock()
            .unwrap()
            .iter()
            .find(|channel| channel.local == a_link.local && channel.lifetime.is_current())
            .expect("scope of the actual selected local channel")
            .attempt
            .clone();
        let offered = active.validated_offer().clone();
        let mut control = gateways[0].control.lock().await;
        control.cancel_udp_traversal(&offered).await.unwrap();
        assert!(active.ensure_current().is_err());
        // Server cancellation closes acquisition, not either authenticated link.
        assert_eq!(a.snapshot().unwrap().links[0].id, a_link.id);
        assert_eq!(b.snapshot().unwrap().links[0].id, b_link.id);
    }
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
        panic!("Noise responder");
    };
    b.send(response).await.unwrap();
    let (link, received) = frame(&mut a).await;
    assert!(matches!(
        secure_a.receive(&link.carrier, received.payload).unwrap(),
        meshway_dataplane::secure::Received::Established { .. }
    ));
    stop.cancel();
    service.await.unwrap();
    // Establish an actual reply flow before independently publishing five DIDs.
    // There is no STUN service for any subsequent reacquisition to fall back on.
    reattachment::exchange(
        &a,
        &mut b,
        &mut secure_a,
        &mut secure_b,
        circuit,
        reattachment::packet(&fixture, false),
    )
    .await;
    let socket_before = listener_socket(&a);
    let old_scope = a.shared.links.lock().unwrap()[&ids[1]].scope.clone();
    let sender = a.shared.links.lock().unwrap()[&ids[1]].sender.clone();
    let pending = secure_a
        .send(circuit, reattachment::packet(&fixture, false))
        .unwrap();
    traffic.block_data.store(true, Ordering::SeqCst);
    let mut unpolled = Box::pin(meshway_dataplane::carrier::send_shared(
        &sender,
        pending,
        a_link.remote,
        Arc::new(old_scope.clone()),
    ));
    assert!(futures_util::poll!(unpolled.as_mut()).is_pending());
    tokio::time::timeout(Duration::from_secs(1), async {
        while !traffic.data_blocked.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let writes = traffic.data_writes.load(Ordering::SeqCst);
    let mut demands = Vec::new();
    for index in 0..5 {
        let request_id = support::request();
        {
            let mut control = gateways[0].control.lock().await;
            let instance = control.execution_snapshot().unwrap().instance();
            let response = control
                .create_demand(&meshway_protocol::DemandRequest {
                    instance,
                    request_id,
                    target: meshway_configuration::execution::BusinessTarget::Mesh { node: ids[1] },
                    protocol: meshway_configuration::execution::BusinessProtocol::Udp,
                    port: Some(42424.try_into().unwrap()),
                    profile: "mesh".into(),
                    duration_seconds: 120,
                })
                .await
                .unwrap();
            assert!(matches!(
                response.decision,
                meshway_configuration::execution::DemandDecision::Permitted { .. }
            ));
        }
        demands.push(request_id);
        fixture.identities[0] = gateways[0].synchronize(&mut a, &mut secure_a).await;
        tokio::time::sleep(Duration::from_millis(25)).await;
        fixture.identities[1] = gateways[1].synchronize(&mut b, &mut secure_b).await;
        if index == 0 {
            assert!(old_scope.snapshot.ensure_current().is_err());
            assert!(old_scope.snapshot.retention_deadline().is_ok());
            traffic.block_data.store(false, Ordering::SeqCst);
            traffic.waker.wake();
            tokio::time::sleep(Duration::from_millis(25)).await;
            assert_eq!(
                traffic.data_writes.load(Ordering::SeqCst),
                writes,
                "an unpolled old output must not revive under a new physical gate"
            );
        }
        assert_eq!(a.snapshot().unwrap().links[0], a_link);
        assert_eq!(b.snapshot().unwrap().links[0], b_link);
        assert_eq!(traffic.opened.load(Ordering::SeqCst), opened);
        assert_eq!(traffic.dropped.load(Ordering::SeqCst), 0);
        let socket = listener_socket(&a);
        assert_eq!(
            socket.reference.socket_id,
            socket_before.reference.socket_id
        );
        assert_eq!(
            socket.reference.socket_generation,
            socket_before.reference.socket_generation
        );
        assert_ne!(
            socket.reference.configuration,
            socket_before.reference.configuration
        );
        assert!(
            socket
                .endpoints
                .iter()
                .all(|observation| socket_before.endpoints.contains(observation)),
            "handoff cannot create or retimestamp an observation"
        );
        for observation in &socket_before.endpoints {
            if observation.expires_at.timestamp_millis()
                > i64::try_from(fixture.identities[0].snapshot().now_ms().unwrap()).unwrap()
            {
                assert!(
                    socket.endpoints.contains(observation),
                    "live observations survive handoff unchanged"
                );
            }
        }
        // A only receives business traffic; only signed Pongs renew link health.
        reattachment::exchange(
            &b,
            &mut a,
            &mut secure_b,
            &mut secure_a,
            circuit,
            reattachment::packet(&fixture, true),
        )
        .await;
    }
    drop(unpolled);
    for request_id in &demands {
        {
            let mut control = gateways[0].control.lock().await;
            let lease = control.renew_instance().await.unwrap();
            control
                .renew_demand(&meshway_protocol::RenewDemandRequest {
                    instance: meshway_protocol::InstanceRequest {
                        instance_id: lease.instance_id,
                        generation: lease.generation,
                    },
                    request_id: *request_id,
                    expires_at_ms: lease.expires_at_ms,
                })
                .await
                .unwrap();
        }
        fixture.identities[0] = gateways[0].synchronize(&mut a, &mut secure_a).await;
        fixture.identities[1] = gateways[1].synchronize(&mut b, &mut secure_b).await;
        assert_eq!(a.snapshot().unwrap().links[0], a_link);
        assert_eq!(b.snapshot().unwrap().links[0], b_link);
        reattachment::exchange(
            &b,
            &mut a,
            &mut secure_b,
            &mut secure_a,
            circuit,
            reattachment::packet(&fixture, true),
        )
        .await;
    }
    // C cancels its independent C->B demand. A/B receive a direct publication
    // successor while their own established A->B business authority survives.
    // Local Control::cancel_demand intentionally hard-fences its own lineage.
    // C has no runtime pump: explicitly renew its real instance before its
    // final operation and the later candidate-expiry wait. Otherwise a slow
    // full-workspace run can expire this unused helper before strict shutdown.
    let instance = fixture.controls[0].renew_instance().await.unwrap();
    fixture.controls[0]
        .cancel_demand(&meshway_protocol::CancelDemandRequest {
            instance: meshway_protocol::InstanceRequest {
                instance_id: instance.instance_id,
                generation: instance.generation,
            },
            request_id: fixture.demands[1],
        })
        .await
        .unwrap();
    fixture.identities[0] = gateways[0].synchronize(&mut a, &mut secure_a).await;
    fixture.identities[1] = gateways[1].synchronize(&mut b, &mut secure_b).await;
    assert_eq!(a.snapshot().unwrap().links[0], a_link);
    assert_eq!(b.snapshot().unwrap().links[0], b_link);
    reattachment::exchange(
        &b,
        &mut a,
        &mut secure_b,
        &mut secure_a,
        circuit,
        reattachment::packet(&fixture, true),
    )
    .await;
    // The ten-second grant and twenty-second candidate expiration do not own
    // the selected channel or the established end-to-end circuit.
    tokio::time::sleep(Duration::from_secs(21)).await;
    assert_eq!(a.snapshot().unwrap().links[0].id, a_link.id);
    assert_eq!(b.snapshot().unwrap().links[0].id, b_link.id);
    assert!(
        secure_a
            .ready_circuits()
            .unwrap()
            .iter()
            .any(|ready| ready.id == circuit)
    );
    let original = listener_socket(&a).reference;
    fixture.change_udp_buffer(ids[0]).await;
    let incompatible = {
        let mut control = gateways[0].control.lock().await;
        control.refresh_configuration().await.unwrap();
        control
            .dataplane_identity(control.execution_snapshot().unwrap())
            .unwrap()
    };
    assert!(
        !a.can_reconcile(&incompatible),
        "changed physical options must replace the owner"
    );
    a.reconcile(incompatible.clone()).await.unwrap();
    assert_ne!(listener_socket(&a).reference.socket_id, original.socket_id);
    assert_ne!(
        listener_socket(&a).reference.socket_generation,
        original.socket_generation
    );
    fixture.identities[0] = incompatible;
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    let mut replacement = Manager::start_with_traversal(
        fixture.identities[0].clone(),
        factory,
        Default::default(),
        gateways[0].clone(),
        generation.checked_next().unwrap(),
    )
    .await
    .unwrap();
    let new = listener_socket(&replacement).reference;
    assert_ne!(original.socket_id, new.socket_id);
    assert_ne!(original.socket_generation, new.socket_generation);
    assert!(listener_socket(&replacement).endpoints.is_empty());
    replacement.shutdown().await.unwrap();
    let last = fixture.identities[0].snapshot().clone();
    gateways[0].control.lock().await.stop().await.unwrap();
    assert!(
        last.retention_deadline().is_err(),
        "a hard Control fence ends inactive FD retention too"
    );
    {
        let mut control = gateways[0].control.lock().await;
        control.authenticate().await.unwrap();
        control
            .acquire_instance(crate::capabilities())
            .await
            .unwrap();
        control.refresh_configuration().await.unwrap();
        let next = control.execution_snapshot().unwrap();
        assert_ne!(last.instance(), next.instance());
        assert!(next.continuation_from(&last).is_err());
    }
    drop(a);
    drop(b);
    drop(replacement);
    for gateway in gateways {
        Arc::try_unwrap(gateway)
            .ok()
            .expect("joined gateway")
            .control
            .into_inner()
            .shutdown()
            .await
            .unwrap();
    }
    fixture.close().await;
}

#[tokio::test]
async fn revoked_then_restored_access_acquires_fresh_authenticated_channels_and_circuit() {
    let stun = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let stun_address = public(stun.local_addr().unwrap());
    let stop = CancellationToken::new();
    let stopped = stop.clone();
    let service = tokio::spawn(async move {
        let mut bytes = [0; meshway_stun::MAX_DATAGRAM_SIZE];
        loop {
            let (len, source) = tokio::select! { _ = stopped.cancelled() => break, result = stun.recv_from(&mut bytes) => result.unwrap() };
            let request = meshway_stun::parse_request(&bytes[..len]).unwrap();
            stun.send_to(
                &meshway_stun::encode_response(&request, public(source)),
                source,
            )
            .await
            .unwrap();
        }
    });
    let mut fixture = support::Fixture::traversal(stun_address).await;
    let ids = fixture.ids();
    let sockets = [
        fixture.udp.take().unwrap(),
        fixture.reverse_udp.take().unwrap(),
    ]
    .into_iter()
    .map(|socket| (socket.local_addr().unwrap(), socket))
    .collect();
    let factory = Arc::new(NatFactory {
        sockets: Mutex::new(sockets),
        stun: stun_address,
        traffic: Arc::new(Traffic::default()),
    });
    let mut gateways = Vec::new();
    for mut control in fixture.controls.drain(..2) {
        control.acknowledge_saved().await.unwrap();
        let ticket = control.begin_application().await.unwrap();
        control
            .finish_application(&ticket, Ok(()), chrono::Utc::now())
            .await
            .unwrap();
        gateways.push(Arc::new(Gateway {
            control: AsyncMutex::new(control),
            acknowledged: Mutex::new(Vec::new()),
        }));
    }
    let generation = Sequence::MIN.checked_next().unwrap();
    let mut a = Manager::start_with_traversal(
        fixture.identities[0].clone(),
        factory.clone(),
        Default::default(),
        gateways[0].clone(),
        generation,
    )
    .await
    .unwrap();
    let mut b = Manager::start_with_traversal(
        fixture.identities[1].clone(),
        factory,
        Default::default(),
        gateways[1].clone(),
        generation,
    )
    .await
    .unwrap();
    let mut sa = meshway_dataplane::secure::SecureNode::new(
        fixture.identities[0].clone(),
        Default::default(),
        BTreeMap::new(),
    )
    .unwrap();
    let mut sb = meshway_dataplane::secure::SecureNode::new(
        fixture.identities[1].clone(),
        Default::default(),
        BTreeMap::new(),
    )
    .unwrap();
    let mut old = None;
    for restored in [false, true] {
        if restored {
            fixture.set_access(false).await;
            fixture.identities[0] = gateways[0].install(&mut a, &mut sa, false).await;
            fixture.identities[1] = gateways[1].install(&mut b, &mut sb, false).await;
            assert!(a.snapshot().unwrap().links.is_empty());
            assert!(b.snapshot().unwrap().links.is_empty());
            assert!(sa.ready_circuits().unwrap().is_empty());
            fixture.set_access(true).await;
            fixture.identities[0] = gateways[0].install(&mut a, &mut sa, false).await;
            fixture.identities[1] = gateways[1].install(&mut b, &mut sb, false).await;
        }
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                let left = a.snapshot().unwrap();
                let right = b.snapshot().unwrap();
                if left.links.len() == 1
                    && right.links.len() == 1
                    && left.links[0].id == right.links[0].id
                {
                    break;
                }
                gateways[0].report_changes(left).await;
                gateways[1].report_changes(right).await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("acquisition after real Server authorization");
        let channel = a.snapshot().unwrap().links[0].id;
        sa.set_carriers(a.carriers().unwrap()).unwrap();
        sb.set_carriers(b.carriers().unwrap()).unwrap();
        let init = sa.start(fixture.demands[0], vec![ids[0], ids[1]]).unwrap();
        let cid = init.circuit();
        a.send(init).await.unwrap();
        let (link, received) = frame(&mut b).await;
        let meshway_dataplane::secure::Received::Established {
            response: Some(response),
            ..
        } = sb.receive(&link.carrier, received.payload).unwrap()
        else {
            panic!("fresh responder");
        };
        b.send(response).await.unwrap();
        let (link, received) = frame(&mut a).await;
        assert!(matches!(
            sa.receive(&link.carrier, received.payload).unwrap(),
            meshway_dataplane::secure::Received::Established { .. }
        ));
        reattachment::exchange(
            &a,
            &mut b,
            &mut sa,
            &mut sb,
            cid,
            reattachment::packet(&fixture, false),
        )
        .await;
        if let Some((old_channel, old_cid)) = old {
            assert_ne!(channel, old_channel);
            assert_ne!(cid, old_cid);
        }
        old = Some((channel, cid));
    }
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    drop(a);
    drop(b);
    stop.cancel();
    service.await.unwrap();
    for gateway in gateways {
        Arc::try_unwrap(gateway)
            .ok()
            .unwrap()
            .control
            .into_inner()
            .shutdown()
            .await
            .unwrap();
    }
    fixture.close().await;
}
