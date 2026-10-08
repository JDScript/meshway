use super::*;
use meshway_configuration::execution::DialEndpointSource;
use meshway_model::state::*;

// Documentation-address State fixtures use a loopback raw-I/O namespace. The
// real Server, execution authority, provider TLS, adjacency and packets run
// unchanged; only the SocketFactory translates the test's underlay address.
struct LocalFactory {
    inner: Arc<Factory>,
    attempted: Mutex<Vec<DialEndpointSource>>,
}
impl SocketFactory for LocalFactory {
    fn connect<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
        index: usize,
    ) -> BoxFuture<'a, Result<AcceptedSocket, Error>> {
        Box::pin(async move {
            let endpoint = &snapshot
                .configuration()
                .map_err(|_| Error::Authority)?
                .dialers[key]
                .endpoints[index];
            self.attempted.lock().unwrap().push(endpoint.source);
            if endpoint.source == DialEndpointSource::Configured {
                return Err(Error::Unavailable);
            }
            let stream = TcpStream::connect(("127.0.0.1", endpoint.port.get())).await?;
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
        self.inner.listen(snapshot, key)
    }
    fn datagram_listener<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn Datagram>, Error>> {
        self.inner.datagram_listener(snapshot, key)
    }
    fn datagram_candidate<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
        index: usize,
    ) -> BoxFuture<'a, Result<Vec<DatagramSocket>, Error>> {
        Box::pin(async move {
            let endpoint = &snapshot
                .configuration()
                .map_err(|_| Error::Authority)?
                .dialers[key]
                .endpoints[index];
            self.attempted.lock().unwrap().push(endpoint.source);
            if endpoint.source == DialEndpointSource::Configured {
                return Err(Error::Unavailable);
            }
            let socket = UdpSocket::bind("127.0.0.1:0").await?;
            Ok(vec![DatagramSocket {
                endpoint: Box::new(UdpTransport::adopt(socket, &Default::default()).unwrap()),
                targets: vec![SocketAddr::from(([127, 0, 0, 1], endpoint.port.get()))],
            }])
        })
    }
}

async fn report_listener(fixture: &mut support::Fixture, protocol: TransportProtocol) {
    let control = &mut fixture.controls[1];
    let snapshot = control.execution_snapshot().unwrap();
    let cfg = snapshot.configuration().unwrap();
    let reference = snapshot.reference();
    let (key, listener) = cfg
        .listeners
        .iter()
        .find(|(_, listener)| {
            matches!(
                (listener.declaration(), protocol),
                (Listener::NativeUdp(_), TransportProtocol::NativeUdp)
                    | (Listener::Vless(_), TransportProtocol::Vless)
            )
        })
        .unwrap();
    let port = match listener.declaration() {
        Listener::NativeUdp(listener) => listener.listen_port,
        Listener::Vless(listener) => listener.listen_port,
        _ => unreachable!(),
    };
    let state = NodeState {
        system: SystemState {
            hostname: cfg.hostname.clone(),
            os: "test".into(),
            arch: "test".into(),
            agent_version: "test".into(),
        },
        capabilities: cfg.self_keys.capabilities.clone(),
        configuration: ConfigurationState {
            saved: Some(reference.clone()),
            applied: Some(reference.clone()),
            progress: None,
        },
        listeners: BTreeMap::from([(
            key.clone(),
            ListenerState {
                phase: ListenerPhase::Listening,
                socket_ids: vec!["listener".into()],
                loaded_certificate_sha256: None,
                problem: None,
                configuration: Some(reference.clone()),
            },
        )]),
        sockets: BTreeMap::from([(
            "listener".into(),
            SocketState {
                resource: ResourceRef::Listener { key: key.clone() },
                configuration: Some(reference),
                generation: meshway_model::Sequence::new(1).unwrap(),
                network: if protocol == TransportProtocol::NativeUdp {
                    SocketNetwork::Udp
                } else {
                    SocketNetwork::Tcp
                },
                bound_address: SocketAddr::from(([0, 0, 0, 0], port.get())),
                endpoints: vec![],
                local_endpoints: Some(vec![LocalEndpoint {
                    address: SocketAddr::from(([192, 0, 2, 2], port.get())),
                    source: LocalEndpointSource::Local,
                }]),
            },
        )]),
        links: BTreeMap::new(),
        paths: BTreeMap::new(),
        selection: Default::default(),
        probes: vec![],
        problems: vec![],
        exit_capability: None,
    };
    let ticket = control.begin_application().await.unwrap();
    control
        .finish_application(&ticket, Ok(()), chrono::Utc::now())
        .await
        .unwrap();
    control
        .report_runtime_state(state, chrono::Utc::now())
        .await
        .unwrap();
}

async fn connect(protocol: TransportProtocol, stale: bool) {
    let mut fixture = if protocol == TransportProtocol::Vless {
        streaming::fixture(protocol).await
    } else {
        support::Fixture::new(false, false).await
    };
    let peer = fixture.ids()[1];
    fixture
        .configure(|cfg| {
            for listener in cfg.nodes.get_mut(&peer).unwrap().listeners.values_mut() {
                let endpoints = match listener {
                    Listener::NativeUdp(listener) => &mut listener.endpoints,
                    Listener::Vless(listener) => &mut listener.endpoints,
                    _ => continue,
                };
                if !stale {
                    endpoints.clear();
                }
            }
        })
        .await;
    for index in 0..3 {
        fixture.refresh(index).await;
    }
    let factory = Arc::new(LocalFactory {
        inner: Factory::new(&mut fixture, Arc::new(Faults::default())),
        attempted: Mutex::new(vec![]),
    });
    let mut b = fixture.manager(1, factory.clone()).start().await;
    report_listener(&mut fixture, protocol).await;
    fixture.refresh(0).await;
    let mut a = fixture.manager(0, factory.clone()).start().await;
    linked(&a, 1, protocol).await;
    linked(&b, 1, protocol).await;
    assert_eq!(
        a.snapshot().unwrap().links[0].acquisition,
        LinkAcquisition::LocalEndpoint
    );
    assert_eq!(
        *factory.attempted.lock().unwrap(),
        if stale {
            vec![DialEndpointSource::Configured, DialEndpointSource::Local]
        } else {
            vec![DialEndpointSource::Local]
        }
    );
    let mut sa = reattachment::secure(&fixture.identities[0], &a);
    let mut sb = reattachment::secure(&fixture.identities[1], &b);
    let cid = reattachment::handshake(
        &mut a,
        &mut b,
        &mut sa,
        &mut sb,
        fixture.demands[0],
        vec![fixture.ids()[0], peer],
    )
    .await;
    reattachment::exchange(
        &a,
        &mut b,
        &mut sa,
        &mut sb,
        cid,
        reattachment::packet(&fixture, false),
    )
    .await;
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn native_udp_uses_reported_local_candidate_without_configured_endpoint() {
    connect(TransportProtocol::NativeUdp, false).await;
}
#[tokio::test]
async fn vless_uses_reported_local_candidate_without_configured_endpoint() {
    connect(TransportProtocol::Vless, false).await;
}
#[tokio::test]
async fn stale_configured_endpoints_fall_through_to_local_udp_and_vless() {
    connect(TransportProtocol::NativeUdp, true).await;
    connect(TransportProtocol::Vless, true).await;
}

#[tokio::test]
async fn configured_tcp_precedes_higher_protocol_priority_local_udp() {
    let mut fixture = support::Fixture::new(true, false).await;
    let peer = fixture.ids()[1];
    fixture
        .configure(|cfg| {
            for listener in cfg.nodes.get_mut(&peer).unwrap().listeners.values_mut() {
                if let Listener::NativeUdp(listener) = listener {
                    listener.endpoints.clear();
                }
            }
        })
        .await;
    for index in 0..3 {
        fixture.refresh(index).await;
    }
    let faults = Arc::new(Faults::default());
    faults.blackhole.store(true, Ordering::SeqCst);
    let factory = Factory::new(&mut fixture, faults);
    let mut b = fixture.manager(1, factory.clone()).start().await;
    report_listener(&mut fixture, TransportProtocol::NativeUdp).await;
    fixture.refresh(0).await;
    let mut a = fixture.manager(0, factory.clone()).start().await;
    linked(&a, 1, TransportProtocol::NativeTcp).await;
    linked(&b, 1, TransportProtocol::NativeTcp).await;
    assert_eq!(
        factory.acquisitions.load(Ordering::SeqCst),
        1,
        "local UDP must not precede the configured TCP fallback"
    );
    assert_eq!(
        a.snapshot().unwrap().links[0].acquisition,
        LinkAcquisition::PublishedEndpoint
    );
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}
