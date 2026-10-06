use super::*;
use meshway_policy::admission::*;

fn compiled() -> (CompiledPublication, ObservationId) {
    let mut configuration = configuration();
    configuration.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec![format!("node:{}", node(2))],
        ip: vec!["icmp:*".into()],
        bidirectional: false,
    }];
    configuration.routing.mesh = vec![MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec![format!("node:{}", node(2))],
        via: vec![format!("node:{}", node(3))],
    }];
    let id = ObservationId::new(request(1).into_uuid()).unwrap();
    let diagnostic = DiagnosticDemand {
        id,
        source: node(1),
        instance_id: InstanceId::new(request(2).into_uuid()).unwrap(),
        target: node(2),
        service: ObservationServiceScope::IcmpEcho,
        path: None,
        expires_at_ms: 10_000,
    };
    let publication = compile_with_diagnostics(
        CompilationInput {
            network: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
            revision: Revision::MIN,
            publication: "00000000-0000-4004-8000-000000000001".parse().unwrap(),
            configuration: &configuration,
            subjects: &SubjectContext::default(),
            inventory: &inventory(),
            demands: &BTreeMap::new(),
            dns: &[],
            rule_sets: &BTreeMap::new(),
            selected_profiles: &BTreeMap::new(),
            now_ms: 1000,
        },
        &BTreeMap::from([(id, diagnostic)]),
    )
    .unwrap();
    (publication, id)
}

#[test]
fn independent_diagnostic_full_path_both_carrier_directions_and_no_ip_grant() {
    let (publication, id) = compiled();
    let auth = CircuitAuthorization::Diagnostic(id);
    let path = [node(1), node(3), node(2)];
    for (position, local) in path.iter().enumerate() {
        let cfg = &publication.nodes[local].configuration;
        assert!(cfg.permits.is_empty());
        assert!(cfg.decisions.is_empty());
        assert_eq!(cfg.diagnostics[&id].paths[0].nodes, path);
        let previous = position.checked_sub(1).map(|index| path[index]);
        let next = path.get(position + 1).copied();
        assert_eq!(
            admit_circuit_handshake(cfg, auth, &path, node(1), previous, false, 1000),
            Ok(next.map_or(Hop::Deliver, Hop::Forward))
        );
        assert_eq!(
            admit_circuit_handshake(cfg, auth, &path, node(1), next, true, 1000),
            Ok(previous.map_or(Hop::Deliver, Hop::Forward))
        );
        assert!(admit_handshake(cfg, request(1), &path, node(1), previous, false, 1000).is_err());
        assert!(
            admit_forward(
                cfg,
                request(1),
                &path,
                node(1),
                previous,
                &Traffic {
                    source: "100.64.0.1".parse().unwrap(),
                    destination: "100.64.0.2".parse().unwrap(),
                    protocol: BusinessProtocol::IcmpEcho,
                    source_port: None,
                    destination_port: None
                },
                1000
            )
            .is_err()
        );
        assert_eq!(
            admit_circuit_handshake(cfg, auth, &path, node(1), previous, false, 10_000),
            Err(AdmissionError::Expired)
        );
    }
    for edge in path.windows(2) {
        for (dialer, listener) in [(edge[0], edge[1]), (edge[1], edge[0])] {
            let carrier = Carrier {
                dialer,
                listener,
                listener_key: "udp".into(),
                protocol: TransportProtocol::NativeUdp,
            };
            for (local, peer) in [(dialer, listener), (listener, dialer)] {
                let cfg = &publication.nodes[&local].configuration;
                assert!(admit_circuit_carrier(cfg, auth, &path, peer, &carrier).is_ok());
                assert_eq!(admit_adjacency(cfg, peer, &carrier, 1000), Ok(10_000));
                let mut malformed = cfg.clone();
                malformed.diagnostics.get_mut(&id).unwrap().observation_id =
                    ObservationId::new(request(3).into_uuid()).unwrap();
                assert!(admit_adjacency(&malformed, peer, &carrier, 1000).is_err());
                assert!(admit_adjacency(cfg, peer, &carrier, 10_000).is_err());
            }
        }
    }
    assert!(
        admit_circuit_handshake(
            &publication.nodes[&node(1)].configuration,
            auth,
            &[node(1), node(2)],
            node(1),
            None,
            false,
            1000
        )
        .is_err()
    );
}

#[test]
fn diagnostic_digest_binds_instance_service_target_and_path_but_excludes_expiry() {
    let (publication, id) = compiled();
    let permit = &publication.nodes[&node(1)].configuration.diagnostics[&id];
    let digest = meshway_policy::diagnostic_authorization_digest(permit).unwrap();
    let mut renewed = permit.clone();
    renewed.expires_at_ms += 1000;
    assert_eq!(
        digest,
        meshway_policy::diagnostic_authorization_digest(&renewed).unwrap()
    );
    for field in 0..6 {
        let mut changed = permit.clone();
        match field {
            0 => changed.instance_id = InstanceId::new(request(3).into_uuid()).unwrap(),
            1 => changed.source = node(3),
            2 => changed.target = node(3),
            3 => {
                changed.service = ObservationServiceScope::Udp {
                    port: Port::new(443).unwrap(),
                }
            }
            4 => {
                changed.paths[0].nodes.remove(1);
            }
            _ => changed.observation_id = ObservationId::new(request(3).into_uuid()).unwrap(),
        };
        assert_ne!(
            digest,
            meshway_policy::diagnostic_authorization_digest(&changed).unwrap()
        );
    }
}
