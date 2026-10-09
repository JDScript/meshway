//! The projection is a monotone subset of the full configuration.
use super::*;

fn subsequence<T: PartialEq + std::fmt::Debug>(part: &[T], full: &[T]) {
    let mut rest = full.iter();
    for item in part {
        assert!(
            rest.any(|f| f == item),
            "{item:?} is not in order in the full list"
        );
    }
}

fn subset<K: Ord + std::fmt::Debug, V: PartialEq + std::fmt::Debug>(
    part: &BTreeMap<K, V>,
    full: &BTreeMap<K, V>,
) {
    for (key, value) in part {
        assert_eq!(full.get(key), Some(value), "{key:?}");
    }
}

#[test]
fn projections_are_unchanged_subsets_in_original_order() {
    for seed in 0..64 {
        let s = generated(seed);
        let compiled = netmaps(&s);
        for (id, netmap) in &compiled.nodes {
            let projection = &netmap.netmap.projection;
            let view = &projection.configuration;
            assert!(view.nodes.contains_key(id));
            subsequence(&view.access, &s.cfg.access);
            subsequence(&view.topology, &s.cfg.topology);
            subsequence(&view.routing.mesh, &s.cfg.routing.mesh);
            assert_eq!(view.routing.egress, s.cfg.routing.egress);
            assert_eq!(view.defaults, DefaultsView::from(&s.cfg.defaults));
            assert_eq!(view.dns, s.cfg.dns);
            assert_eq!(view.groups, s.cfg.groups);
            assert_eq!(view.tags_owners, s.cfg.tags_owners);
            assert_eq!(view.address_pools, s.cfg.address_pools);
            assert_eq!(view.rule_sets, s.cfg.rule_sets);
            for (peer, settings) in &view.nodes {
                assert_eq!(settings, &NodeSettingsView::from(&s.cfg.nodes[peer]));
                assert!(s.inventory.contains_key(peer));
            }
            subset(&projection.inventory, &s.inventory);
            subset(&projection.subjects.users, &s.subjects.users);
            subset(&projection.subjects.node_users, &s.subjects.node_users);
            subset(&projection.subjects.members, &s.subjects.members);
            subset(&projection.selection_versions, &s.selection_versions);
        }
    }
}

#[test]
fn islands_see_only_themselves_and_their_access_peers() {
    let mut s = base(5);
    s.cfg.defaults.topology = TopologyDefault::Deny;
    for (a, b) in [(1, 2), (3, 4)] {
        s.cfg.topology.push(TopologyRule {
            remark: String::new(),
            action: TopologyAction::Allow,
            a: sel(a),
            b: sel(b),
            transports: Input::Omitted,
        });
    }
    s.cfg.access = vec![
        access(&[sel(1)], &[sel(2)], &["tcp:443"], false),
        access(&[sel(3)], &[sel(4)], &["tcp:443"], false),
        access(&[sel(5)], &[sel(1)], &["icmp:*"], false),
    ];
    let compiled = netmaps(&s);
    let visible = |i: u8| -> Vec<NodeId> {
        compiled.nodes[&node(i)]
            .netmap
            .projection
            .configuration
            .nodes
            .keys()
            .copied()
            .collect()
    };
    assert_eq!(visible(1), [node(1), node(2), node(5)]);
    assert_eq!(visible(3), [node(3), node(4)]);
    assert_eq!(visible(5), [node(1), node(5)]);
    let island = &compiled.nodes[&node(3)].netmap.projection.configuration;
    assert_eq!(island.access, [s.cfg.access[1].clone()]);
    assert_eq!(island.topology, [s.cfg.topology[1].clone()]);
    // Dial material only toward topology-adjacent visible Nodes.
    let dialers = &compiled.nodes[&node(1)].netmap.dialers;
    assert!(dialers.values().all(|d| d.peer == node(2)) && !dialers.is_empty());
}
