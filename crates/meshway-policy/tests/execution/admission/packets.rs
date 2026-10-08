use super::*;

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
