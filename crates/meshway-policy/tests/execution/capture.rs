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
fn capture_pools_are_required_digest_fields_and_delta_replacements() {
    let mut c = configuration();
    let inventory = inventory();
    let old = run(&c, &inventory, &BTreeMap::new(), &[])
        .nodes
        .remove(&node(1))
        .unwrap();
    c.address_pools.ipv4 = vec!["100.64.0.0/16".into(), "100.65.0.0/16".into()];
    c.address_pools.ipv6 = vec!["fd00:1234::/48".into()];
    let new = run(&c, &inventory, &BTreeMap::new(), &[])
        .nodes
        .remove(&node(1))
        .unwrap();
    let delta = configuration_delta(&old, &new).unwrap();
    assert_eq!(delta.mesh_address_pools, c.address_pools);
    assert_eq!(apply_configuration_delta(&old, delta.clone()).unwrap(), new);
    let reverse = configuration_delta(&new, &old).unwrap();
    assert!(reverse.mesh_address_pools.ipv6.is_empty());
    assert_eq!(apply_configuration_delta(&new, reverse).unwrap(), old);
    let mut forged = delta;
    forged.mesh_address_pools = old.configuration.mesh_address_pools.clone();
    assert!(apply_configuration_delta(&old, forged).is_err());
    let mut changed_only_pools = old.configuration.clone();
    changed_only_pools.mesh_address_pools = c.address_pools;
    assert_ne!(digest(&changed_only_pools).unwrap(), old.digest);
    let mut wire = serde_json::to_value(&old.configuration).unwrap();
    assert!(
        wire.as_object_mut()
            .unwrap()
            .remove("meshAddressPools")
            .is_some()
    );
    assert!(serde_json::from_value::<NodeConfiguration>(wire).is_err());
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
