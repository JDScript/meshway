use super::*;

#[test]
fn capture_pools_are_complete_without_peers_or_demands_and_on_disabled_nodes() {
    let mut c = configuration();
    c.address_pools.ipv4.push("100.65.0.0/16".into());
    c.address_pools.ipv6 = vec!["fd00:1234::/48".into()];
    c.nodes.get_mut(&node(2)).unwrap().enabled = false;
    let result = run(&c, &inventory(), &BTreeMap::new(), &[]);
    assert_eq!(result.nodes.len(), 4);
    for slice in result.nodes.values() {
        assert_eq!(slice.configuration.mesh_address_pools, c.address_pools);
        assert!(slice.configuration.peers.is_empty());
        assert!(slice.configuration.permits.is_empty());
        assert!(slice.configuration.local_egress.is_empty());
        assert!(slice.configuration.dialers.is_empty());
        assert!(slice.configuration.decisions.is_empty());
    }
    assert!(!result.nodes[&node(2)].configuration.enabled);
    c.nodes.clear();
    assert!(
        run(&c, &BTreeMap::new(), &BTreeMap::new(), &[])
            .nodes
            .is_empty()
    );
}

#[test]
fn captured_unassigned_and_unauthorized_addresses_never_gain_permits() {
    let mut c = configuration();
    c.access.clear();
    for address in ["100.64.0.2", "100.64.0.200"] {
        let mut demand = demand(1, 1, 2);
        demand.target = BusinessTarget::Ip {
            address: ExecutionAddress(address.parse().unwrap()),
        };
        let result = run(
            &c,
            &inventory(),
            &BTreeMap::from([(demand.id, demand)]),
            &[],
        );
        let slice = &result.nodes[&node(1)].configuration;
        assert_eq!(slice.mesh_address_pools, c.address_pools);
        assert!(matches!(
            slice.decisions[&request(1)],
            DemandDecision::Unavailable { .. }
        ));
        assert!(slice.permits.is_empty());
        assert!(slice.peers.is_empty());
        assert!(slice.dialers.is_empty());
    }
}
