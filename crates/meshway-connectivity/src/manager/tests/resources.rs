use super::*;
use meshway_configuration::{
    AccessRule,
    execution::{BusinessProtocol, BusinessTarget},
};
use meshway_dataplane::secure::{Received, SecureNode};
use meshway_transport_spec::{
    Endpoint as PublishedEndpoint, NativeTcpDialer, NativeTcpListener, NativeUdpListener,
};

pub(super) async fn setup(protocol: TransportProtocol) -> support::Fixture {
    let stream = protocol == TransportProtocol::NativeTcp;
    let mut fixture = support::Fixture::new(stream, false).await;
    let ids = fixture.ids();
    let mut addresses = Vec::new();
    for index in [0, 2] {
        let address = if stream {
            let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = socket.local_addr().unwrap();
            fixture.extra_tcp.push(socket);
            address
        } else {
            let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let address = socket.local_addr().unwrap();
            fixture.extra_udp.push(socket);
            address
        };
        addresses.push((ids[index], address));
    }
    fixture
        .configure(|cfg| {
            cfg.defaults.topology = meshway_configuration::TopologyDefault::Deny;
            cfg.topology = [0, 1]
                .into_iter()
                .map(|index| meshway_configuration::TopologyRule {
                    remark: String::new(),
                    action: meshway_configuration::TopologyAction::Allow,
                    a: format!("node:{}", ids[index]),
                    b: format!("node:{}", ids[2]),
                })
                .collect();
            if stream {
                cfg.defaults.dialers.native_udp = None;
                cfg.defaults.dialers.native_tcp = Some(NativeTcpDialer::default());
                for node in cfg.nodes.values_mut() {
                    node.listeners
                        .retain(|_, listener| matches!(listener, Listener::NativeTcp(_)));
                }
            }
            for (node, address) in addresses {
                let endpoints = vec![PublishedEndpoint {
                    host: address.ip().to_string(),
                    port: address.port().try_into().unwrap(),
                }];
                let listener = if stream {
                    Listener::NativeTcp(NativeTcpListener {
                        enabled: true,
                        remark: String::new(),
                        listen: address.ip(),
                        listen_port: address.port().try_into().unwrap(),
                        endpoints,
                        bind_interface: String::new(),
                        socket: Default::default(),
                        tcp: Default::default(),
                    })
                } else {
                    Listener::NativeUdp(NativeUdpListener {
                        enabled: true,
                        remark: String::new(),
                        listen: address.ip(),
                        listen_port: address.port().try_into().unwrap(),
                        endpoints,
                        bind_interface: String::new(),
                        socket: Default::default(),
                    })
                };
                cfg.nodes
                    .get_mut(&node)
                    .unwrap()
                    .listeners
                    .insert(if stream { "tcp" } else { "udp" }.into(), listener);
            }
            cfg.access.push(direct_rule(&ids));
        })
        .await;
    for index in 0..3 {
        fixture.refresh(index).await;
    }
    fixture
}

pub(super) fn direct_rule(ids: &[NodeId]) -> AccessRule {
    AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", ids[0])],
        dst: vec![format!("node:{}", ids[2])],
        ip: vec!["udp:42424".into()],
        bidirectional: false,
    }
}

async fn handshake(
    from: &Manager,
    to: &mut Manager,
    source: &mut SecureNode,
    target: &mut SecureNode,
    demand: meshway_model::RequestId,
    path: Vec<NodeId>,
) -> [u8; 32] {
    let pending = source.start(demand, path).unwrap();
    let cid = pending.circuit();
    from.send(pending).await.unwrap();
    let (link, frame) = frame(to).await;
    let Received::Established {
        response: Some(response),
        ..
    } = target.receive(&link.carrier, frame.payload).unwrap()
    else {
        panic!("real Noise response")
    };
    to.send(response).await.unwrap();
    cid
}

async fn churn(protocol: TransportProtocol, relay_dials: bool) {
    let mut fixture = setup(protocol).await;
    let ids = fixture.ids();
    let direct = support::request();
    let instance = fixture.controls[0].execution_snapshot().unwrap().instance();
    fixture.controls[0]
        .create_demand(&meshway_protocol::DemandRequest {
            instance,
            request_id: direct,
            target: BusinessTarget::Mesh { node: ids[2] },
            protocol: BusinessProtocol::Udp,
            port: Some(42424.try_into().unwrap()),
            profile: "mesh".into(),
            duration_seconds: 120,
        })
        .await
        .unwrap();
    for index in 0..3 {
        fixture.refresh(index).await;
    }
    let faults = Arc::new(Faults::default());
    let factory = Factory::new(&mut fixture, faults);
    factory
        .blocked_dialers
        .lock()
        .unwrap()
        .insert(if relay_dials { ids[0] } else { ids[2] });
    let mut managers = Vec::new();
    for identity in &fixture.identities {
        managers.push(
            Manager::start(identity.clone(), factory.clone(), Default::default())
                .await
                .unwrap(),
        );
    }
    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            if managers[2].carriers().unwrap().contains_key(&ids[0])
                && managers[2].carriers().unwrap().contains_key(&ids[1])
                && managers[0].carriers().unwrap().contains_key(&ids[2])
                && managers[1].carriers().unwrap().contains_key(&ids[2])
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let original = managers[2].shared.links.lock().unwrap()[&ids[0]]
        .fact
        .clone();
    assert_eq!(
        original.carrier.dialer,
        if relay_dials { ids[2] } else { ids[0] }
    );
    let retired = managers[2].shared.links.lock().unwrap()[&ids[1]]
        .fact
        .clone();
    let original_sender = managers[2].shared.links.lock().unwrap()[&ids[0]]
        .sender
        .clone();
    let old_scope = managers[2].shared.links.lock().unwrap()[&ids[1]]
        .scope
        .clone();
    let mut nodes: Vec<_> = fixture
        .identities
        .iter()
        .zip(&managers)
        .map(|(identity, manager)| {
            SecureNode::new(
                identity.clone(),
                Default::default(),
                manager.carriers().unwrap(),
            )
            .unwrap()
        })
        .collect();
    let initial_b = nodes[2]
        .start(fixture.demands[1], vec![ids[2], ids[1]])
        .unwrap();
    let old_output = managers[2].send_with_receipt(initial_b);
    let (a, r) = managers.split_at_mut(2);
    let (sa, sr) = nodes.split_at_mut(2);
    let cid = handshake(
        &a[0],
        &mut r[0],
        &mut sa[0],
        &mut sr[0],
        direct,
        vec![ids[0], ids[2]],
    )
    .await;
    let (link, received) = frame(&mut a[0]).await;
    assert!(matches!(
        sa[0].receive(&link.carrier, received.payload).unwrap(),
        Received::Established { .. }
    ));
    reattachment::exchange(
        &a[0],
        &mut r[0],
        &mut sa[0],
        &mut sr[0],
        cid,
        reattachment::packet_between(&fixture, 0, 2, false),
    )
    .await;

    fixture
        .configure(|cfg| cfg.access = vec![direct_rule(&ids)])
        .await;
    for index in 0..3 {
        let next = fixture.refresh(index).await;
        assert!(managers[index].can_reconcile(&next));
        nodes[index].reconcile(next.clone()).unwrap();
        managers[index].reconcile(next).await.unwrap();
        nodes[index]
            .set_carriers(managers[index].carriers().unwrap())
            .unwrap();
    }
    assert_eq!(
        managers[2].snapshot().unwrap().links,
        vec![original.clone()]
    );
    assert!(Arc::ptr_eq(
        &original_sender,
        &managers[2].shared.links.lock().unwrap()[&ids[0]].sender
    ));
    assert!(old_scope.check().is_err());
    assert!(old_output.await.is_err());
    let (a, r) = managers.split_at_mut(2);
    let (sa, sr) = nodes.split_at_mut(2);
    reattachment::exchange(
        &r[0],
        &mut a[0],
        &mut sr[0],
        &mut sa[0],
        cid,
        reattachment::packet_between(&fixture, 0, 2, true),
    )
    .await;

    fixture
        .configure(|cfg| {
            cfg.access.push(AccessRule {
                remark: String::new(),
                src: vec![format!("node:{}", ids[2])],
                dst: vec![format!("node:{}", ids[1])],
                ip: vec!["udp:42424".into()],
                bidirectional: false,
            })
        })
        .await;
    for index in 0..3 {
        let next = fixture.refresh(index).await;
        nodes[index].reconcile(next.clone()).unwrap();
        managers[index].reconcile(next).await.unwrap();
    }
    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            if managers[2].carriers().unwrap().contains_key(&ids[1])
                && managers[1].carriers().unwrap().contains_key(&ids[2])
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        managers[2].shared.links.lock().unwrap()[&ids[0]].fact,
        original
    );
    assert_ne!(
        managers[2].shared.links.lock().unwrap()[&ids[1]].fact.id,
        retired.id
    );
    for (node, manager) in nodes.iter_mut().zip(&managers) {
        node.set_carriers(manager.carriers().unwrap()).unwrap();
    }
    let (a, r) = managers.split_at_mut(2);
    let (sa, sr) = nodes.split_at_mut(2);
    reattachment::exchange(
        &a[0],
        &mut r[0],
        &mut sa[0],
        &mut sr[0],
        cid,
        reattachment::packet_between(&fixture, 0, 2, false),
    )
    .await;
    for manager in &mut managers {
        manager.shutdown().await.unwrap();
    }
    fixture.close().await;
}

#[tokio::test]
async fn udp_dependency_remove_add_preserves_selected_resource_in_both_dial_directions() {
    for relay_dials in [true, false] {
        churn(TransportProtocol::NativeUdp, relay_dials).await;
    }
}

#[tokio::test]
async fn tcp_dependency_remove_add_preserves_selected_resource_in_both_dial_directions() {
    for relay_dials in [true, false] {
        churn(TransportProtocol::NativeTcp, relay_dials).await;
    }
}
