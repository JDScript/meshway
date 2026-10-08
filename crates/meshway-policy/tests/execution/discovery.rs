use super::*;

#[test]
fn published_dns_filters_path_peers_by_business_access_for_each_source() {
    let mut c = configuration();
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec![format!("node:{}", node(3))],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    c.routing.mesh = vec![MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{}", node(1))],
        dst: vec![format!("node:{}", node(3))],
        via: vec![format!("node:{}", node(2))],
    }];
    let d = demand(1, 1, 3);
    let result = run(&c, &inventory(), &BTreeMap::from([(d.id, d)]), &[]);
    let a = &result.nodes[&node(1)].configuration;
    assert!(a.peers.contains_key(&node(2)));
    assert!(a.dns.mesh_hosts.contains_key("n3.test.internal"));
    assert!(!a.dns.mesh_hosts.contains_key("n2.test.internal"));
    for id in [node(2), node(3)] {
        let configuration = &result.nodes[&id].configuration;
        assert!(!configuration.peers.is_empty());
        assert!(
            configuration.dns.mesh_hosts.is_empty(),
            "relay/reply qualification does not grant name visibility"
        );
    }
}

#[test]
fn discovery_access_and_concrete_demands_share_traffic_and_direction() {
    let mut c = configuration();
    let subjects = SubjectContext::default();
    for (token, protocol, allowed, denied) in [
        ("tcp:65432", BusinessProtocol::Tcp, Some(65432), Some(443)),
        (
            "udp:41000-41007",
            BusinessProtocol::Udp,
            Some(41007),
            Some(41008),
        ),
        ("tcp:*", BusinessProtocol::Tcp, Some(65535), None),
        ("udp:*", BusinessProtocol::Udp, Some(1), None),
        ("icmp:*", BusinessProtocol::IcmpEcho, None, None),
        ("*", BusinessProtocol::Tcp, Some(65535), None),
    ] {
        c.access = vec![AccessRule {
            remark: String::new(),
            src: vec![format!("node:{}", node(1))],
            dst: vec![format!("node:{}", node(2))],
            ip: vec![token.into()],
            bidirectional: false,
        }];
        assert!(
            mesh_access_allows_any(&c, &subjects, node(1), node(2)),
            "{token}"
        );
        assert!(!mesh_access_allows_any(&c, &subjects, node(2), node(1)));
        let mut forward = demand(1, 1, 2);
        forward.protocol = protocol;
        forward.port = allowed.map(|p| Port::new(p).unwrap());
        let mut reverse = forward.clone();
        reverse.id = request(2);
        reverse.source = node(2);
        reverse.target = BusinessTarget::Mesh { node: node(1) };
        let mut demands = BTreeMap::from([(forward.id, forward.clone()), (reverse.id, reverse)]);
        let result = run(&c, &inventory(), &demands, &[]);
        assert!(
            result.nodes[&node(1)]
                .configuration
                .permits
                .contains_key(&request(1))
        );
        assert!(
            !result.nodes[&node(1)]
                .configuration
                .permits
                .contains_key(&request(2))
        );
        c.access[0].bidirectional = true;
        assert!(mesh_access_allows_any(&c, &subjects, node(2), node(1)));
        let result = run(&c, &inventory(), &demands, &[]);
        assert!(
            result.nodes[&node(1)]
                .configuration
                .permits
                .contains_key(&request(2))
        );
        if let Some(port) = denied {
            forward.port = Some(Port::new(port).unwrap());
            demands.insert(forward.id, forward);
            assert!(
                !run(&c, &inventory(), &demands, &[]).nodes[&node(1)]
                    .configuration
                    .permits
                    .contains_key(&request(1))
            );
        }
    }
    c.nodes.get_mut(&node(1)).unwrap().enabled = false;
    c.nodes.get_mut(&node(2)).unwrap().enabled = false;
    assert!(mesh_access_allows_any(&c, &subjects, node(1), node(2)));
    for destination in [
        "domain:test.internal",
        "100.64.0.0/24",
        "autogroup:internet",
    ] {
        c.access[0].dst = vec![destination.into()];
        assert!(!mesh_access_allows_any(&c, &subjects, node(1), node(2)));
    }
    c.access.clear();
    assert!(!mesh_access_allows_any(&c, &subjects, node(1), node(1)));
    assert!(!mesh_access_allows_any(&c, &subjects, node(1), node(2)));
}

#[test]
fn discovery_wildcards_and_node_selectors_cannot_bypass_active_identity() {
    let mut c = configuration();
    let binding = UserBinding {
        user_id: "00000000-0000-4006-8000-000000000001".parse().unwrap(),
        membership_id: "00000000-0000-4006-8000-000000000002".parse().unwrap(),
    };
    let email = "member@example.test".to_owned();
    c.nodes.get_mut(&node(2)).unwrap().identity = NodeIdentity::User {
        user: email.clone(),
    };
    let mut subjects = SubjectContext {
        users: BTreeMap::from([(email.clone(), binding.clone())]),
        node_users: BTreeMap::from([(node(2), binding.clone())]),
        members: BTreeMap::from([(email.clone(), binding.clone())]),
    };
    for selector in ["*".to_owned(), format!("node:{}", node(2))] {
        c.access[0].dst = vec![selector];
        assert!(mesh_access_allows_any(&c, &subjects, node(1), node(2)));
        subjects.members.clear();
        assert!(!mesh_access_allows_any(&c, &subjects, node(1), node(2)));
        let mut replacement = binding.clone();
        replacement.membership_id = "00000000-0000-4006-8000-000000000003".parse().unwrap();
        subjects.members.insert(email.clone(), replacement);
        assert!(!mesh_access_allows_any(&c, &subjects, node(1), node(2)));
        subjects.members.insert(email.clone(), binding.clone());
    }
    c.tags_owners.clear();
    assert!(!mesh_access_allows_any(&c, &subjects, node(1), node(2)));
}

#[test]
fn compiler_emits_bounded_normalized_node_dns_names() {
    let mut c = configuration();
    c.dns.domain = "BÜCHER.Example.".into();
    let demands = BTreeMap::from([(request(1), demand(1, 1, 2))]);
    let publication = run(&c, &inventory(), &demands, &[]);
    let hosts = &publication.nodes[&node(1)].configuration.dns.mesh_hosts;
    assert_eq!(
        hosts["n2.xn--bcher-kva.example"],
        c.nodes[&node(2)].addresses
    );
    assert!(
        hosts
            .keys()
            .all(|name| name.ends_with(".xn--bcher-kva.example"))
    );
}
