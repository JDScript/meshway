use super::*;
use meshway_transport_spec::{
    DialerSet, ListenerTls, TlsMode, TlsVersion, TrojanListener, VlessListener, VlessStream,
};
use reattachment::{exchange, handshake, packet, secure};

pub(super) fn listener_address(listener: &Listener) -> Result<SocketAddr, Error> {
    let (ip, port) = match listener {
        Listener::NativeTcp(value) => (value.listen, value.listen_port),
        Listener::Vless(value) => (value.listen, value.listen_port),
        Listener::Trojan(value) => (value.listen, value.listen_port),
        Listener::NativeUdp(_) => return Err(Error::Configuration),
    };
    Ok(SocketAddr::new(ip, port.get()))
}

pub(super) async fn fixture(protocol: TransportProtocol) -> support::Fixture {
    let mut fixture = support::Fixture::new(true, false).await;
    fixture
        .configure(|cfg| {
            cfg.defaults.dialers = DialerSet {
                native_udp: None,
                native_tcp: None,
                vless: (protocol == TransportProtocol::Vless).then(Default::default),
                trojan: (protocol == TransportProtocol::Trojan).then(Default::default),
            };
            for node in cfg.nodes.values_mut() {
                node.listeners
                    .retain(|_, listener| matches!(listener, Listener::NativeTcp(_)));
                for listener in node.listeners.values_mut() {
                    let Listener::NativeTcp(old) = listener else {
                        unreachable!()
                    };
                    let tls = ListenerTls {
                        mode: TlsMode::Managed,
                        server_name: "localhost".into(),
                        certificate: None,
                        private_key: None,
                        min_version: TlsVersion::Tls12,
                        max_version: TlsVersion::Tls13,
                        alpn: vec![],
                        handshake_timeout: "2s".parse().unwrap(),
                    };
                    *listener = match protocol {
                        TransportProtocol::Vless => Listener::Vless(VlessListener {
                            enabled: true,
                            remark: String::new(),
                            listen: old.listen,
                            listen_port: old.listen_port,
                            endpoints: old.endpoints.clone(),
                            bind_interface: String::new(),
                            socket: Default::default(),
                            tcp: Default::default(),
                            stream: VlessStream::Tcp,
                            uuid: None,
                            flow: String::new(),
                            tls,
                            websocket: None,
                        }),
                        TransportProtocol::Trojan => Listener::Trojan(TrojanListener {
                            enabled: true,
                            remark: String::new(),
                            listen: old.listen,
                            listen_port: old.listen_port,
                            endpoints: old.endpoints.clone(),
                            bind_interface: String::new(),
                            socket: Default::default(),
                            tcp: Default::default(),
                            stream: Default::default(),
                            password: None,
                            tls,
                        }),
                        _ => unreachable!(),
                    };
                }
            }
        })
        .await;
    for index in 0..3 {
        fixture.refresh(index).await;
    }
    fixture
}

async fn continuity(protocol: TransportProtocol) {
    let mut fixture = fixture(protocol).await;
    let ids = fixture.ids();
    let factory = Factory::new(&mut fixture, Arc::new(Faults::default()));
    let mut a = Manager::start(
        fixture.identities[0].clone(),
        factory.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    let mut b = Manager::start(
        fixture.identities[1].clone(),
        factory.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    linked(&a, 1, protocol).await;
    linked(&b, 1, protocol).await;
    let before = [
        a.snapshot().unwrap().links[0].clone(),
        b.snapshot().unwrap().links[0].clone(),
    ];
    let old_scope = a.shared.links.lock().unwrap()[&ids[1]].scope.clone();
    let (mut sa, mut sb) = (
        secure(&fixture.identities[0], &a),
        secure(&fixture.identities[1], &b),
    );
    let cid = handshake(
        &mut a,
        &mut b,
        &mut sa,
        &mut sb,
        fixture.demands[0],
        vec![ids[0], ids[1]],
    )
    .await;
    exchange(&a, &mut b, &mut sa, &mut sb, cid, packet(&fixture, false)).await;
    let acquisitions = factory.acquisitions.load(Ordering::SeqCst);
    let lease = fixture.controls[0].renew_instance().await.unwrap();
    fixture.controls[0]
        .renew_demand(&meshway_protocol::RenewDemandRequest {
            instance: meshway_protocol::InstanceRequest {
                instance_id: lease.instance_id,
                generation: lease.generation,
            },
            request_id: fixture.demands[0],
            expires_at_ms: lease.expires_at_ms,
        })
        .await
        .unwrap();
    for (index, manager, node) in [(0, &mut a, &mut sa), (1, &mut b, &mut sb)] {
        fixture.controls[index]
            .refresh_configuration()
            .await
            .unwrap();
        let next = fixture.controls[index]
            .dataplane_identity(fixture.controls[index].execution_snapshot().unwrap())
            .unwrap();
        assert!(manager.can_reconcile(&next));
        node.reconcile(next.clone()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), manager.reconcile(next.clone()))
            .await
            .unwrap()
            .unwrap();
        node.set_carriers(manager.carriers().unwrap()).unwrap();
        fixture.identities[index] = next;
    }
    assert!(
        old_scope.check().is_err(),
        "old output must not borrow the next execution"
    );
    assert_eq!(a.snapshot().unwrap().links, vec![before[0].clone()]);
    assert_eq!(b.snapshot().unwrap().links, vec![before[1].clone()]);
    assert_eq!(factory.acquisitions.load(Ordering::SeqCst), acquisitions);
    exchange(&b, &mut a, &mut sb, &mut sa, cid, packet(&fixture, true)).await;
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn vless_secure_packets_survive_unchanged_material_execution_handoff() {
    continuity(TransportProtocol::Vless).await;
}

#[tokio::test]
async fn trojan_secure_packets_survive_unchanged_material_execution_handoff() {
    continuity(TransportProtocol::Trojan).await;
}
