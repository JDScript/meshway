use super::*;

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
