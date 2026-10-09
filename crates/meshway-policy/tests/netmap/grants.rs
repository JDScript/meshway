//! Grant shape: node-pair granularity, bidirectional rules, relay copies,
//! adjacency-only edges and observable paths.
use super::*;
use meshway_policy::admission::netmap::{Destination, classify};

fn tcp(
    port_number: u16,
) -> (
    meshway_configuration::execution::BusinessProtocol,
    Option<Port>,
) {
    (BusinessProtocol::Tcp, port(port_number))
}

fn mesh_key(source: u8, target: u8) -> GrantKey {
    GrantKey {
        source: node(source),
        target: GrantTarget::Mesh { node: node(target) },
    }
}

#[test]
fn bidirectional_rule_yields_both_directions_and_one_way_rule_only_one() {
    let mut s = base(4);
    s.cfg.access = vec![
        access(&[sel(1)], &[sel(2)], &["tcp:443"], true),
        access(&[sel(3)], &[sel(4)], &["tcp:443"], false),
    ];
    let compiled = netmaps(&s);
    let at = |i: u8| &compiled.nodes[&node(i)].netmap;
    let address = |i: u8| s.cfg.nodes[&node(i)].addresses.ipv4.unwrap().into();
    let (protocol, p) = tcp(443);
    assert_eq!(
        classify(at(2), address(1), protocol, p),
        Ok(Destination::Mesh(mesh_key(2, 1)))
    );
    assert_eq!(
        classify(at(1), address(2), protocol, p),
        Ok(Destination::Mesh(mesh_key(1, 2)))
    );
    assert!(classify(at(3), address(4), protocol, p).is_ok());
    assert!(classify(at(4), address(3), protocol, p).is_err());
    assert!(!at(4).grants.contains_key(&mesh_key(4, 3)));
}

#[test]
fn access_union_is_rule_order_free_and_pair_scoped() {
    let mut s = base(3);
    s.cfg.access = vec![
        access(&["tag:server".into()], &[sel(1)], &["tcp:8000-8100"], false),
        access(
            &[sel(3)],
            &["group:ops".into()],
            &["udp:53", "tcp:8101"],
            false,
        ),
        access(&[ALICE.into()], &[sel(3)], &["icmp:*"], false),
    ];
    let a = netmaps(&s);
    s.cfg.access.reverse();
    let b = netmaps(&s);
    for id in a.nodes.keys() {
        assert_eq!(a.nodes[id].digest, b.nodes[id].digest);
    }
    let grant = &a.nodes[&node(3)].netmap.grants[&mesh_key(3, 1)];
    assert_eq!(
        grant.services.clone().unwrap(),
        ServiceSet::from_grants([
            (BusinessProtocol::Tcp, Some((8000, 8101))),
            (BusinessProtocol::Udp, Some((53, 53))),
        ])
    );
    let reverse = &a.nodes[&node(1)].netmap.grants[&mesh_key(1, 3)];
    assert_eq!(
        reverse.services.clone().unwrap(),
        ServiceSet::from_grants([(BusinessProtocol::IcmpEcho, None)])
    );
}

#[test]
fn relays_hold_paths_without_services_and_edges_list_only_neighbours() {
    let mut s = base(4);
    s.cfg.access = vec![access(&[sel(1)], &[sel(4)], &["tcp:443"], false)];
    s.cfg.routing.mesh = vec![MeshRoute {
        remark: String::new(),
        src: vec![sel(1)],
        dst: vec![sel(4)],
        via: vec![sel(2), sel(3)],
    }];
    let compiled = netmaps(&s);
    let key = mesh_key(1, 4);
    for i in 1..=4 {
        let netmap = &compiled.nodes[&node(i)].netmap;
        let grant = &netmap.grants[&key];
        assert_eq!(grant.services.is_some(), i == 1 || i == 4);
        assert!(
            grant
                .paths
                .iter()
                .all(|p| p.nodes == [node(1), node(2), node(3), node(4)])
        );
        let neighbours: Vec<NodeId> = match i {
            1 => vec![node(2)],
            2 => vec![node(1), node(3)],
            3 => vec![node(2), node(4)],
            _ => vec![node(3)],
        };
        assert_eq!(netmap.edges.keys().copied().collect::<Vec<_>>(), neighbours);
        let peers: Vec<NodeId> = (1..=4).filter(|j| *j != i).map(node).collect();
        assert_eq!(netmap.peers.keys().copied().collect::<Vec<_>>(), peers);
        for (peer, edges) in &netmap.edges {
            assert!(!edges.dial.is_empty() || !edges.accept.is_empty());
            for key in &edges.dial {
                assert_eq!(netmap.dialers[key].peer, *peer);
            }
            for key in &edges.accept {
                assert!(netmap.self_.listeners.contains_key(key));
            }
        }
        assert_eq!(
            netmap.dialers.keys().collect::<Vec<_>>(),
            netmap
                .edges
                .values()
                .flat_map(|e| &e.dial)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        );
    }
    // Node 3 lacks feature:path-observation in the base inventory.
    assert!(
        compiled.nodes[&node(1)].netmap.grants[&key]
            .paths
            .iter()
            .all(|p| !p.observable)
    );
    s.inventory.insert(node(3), trusted(3, false, true));
    let observable = netmaps(&s);
    assert!(
        observable.nodes[&node(1)].netmap.grants[&key]
            .paths
            .iter()
            .all(|p| p.observable)
    );
}

#[test]
fn unreachable_targets_and_disabled_nodes_get_no_grant() {
    let mut s = base(3);
    s.cfg.access = vec![access(&["*".into()], &["*".into()], &["*"], false)];
    s.cfg.nodes.get_mut(&node(3)).unwrap().enabled = false;
    s.cfg.topology.push(TopologyRule {
        remark: String::new(),
        action: TopologyAction::Block,
        a: sel(1),
        b: sel(2),
        transports: Input::Omitted,
    });
    let compiled = netmaps(&s);
    for netmap in compiled.nodes.values() {
        assert!(
            netmap.netmap.grants.is_empty(),
            "{:?}",
            netmap.netmap.node_id
        );
        assert!(netmap.netmap.edges.is_empty());
    }
    let disabled = &compiled.nodes[&node(3)].netmap;
    assert!(!disabled.self_.enabled);
    assert!(disabled.self_.listeners.is_empty());
    assert!(disabled.egress.is_none());
}
