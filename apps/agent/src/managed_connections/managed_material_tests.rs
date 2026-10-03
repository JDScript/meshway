use super::*;
use crate::managed_dataplane::{
    PacketPlan, TunOptions,
    tests::{MockDevice, fixtures, packet, snapshot},
    tun::Tun,
};
use meshway_protocol::*;
use sha2::{Digest, Sha256};

fn material(id: &str, mode: VlessStreamTransport) -> ManagedVlessMaterial {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["meshway.test".into()]).unwrap();
    ManagedVlessMaterial {
        listener_id: id.into(),
        credential_ref: format!("managed.{id}"),
        version: 1,
        settings: ManagedVlessSettings {
            stream: mode,
            server_name: "meshway.test".into(),
            websocket_path: (mode == VlessStreamTransport::Websocket).then(|| "/meshway".into()),
            websocket_host: (mode == VlessStreamTransport::Websocket)
                .then(|| "meshway.test".into()),
        },
        uuid: "b590664f-5111-4c8b-a481-82594488df9e".into(),
        certificate_pem: cert.pem(),
        certificate_sha256: Sha256::digest(cert.der())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        private_key_pem: Some(signing_key.serialize_pem()),
    }
}

#[tokio::test]
async fn managed_tcp_and_websocket_tls_noise_deliver_real_ipv4_both_directions_and_expire() {
    for mode in [VlessStreamTransport::Tcp, VlessStreamTransport::Websocket] {
        let (mut payloads, keys) = fixtures(2);
        let mut all = Vec::new();
        for payload in &mut payloads {
            for listener in &mut payload.connections.listeners {
                listener.transport = ConnectionTransport::Vless;
                listener.credential_ref = Some(format!("managed.{}", listener.id));
                all.push(material(&listener.id, mode));
            }
            for candidate in &mut payload.connections.outgoing {
                candidate.transport = ConnectionTransport::Vless;
                candidate.credential_ref = Some(format!("managed.{}", candidate.listener_id));
            }
            for approval in &mut payload.connections.incoming {
                approval.transport = ConnectionTransport::Vless;
            }
        }
        let mut managers = Vec::new();
        let mut sends = Vec::new();
        let mut receives = Vec::new();
        for i in 0..2 {
            let payload = &payloads[i];
            let mut materials: Vec<_> = all
                .iter()
                .filter(|m| {
                    payload
                        .connections
                        .listeners
                        .iter()
                        .any(|l| l.id == m.listener_id)
                        || payload
                            .connections
                            .outgoing
                            .iter()
                            .any(|c| c.listener_id == m.listener_id)
                })
                .cloned()
                .collect();
            for item in &mut materials {
                if !payload
                    .connections
                    .listeners
                    .iter()
                    .any(|l| l.id == item.listener_id)
                {
                    item.private_key_pem = None;
                }
            }
            let (device, send, receive) = MockDevice::new();
            device.allow_writes();
            let mut manager = Manager::new_dataplane(
                None,
                Duration::from_secs(5),
                TunOptions {
                    name: "mock0".into(),
                    mtu: 1000,
                },
                "http://127.0.0.1".into(),
                false,
            );
            manager.mock_tun(Tun::mock(
                device,
                &PacketPlan::compile(payload, 1000).unwrap(),
            ));
            manager
                .validate_materials(&payload.connections, &materials)
                .unwrap();
            manager.install_materials(materials);
            manager
                .reconcile_dataplane(snapshot(payload.clone()), keys[i].clone(), Instant::now())
                .await
                .unwrap();
            assert_eq!(manager.observation().state, Phase::Applied);
            managers.push(manager);
            sends.push(send);
            receives.push(receive);
        }
        for (source, target) in [(0, 1), (1, 0)] {
            let bytes = packet(
                &payloads[source].node.mesh_ipv4,
                &payloads[target].node.mesh_ipv4,
                1,
            );
            time::timeout(Duration::from_secs(4),async{loop{sends[source].send(bytes.clone()).await.unwrap();tokio::select!{value=receives[target].recv()=>{assert_eq!(value,Some(bytes.clone()));break;},_=time::sleep(Duration::from_millis(50))=>{}}}}).await.expect("TLS/VLESS/Noise must deliver real IPv4 packet");
        }
        // Active TLS connections leave TIME_WAIT when the generation stops.
        // Reconfigure on the same bind, with fresh WS path/material version,
        // and require real IPv4 delivery again rather than retrying a failed bind.
        for i in 0..2 {
            payloads[i].revision += 1;
            payloads[i].connections.revision += 1;
            for material in &mut managers[i].managed_materials {
                material.version += 1;
                if mode == VlessStreamTransport::Websocket {
                    material.settings.websocket_path = Some("/changed".into());
                }
            }
            managers[i]
                .reconcile_dataplane(
                    snapshot(payloads[i].clone()),
                    keys[i].clone(),
                    Instant::now(),
                )
                .await
                .unwrap();
            assert_eq!(managers[i].observation().state, Phase::Applied);
        }
        for (source, target) in [(0, 1), (1, 0)] {
            // Distinguish replacement traffic from the initial ICMP echo using
            // UDP, which is part of the current stateful IPv4 capability.
            let bytes = packet(
                &payloads[source].node.mesh_ipv4,
                &payloads[target].node.mesh_ipv4,
                17,
            );
            time::timeout(Duration::from_secs(4),async{loop{sends[source].send(bytes.clone()).await.unwrap();tokio::select!{value=receives[target].recv()=>{assert_eq!(value,Some(bytes.clone()));break;},_=time::sleep(Duration::from_millis(50))=>{}}}}).await.expect("replacement TLS generation must rebind and deliver");
        }
        time::timeout(Duration::from_secs(6), async {
            loop {
                if managers
                    .iter()
                    .all(|m| m.observation().state == Phase::Withdrawn)
                {
                    break;
                }
                time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        for manager in &mut managers {
            manager.withdraw(Reason::ShuttingDown).await;
        }
        for payload in payloads {
            for listener in payload.connections.listeners {
                assert!(tokio::net::TcpListener::bind(&listener.bind).await.is_ok());
            }
        }
    }
}

#[tokio::test]
async fn websocket_wrong_uuid_host_path_and_noise_peer_key_never_authenticate() {
    for bad in 0..4 {
        let (mut server, skey, mut client, ckey) = tests::fixture(ConnectionTransport::Vless);
        let id = server.listeners[0].id.clone();
        let reference = format!("managed.{id}");
        server.listeners[0].credential_ref = Some(reference.clone());
        client.outgoing[0].credential_ref = Some(reference);
        let owner = material(&id, VlessStreamTransport::Websocket);
        let mut dialer = owner.clone();
        dialer.private_key_pem = None;
        match bad {
            0 => dialer.uuid = "00000000-0000-4000-a000-000000000000".into(),
            1 => dialer.settings.websocket_path = Some("/wrong".into()),
            2 => dialer.settings.websocket_host = Some("wrong.test".into()),
            3 => {
                client.peers[0].public_key = meshway_dataplane::Identity::generate()
                    .unwrap()
                    .public_key_hex()
            }
            _ => unreachable!(),
        }
        let targets = resolve_targets(&client, &sockets::SystemUnderlay)
            .await
            .unwrap_or_else(|_| panic!("targets"));
        let mut a = session::Session::start_prepared(
            &server,
            profiles::load_combined(None, &server, &[owner])
                .unwrap_or_else(|_| panic!("server material")),
            &BTreeMap::new(),
            core_config(&server, skey, &BTreeMap::new()).unwrap(),
            std::sync::Arc::new(sockets::SystemUnderlay),
        )
        .await
        .unwrap_or_else(|_| panic!("server start"));
        let mut b = session::Session::start_prepared(
            &client,
            profiles::load_combined(None, &client, &[dialer])
                .unwrap_or_else(|_| panic!("client material")),
            &targets,
            core_config(&client, ckey, &targets).unwrap(),
            std::sync::Arc::new(sockets::SystemUnderlay),
        )
        .await
        .unwrap_or_else(|_| panic!("client start"));
        assert!(
            b.runtime
                .wait_for_peer(&server.node_id, Duration::from_millis(650))
                .await
                .is_err()
        );
        assert!(a.runtime.connected_peers().is_empty());
        a.shutdown().await;
        b.shutdown().await;
    }
}
