use super::*;
use meshway_policy::admission::{
    AdmissionError, Carrier, Hop, Traffic, admit_carrier, admit_forward, admit_handshake,
    admit_reply, path_carrier,
};

fn mesh(protocol: BusinessProtocol) -> (NetworkConfiguration, Demand, Traffic) {
    let mut configuration = configuration();
    let token = match protocol {
        BusinessProtocol::Tcp => "tcp:443",
        BusinessProtocol::Udp => "udp:443",
        BusinessProtocol::IcmpEcho => "icmp:*",
    };
    configuration.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec![format!("node:{}", node(2))],
        ip: vec![token.into()],
        bidirectional: false,
    }];
    configuration.routing.mesh = vec![MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec![format!("node:{}", node(2))],
        via: vec![format!("node:{}", node(3))],
    }];
    let mut demand = demand(1, 1, 2);
    demand.protocol = protocol;
    if protocol == BusinessProtocol::IcmpEcho {
        demand.port = None;
    }
    let traffic = Traffic {
        source: "100.64.0.1".parse().unwrap(),
        destination: "100.64.0.2".parse().unwrap(),
        protocol,
        source_port: (protocol != BusinessProtocol::IcmpEcho).then(|| Port::new(49152).unwrap()),
        destination_port: demand.port,
    };
    (configuration, demand, traffic)
}

// Uses the same full semantic normalization and compiler entry points as
// Server, then the public Node admission API. No transport/auth stand-ins.
fn publication(
    configuration: &NetworkConfiguration,
    demand: &Demand,
    dns: &[TrustedDnsAssociation],
) -> CompiledPublication {
    let context = ValidationContext {
        nodes: configuration.nodes.keys().copied().collect(),
        approved_identities: configuration
            .nodes
            .iter()
            .map(|(id, node)| (*id, node.identity.clone()))
            .collect(),
        ..Default::default()
    };
    let (configuration, _) = meshway_policy::normalize(configuration.into(), &context).unwrap();
    run(
        &configuration,
        &inventory(),
        &BTreeMap::from([(demand.id, demand.clone())]),
        dns,
    )
}

fn reverse(forward: Traffic) -> Traffic {
    Traffic {
        source: forward.destination,
        destination: forward.source,
        source_port: forward.destination_port,
        destination_port: forward.source_port,
        ..forward
    }
}

fn assign_exit_profile(
    configuration: &mut NetworkConfiguration,
    demand: &mut Demand,
    matcher: RouteMatch,
) {
    let profile = "source-exit";
    configuration.routing.egress.profiles.insert(
        profile.into(),
        RoutingProfile {
            rules: vec![EgressRule {
                remark: String::new(),
                matcher,
                action: RuleAction::Exit {
                    nodes: vec![format!("node:{}", node(4))],
                    via: vec![format!("node:{}", node(3))],
                },
            }],
            fallback: RouteAction::Unmanaged,
        },
    );
    configuration.routing.egress.assignments = vec![ProfileAssignment {
        target: vec![format!("node:{}", demand.source)],
        allowed: vec![profile.into()],
        default: profile.into(),
        locked: true,
    }];
    demand.profile = profile.into();
}

#[test]
fn compiler_grants_admit_tcp_udp_echo_and_only_exact_reverse_replies() {
    for protocol in [
        BusinessProtocol::Tcp,
        BusinessProtocol::Udp,
        BusinessProtocol::IcmpEcho,
    ] {
        let (configuration, demand, traffic) = mesh(protocol);
        let publication = publication(&configuration, &demand, &[]);
        let path = [node(1), node(3), node(2)];
        let reply = reverse(traffic);
        for (position, local) in path.iter().enumerate() {
            let configuration = &publication.nodes[local].configuration;
            let previous = position.checked_sub(1).map(|index| path[index]);
            assert_eq!(
                admit_forward(
                    configuration,
                    demand.id,
                    &path,
                    node(1),
                    previous,
                    &traffic,
                    1000
                ),
                Ok(path
                    .get(position + 1)
                    .map_or(Hop::Deliver, |next| Hop::Forward(*next)))
            );
            assert_eq!(
                admit_reply(
                    configuration,
                    demand.id,
                    &path,
                    node(1),
                    path.get(position + 1).copied(),
                    &traffic,
                    &reply,
                    1000
                ),
                Ok(previous.map_or(Hop::Deliver, Hop::Forward))
            );
            // Reversed traffic is not a new forward grant, regardless of ID reuse.
            assert!(
                admit_forward(
                    configuration,
                    demand.id,
                    &path,
                    node(1),
                    previous,
                    &reply,
                    1000
                )
                .is_err()
            );
            let mut forged = reply;
            forged.destination = "100.64.0.4".parse().unwrap();
            assert!(
                admit_reply(
                    configuration,
                    demand.id,
                    &path,
                    node(1),
                    path.get(position + 1).copied(),
                    &traffic,
                    &forged,
                    1000
                )
                .is_err()
            );
        }
        let receiver = &publication.nodes[&node(2)].configuration;
        assert!(
            admit_reply(
                receiver,
                demand.id,
                &path,
                node(1),
                Some(node(3)),
                &traffic,
                &reply,
                1000
            )
            .is_err()
        );
        assert!(
            admit_reply(
                receiver,
                demand.id,
                &[node(2), node(3), node(1)],
                node(1),
                None,
                &traffic,
                &reply,
                1000
            )
            .is_err()
        );
    }
}

#[test]
fn authenticated_upstream_and_original_source_are_both_required() {
    let (configuration, demand, traffic) = mesh(BusinessProtocol::Tcp);
    let publication = publication(&configuration, &demand, &[]);
    let path = [node(1), node(3), node(2)];
    for (local, valid) in [
        (node(1), None),
        (node(3), Some(node(1))),
        (node(2), Some(node(3))),
    ] {
        let configuration = &publication.nodes[&local].configuration;
        for previous in [
            None,
            Some(node(1)),
            Some(node(2)),
            Some(node(3)),
            Some(node(4)),
        ] {
            assert_eq!(
                admit_forward(
                    configuration,
                    demand.id,
                    &path,
                    node(1),
                    previous,
                    &traffic,
                    1000
                )
                .is_ok(),
                previous == valid
            );
        }
        assert!(
            admit_forward(
                configuration,
                demand.id,
                &path,
                node(3),
                valid,
                &traffic,
                1000
            )
            .is_err()
        );
    }
}

#[test]
fn spoofed_tuples_and_malformed_ports_fail_closed() {
    for protocol in [
        BusinessProtocol::Tcp,
        BusinessProtocol::Udp,
        BusinessProtocol::IcmpEcho,
    ] {
        let (configuration, demand, traffic) = mesh(protocol);
        let publication = publication(&configuration, &demand, &[]);
        let configuration = &publication.nodes[&node(3)].configuration;
        let path = [node(1), node(3), node(2)];
        let mut bad_source = traffic;
        bad_source.source = "100.64.0.3".parse().unwrap();
        let mut bad_destination = traffic;
        bad_destination.destination = "100.64.0.4".parse().unwrap();
        let mut bad_source_port = traffic;
        bad_source_port.source_port = if protocol == BusinessProtocol::IcmpEcho {
            Some(Port::new(1).unwrap())
        } else {
            None
        };
        let mut bad_destination_port = traffic;
        bad_destination_port.destination_port = Some(Port::new(80).unwrap());
        let mut bad_protocol = traffic;
        bad_protocol.protocol = if protocol == BusinessProtocol::Tcp {
            BusinessProtocol::Udp
        } else {
            BusinessProtocol::Tcp
        };
        for forged in [
            bad_source,
            bad_destination,
            bad_source_port,
            bad_destination_port,
            bad_protocol,
        ] {
            assert!(
                admit_forward(
                    configuration,
                    demand.id,
                    &path,
                    node(1),
                    Some(node(1)),
                    &forged,
                    1000
                )
                .is_err()
            );
        }
        let mut reply = reverse(traffic);
        reply.destination_port = Some(Port::new(49153).unwrap());
        assert!(
            admit_reply(
                configuration,
                demand.id,
                &path,
                node(1),
                Some(node(2)),
                &traffic,
                &reply,
                1000
            )
            .is_err()
        );
    }
}

#[test]
fn exact_complete_path_current_peers_and_permit_are_mandatory() {
    let (configuration, demand, traffic) = mesh(BusinessProtocol::Tcp);
    let publication = publication(&configuration, &demand, &[]);
    let original = &publication.nodes[&node(3)].configuration;
    let path = [node(1), node(3), node(2)];
    for wrong in [
        vec![],
        vec![node(1), node(2)],
        vec![node(3), node(1), node(2)],
        vec![node(1), node(3), node(3), node(2)],
        vec![node(1), node(3)],
    ] {
        assert!(
            admit_forward(
                original,
                demand.id,
                &wrong,
                node(1),
                Some(node(1)),
                &traffic,
                1000
            )
            .is_err()
        );
    }
    for change in 0..9 {
        let mut configuration = original.clone();
        match change {
            0 => {
                configuration.permits.clear();
            }
            1 => {
                configuration.enabled = false;
            }
            2 => {
                configuration.peers.remove(&node(1));
            }
            3 => {
                configuration
                    .peers
                    .get_mut(&node(1))
                    .unwrap()
                    .addresses
                    .ipv4 = Some("100.64.0.9".parse().unwrap());
            }
            4 => {
                configuration.permits.get_mut(&demand.id).unwrap().demand_id = request(2);
            }
            5 => {
                configuration.permits.get_mut(&demand.id).unwrap().source = node(3);
            }
            6 => {
                configuration.node_id = node(4);
            }
            7 => {
                configuration.permits.get_mut(&demand.id).unwrap().target =
                    BusinessTarget::Mesh { node: node(3) };
            }
            8 => {
                configuration.self_keys.control_key.public_key.algorithm = KeyAlgorithm::X25519;
            }
            _ => unreachable!(),
        }
        assert!(
            admit_forward(
                &configuration,
                demand.id,
                &path,
                node(1),
                Some(node(1)),
                &traffic,
                1000
            )
            .is_err(),
            "mutation {change}"
        );
        assert!(
            admit_reply(
                &configuration,
                demand.id,
                &path,
                node(1),
                Some(node(2)),
                &traffic,
                &reverse(traffic),
                1000
            )
            .is_err(),
            "reply mutation {change}"
        );
    }
    assert_eq!(
        admit_forward(
            original,
            demand.id,
            &path,
            node(1),
            Some(node(1)),
            &traffic,
            demand.expires_at_ms
        ),
        Err(AdmissionError::Expired)
    );
    assert_eq!(
        admit_reply(
            original,
            demand.id,
            &path,
            node(1),
            Some(node(2)),
            &traffic,
            &reverse(traffic),
            demand.expires_at_ms
        ),
        Err(AdmissionError::Expired)
    );
    assert!(
        admit_forward(
            original,
            request(2),
            &path,
            node(1),
            Some(node(1)),
            &traffic,
            1000
        )
        .is_err()
    );
}

#[test]
fn external_exit_requires_access_routing_and_exit_while_service_is_mesh_delivery() {
    let (mut configuration, mut demand, mut traffic) = mesh(BusinessProtocol::Tcp);
    configuration.access[0].dst = vec!["8.8.8.8/32".into()];
    assign_exit_profile(
        &mut configuration,
        &mut demand,
        RouteMatch {
            ip_cidr: vec!["8.8.8.8/32".into()],
            ..Default::default()
        },
    );
    traffic.destination = "8.8.8.8".parse().unwrap();
    demand.target = BusinessTarget::Ip {
        address: ExecutionAddress(traffic.destination),
    };
    let compiled = publication(&configuration, &demand, &[]);
    let path = [node(1), node(3), node(4)];
    let exit = &compiled.nodes[&node(4)].configuration;
    assert_eq!(
        admit_forward(
            exit,
            demand.id,
            &path,
            node(1),
            Some(node(3)),
            &traffic,
            1000
        ),
        Ok(Hop::Exit)
    );
    assert_eq!(
        admit_reply(
            exit,
            demand.id,
            &path,
            node(1),
            None,
            &traffic,
            &reverse(traffic),
            1000
        ),
        Ok(Hop::Forward(node(3)))
    );
    let mut disabled_exit = exit.clone();
    disabled_exit.exit_enabled = false;
    assert_eq!(
        admit_handshake(
            &disabled_exit,
            demand.id,
            &path,
            node(1),
            Some(node(3)),
            false,
            1000
        ),
        Err(AdmissionError::ExitDisabled)
    );
    assert_eq!(
        admit_forward(
            &disabled_exit,
            demand.id,
            &path,
            node(1),
            Some(node(3)),
            &traffic,
            1000
        ),
        Err(AdmissionError::ExitDisabled)
    );
    let mut other_destination = traffic;
    other_destination.destination = "8.8.4.4".parse().unwrap();
    assert!(
        admit_forward(
            exit,
            demand.id,
            &path,
            node(1),
            Some(node(3)),
            &other_destination,
            1000
        )
        .is_err()
    );
    // Revoked Access with the old routing retained compiles to no grant.
    configuration.access.clear();
    let revoked = run(
        &configuration,
        &inventory(),
        &BTreeMap::from([(demand.id, demand.clone())]),
        &[],
    );
    assert!(
        admit_forward(
            &revoked.nodes[&node(4)].configuration,
            demand.id,
            &path,
            node(1),
            Some(node(3)),
            &traffic,
            1000
        )
        .is_err()
    );
    // A Mesh service on the exit is not external egress, even with exit disabled.
    configuration.routing.egress = EgressRouting::default();
    demand.profile = "mesh".into();
    configuration.routing.mesh[0].dst = vec![format!("node:{}", node(4))];
    configuration.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec![format!("node:{}", node(4))],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    configuration.nodes.get_mut(&node(4)).unwrap().exit_enabled = false;
    demand.target = BusinessTarget::Mesh { node: node(4) };
    traffic.destination = "100.64.0.4".parse().unwrap();
    let compiled = publication(&configuration, &demand, &[]);
    assert_eq!(
        admit_forward(
            &compiled.nodes[&node(4)].configuration,
            demand.id,
            &path,
            node(1),
            Some(node(3)),
            &traffic,
            1000
        ),
        Ok(Hop::Deliver)
    );
}

#[test]
fn domain_address_and_dns_expiry_survive_compilation_to_admission() {
    let (mut configuration, mut demand, mut traffic) = mesh(BusinessProtocol::Udp);
    configuration.access[0].dst = vec!["domain:example.test".into()];
    assign_exit_profile(
        &mut configuration,
        &mut demand,
        RouteMatch {
            domain_suffix: vec!["example.test".into()],
            ..Default::default()
        },
    );
    traffic.destination = "8.8.8.8".parse().unwrap();
    demand.target = BusinessTarget::Domain {
        name: "api.example.test".into(),
        address: ExecutionAddress(traffic.destination),
    };
    let dns = [TrustedDnsAssociation {
        network_id: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
        configuration_revision: Revision::MIN,
        source: node(1),
        name: "api.example.test".into(),
        cname_chain: vec![],
        addresses: vec![ExecutionAddress(traffic.destination)],
        expires_at_ms: 2000,
    }];
    let compiled = publication(&configuration, &demand, &dns);
    let configuration = &compiled.nodes[&node(4)].configuration;
    let path = [node(1), node(3), node(4)];
    assert_eq!(
        admit_forward(
            configuration,
            demand.id,
            &path,
            node(1),
            Some(node(3)),
            &traffic,
            1000
        ),
        Ok(Hop::Exit)
    );
    assert_eq!(
        admit_forward(
            configuration,
            demand.id,
            &path,
            node(1),
            Some(node(3)),
            &traffic,
            2000
        ),
        Err(AdmissionError::Expired)
    );
    traffic.destination = "8.8.4.4".parse().unwrap();
    assert!(
        admit_forward(
            configuration,
            demand.id,
            &path,
            node(1),
            Some(node(3)),
            &traffic,
            1000
        )
        .is_err()
    );
}

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
