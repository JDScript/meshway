use super::*;

#[tokio::test]
async fn incompatible_listener_binds_exact_dial_family_and_capacity_failure_releases_sockets() {
    use meshway_configuration::Input;
    use meshway_transport_spec::{DialerSet, NativeUdpDialer};
    let stun = "198.51.100.1:3478".parse().unwrap();
    let mut fixture = fixture::Fixture::traversal(stun).await;
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
    let mut fixture = fixture::Fixture::traversal(stun).await;
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
