use super::*;

#[test]
fn handshake_requires_current_complete_path_in_both_directions() {
    let (configuration, demand, traffic) = mesh(BusinessProtocol::Udp);
    let compiled = publication(&configuration, &demand, &[]);
    let path = [node(1), node(3), node(2)];
    for (position, local) in path.iter().enumerate() {
        let cfg = &compiled.nodes[local].configuration;
        for reply in [false, true] {
            let upstream = position.checked_sub(1).map(|index| path[index]);
            let downstream = path.get(position + 1).copied();
            let (previous, next) = if reply {
                (downstream, upstream)
            } else {
                (upstream, downstream)
            };
            assert_eq!(
                admit_handshake(cfg, demand.id, &path, node(1), previous, reply, 1000),
                Ok(next.map_or(Hop::Deliver, Hop::Forward))
            );
            assert_eq!(
                admit_handshake(
                    cfg,
                    demand.id,
                    &path,
                    node(1),
                    previous,
                    reply,
                    demand.expires_at_ms
                ),
                Err(AdmissionError::Expired)
            );
            assert!(
                admit_handshake(cfg, demand.id, &path, node(1), Some(node(4)), reply, 1000)
                    .is_err()
            );
        }
        assert!(
            admit_forward(
                cfg,
                demand.id,
                &path,
                node(1),
                position.checked_sub(1).map(|index| path[index]),
                &reverse(traffic),
                1000
            )
            .is_err()
        );
    }
    let cfg = &compiled.nodes[&node(3)].configuration;
    for bad in [
        vec![node(1), node(3)],
        vec![node(1), node(3), node(3), node(2)],
        vec![node(1)],
        vec![],
    ] {
        assert!(
            admit_handshake(cfg, demand.id, &bad, node(1), Some(node(1)), false, 1000).is_err()
        );
    }
    for mutation in 0..5 {
        let mut cfg = cfg.clone();
        match mutation {
            0 => cfg.enabled = false,
            1 => {
                cfg.peers.remove(&node(2));
            }
            2 => cfg.self_keys.dataplane_key.public_key.algorithm = KeyAlgorithm::Ed25519,
            3 => {
                cfg.peers
                    .get_mut(&node(1))
                    .unwrap()
                    .control_key
                    .public_key
                    .algorithm = KeyAlgorithm::X25519
            }
            4 => cfg.permits.get_mut(&demand.id).unwrap().source = node(3),
            _ => unreachable!(),
        }
        assert!(
            admit_handshake(&cfg, demand.id, &path, node(1), Some(node(1)), false, 1000).is_err()
        );
    }
}

#[test]
fn carriers_require_selected_business_edges_and_actual_socket_orientation() {
    let (configuration, demand, _) = mesh(BusinessProtocol::Udp);
    let compiled = publication(&configuration, &demand, &[]);
    let path = [node(1), node(3), node(2)];
    let selected = &compiled.nodes[&node(1)].configuration.permits[&demand.id].paths[0];
    assert_eq!(selected.nodes, path);
    for (dialer, listener) in [
        (node(1), node(3)),
        (node(3), node(1)),
        (node(3), node(2)),
        (node(2), node(3)),
    ] {
        let carrier = Carrier {
            dialer,
            listener,
            listener_key: "udp".into(),
            protocol: TransportProtocol::NativeUdp,
        };
        assert!(path_carrier(selected, &carrier));
        for (local, peer) in [(dialer, listener), (listener, dialer)] {
            let cfg = &compiled.nodes[&local].configuration;
            assert_eq!(admit_carrier(cfg, demand.id, &path, peer, &carrier), Ok(()));
            for protocol in [TransportProtocol::NativeTcp, TransportProtocol::Vless] {
                let mismatch = Carrier {
                    protocol,
                    ..carrier.clone()
                };
                assert!(admit_carrier(cfg, demand.id, &path, peer, &mismatch).is_err());
            }
            let mismatch = Carrier {
                listener_key: "other".into(),
                ..carrier.clone()
            };
            assert!(admit_carrier(cfg, demand.id, &path, peer, &mismatch).is_err());
            assert!(admit_carrier(cfg, demand.id, &path, node(4), &carrier).is_err());
            assert!(admit_carrier(cfg, demand.id, &[dialer, listener], peer, &carrier).is_err());
            let mut revoked = cfg.clone();
            revoked.permits.clear();
            assert!(admit_carrier(&revoked, demand.id, &path, peer, &carrier).is_err());
            let mut revoked = cfg.clone();
            revoked.permits.get_mut(&demand.id).unwrap().paths[0]
                .dial_candidates
                .clear();
            assert!(admit_carrier(&revoked, demand.id, &path, peer, &carrier).is_err());
        }
        for mutation in 0..3 {
            let mut cfg = compiled.nodes[&dialer].configuration.clone();
            let candidate = cfg.dialers.get_mut(&format!("{listener}/udp")).unwrap();
            match mutation {
                0 => candidate.peer = node(4),
                1 => candidate.listener_key = "other".into(),
                2 => {
                    candidate.parameters = DialParameters::NativeTcp {
                        options: NativeTcpDialer::default(),
                    }
                }
                _ => unreachable!(),
            }
            assert!(admit_carrier(&cfg, demand.id, &path, listener, &carrier).is_err());
        }
    }
    let shortcut = Carrier {
        dialer: node(1),
        listener: node(2),
        listener_key: "udp".into(),
        protocol: TransportProtocol::NativeUdp,
    };
    assert!(!path_carrier(selected, &shortcut));
}

#[test]
fn vless_selected_path_cannot_use_an_available_native_udp_carrier() {
    let (mut configuration, demand, _) = mesh(BusinessProtocol::Udp);
    for settings in configuration.nodes.values_mut() {
        let listener: Listener = serde_json::from_str(r#"{"type":"vless","listenPort":443,"uuid":"00000000-0000-4000-8000-000000000001","tls":{"serverName":"test.example","minVersion":"1.3","maxVersion":"1.3","certificate":"certificate","privateKey":"private-key"}}"#).unwrap();
        settings
            .listeners
            .insert("vless".into(), PreparedListener::prepare(listener).unwrap());
    }
    let mut trusted = inventory();
    for node in trusted.values_mut() {
        node.capabilities.insert(
            "transport:vless".into(),
            Capability::Transport {
                roles: vec![TransportRole::Dial, TransportRole::Listen],
                streams: vec![StreamKind::Tcp],
            },
        );
    }
    let mut compiled = run(
        &configuration,
        &trusted,
        &BTreeMap::from([(demand.id, demand.clone())]),
        &[],
    );
    let path = [node(1), node(3), node(2)];
    // Model a selected path restricted to VLESS while the installed dialers and
    // listeners still contain native UDP for other permitted/diagnostic uses.
    for node in compiled.nodes.values_mut() {
        if let Some(permit) = node.configuration.permits.get_mut(&demand.id) {
            for candidate in &mut permit.paths {
                for keys in candidate.dial_candidates.values_mut() {
                    keys.retain(|key| key.ends_with("/vless"));
                }
            }
        }
    }
    let carrier = Carrier {
        dialer: node(1),
        listener: node(3),
        listener_key: "vless".into(),
        protocol: TransportProtocol::Vless,
    };
    for (local, peer) in [(node(1), node(3)), (node(3), node(1))] {
        let cfg = &compiled.nodes[&local].configuration;
        assert_eq!(admit_carrier(cfg, demand.id, &path, peer, &carrier), Ok(()));
        let udp = Carrier {
            listener_key: "udp".into(),
            protocol: TransportProtocol::NativeUdp,
            ..carrier.clone()
        };
        assert!(admit_carrier(cfg, demand.id, &path, peer, &udp).is_err());
    }
}
