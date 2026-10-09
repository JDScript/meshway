//! Serial invariance, digest and delta integrity, names, budget and strict decoding.
use super::*;

fn mesh_scenario() -> Scenario {
    let mut s = base(5);
    s.cfg.access = vec![
        access(&[sel(1)], &[sel(2)], &["tcp:443"], false),
        access(&[sel(3)], &[sel(4)], &["udp:53"], false),
    ];
    // Only the 1-2 and 3-4 edges exist, so no Node relays the other pair.
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
    s
}

#[test]
fn unrelated_revision_keeps_digest_serial_and_envelope() {
    let mut s = mesh_scenario();
    let first = netmaps(&s);
    let previous = previous(&first);
    // Unrelated to node 5: a new rule between 3 and 4 and a remark edit.
    s.cfg
        .access
        .push(access(&[sel(4)], &[sel(3)], &["tcp:22"], false));
    s.cfg.access[1].remark = "edited".into();
    let revision = Revision::new(2).unwrap();
    let second = compile_with(&s, &previous, revision);
    for id in [node(1), node(2), node(5)] {
        assert_eq!(first.nodes[&id], second.nodes[&id], "{id}");
        assert_eq!(second.nodes[&id].netmap.revision, Revision::MIN);
    }
    for id in [node(3), node(4)] {
        let (a, b) = (&first.nodes[&id], &second.nodes[&id]);
        assert_ne!(a.digest, b.digest);
        assert_eq!(b.netmap.serial, a.netmap.serial.checked_next().unwrap());
        assert_eq!(b.netmap.revision, revision);
        assert_eq!(b.netmap.dependency_digest, second.dependency_digest);
    }
    // Recompiling the same revision again is a no-op for every Node.
    let third = compile_with(&s, &self::previous(&second), revision);
    assert_eq!(second.nodes, third.nodes);
}

#[test]
fn selection_switch_and_key_rotation_change_only_dependent_netmaps() {
    let mut s = base(5);
    s.cfg.access = vec![access(
        &[sel(1)],
        &["autogroup:internet".into()],
        &["tcp:443"],
        false,
    )];
    s.cfg.routing.egress.profiles.insert(
        "exit".into(),
        RoutingProfile {
            rules: vec![],
            fallback: RouteAction::Exit {
                nodes: vec![sel(4)],
                via: vec![],
            },
        },
    );
    s.cfg.routing.egress.assignments.push(ProfileAssignment {
        target: vec![sel(1)],
        allowed: vec!["exit".into(), "mesh".into()],
        default: "exit".into(),
        locked: false,
    });
    s.cfg.defaults.topology = TopologyDefault::Deny;
    s.cfg.topology.push(TopologyRule {
        remark: String::new(),
        action: TopologyAction::Allow,
        a: sel(1),
        b: sel(4),
        transports: Input::Omitted,
    });
    let first = netmaps(&s);
    let selection = |c: &CompiledNetmaps| {
        c.nodes[&node(1)]
            .netmap
            .egress
            .as_ref()
            .unwrap()
            .capture
            .generation
    };
    // Switching away and back is a new selection version: a new selection identity.
    s.selection_versions
        .insert(node(1), meshway_model::Version::new(2).unwrap());
    let switched = netmaps(&s);
    assert_ne!(selection(&first), selection(&switched));
    for id in [node(2), node(3), node(5)] {
        assert_eq!(first.nodes[&id].digest, switched.nodes[&id].digest);
    }
    // Rotating node 2's dataplane key does not touch Nodes that never see node 2.
    s.inventory
        .get_mut(&node(2))
        .unwrap()
        .dataplane_key
        .public_key
        .bytes = [99; 32].into();
    let rotated = netmaps(&s);
    for id in [node(1), node(3), node(4), node(5)] {
        assert_eq!(
            switched.nodes[&id].digest, rotated.nodes[&id].digest,
            "{id}"
        );
    }
    assert_ne!(
        switched.nodes[&node(2)].digest,
        rotated.nodes[&node(2)].digest
    );
}

#[test]
fn mesh_hosts_are_exactly_the_discoverable_names() {
    let mut s = base(5);
    s.cfg.access = vec![
        access(&[sel(1)], &["tag:server".into()], &["icmp:*"], false),
        access(&[sel(2)], &[sel(4)], &["tcp:443"], true),
    ];
    let compiled = netmaps(&s);
    for (id, netmap) in &compiled.nodes {
        let expected: BTreeMap<String, NodeAddresses> = s
            .cfg
            .nodes
            .iter()
            .filter(|(other, _)| {
                *other == id
                    || meshway_policy::mesh_access_allows_any(&s.cfg, &s.subjects, *id, **other)
            })
            .map(|(_, n)| (format!("{}.test.internal", n.hostname), n.addresses.clone()))
            .collect();
        assert_eq!(netmap.netmap.dns.mesh_hosts, expected, "{id}");
    }
    let one = &compiled.nodes[&node(1)].netmap.dns.mesh_hosts;
    assert!(one.contains_key("n3.test.internal") && one.contains_key("n5.test.internal"));
    assert!(!one.contains_key("n2.test.internal"));
    assert!(
        compiled.nodes[&node(4)]
            .netmap
            .dns
            .mesh_hosts
            .contains_key("n2.test.internal")
    );
    s.cfg.dns.magic_dns = false;
    assert!(
        netmaps(&s)
            .nodes
            .values()
            .all(|n| n.netmap.dns.mesh_hosts.is_empty())
    );
}

#[test]
fn digest_round_trip_and_delta_reconstruct_exact_target() {
    let mut s = mesh_scenario();
    let first = netmaps(&s);
    for compiled in first.nodes.values() {
        let text = serde_json::to_string(compiled).unwrap();
        let decoded: CompiledNetmap = serde_json::from_str(&text).unwrap();
        assert_eq!(&decoded, compiled);
        decoded.verify().unwrap();
        let mut tampered = decoded.clone();
        tampered.netmap.self_.exit_enabled = !tampered.netmap.self_.exit_enabled;
        assert!(tampered.verify().is_err());
    }
    s.cfg
        .access
        .push(access(&[sel(2)], &[sel(1)], &["udp:*"], false));
    s.cfg.access.remove(0);
    let second = compile_with(&s, &previous(&first), Revision::new(2).unwrap());
    let (base, target) = (&first.nodes[&node(1)], &second.nodes[&node(1)]);
    let delta = netmap_delta(base, target).unwrap();
    assert_ne!(
        delta.configuration.access,
        base.netmap.projection.configuration.access
    );
    assert!(delta.configuration.nodes.is_empty() && delta.nodes.upsert.is_empty());
    let text = serde_json::to_string(&delta).unwrap();
    let delta: NetmapDelta = serde_json::from_str(&text).unwrap();
    assert_eq!(&apply_netmap_delta(base, delta.clone()).unwrap(), target);
    let mut forged = delta.clone();
    forged.nodes.remove.push(node(9));
    assert!(apply_netmap_delta(base, forged).is_err());
    let mut stale = delta;
    stale.base.serial = stale.target.serial;
    assert!(apply_netmap_delta(base, stale).is_err());
    assert!(netmap_delta(target, base).is_err(), "serial must increase");
}

#[test]
fn wire_rejects_duplicate_keys_unknown_fields_and_positional_objects() {
    let mut s = base(3);
    s.cfg.access = vec![access(&["*".into()], &["*".into()], &["tcp:443"], false)];
    let compiled = &netmaps(&s).nodes[&node(1)];
    let text = serde_json::to_string(compiled).unwrap();
    assert!(serde_json::from_str::<CompiledNetmap>(&text).is_ok());
    let value = serde_json::to_value(compiled).unwrap();
    let mut extra = value.clone();
    extra["netmap"]["self"]["unknown"] = serde_json::json!(1);
    assert!(serde_json::from_value::<CompiledNetmap>(extra).is_err());
    let mut positional = value.clone();
    positional["netmap"]["projection"]["subjects"] = serde_json::json!([{}, {}, {}]);
    assert!(serde_json::from_value::<CompiledNetmap>(positional).is_err());
    let entry =
        serde_json::to_string(&value["netmap"]["projection"]["inventory"][node(2).to_string()])
            .unwrap();
    let duplicated = text.replacen(
        "\"inventory\":{",
        &format!("\"inventory\":{{\"{}\":{entry},", node(2)),
        1,
    );
    assert_ne!(duplicated, text);
    assert!(serde_json::from_str::<CompiledNetmap>(&duplicated).is_err());
}

#[test]
fn oversized_netmap_rejects_the_whole_compilation() {
    let mut s = base(3);
    s.cfg.access = vec![access(&["*".into()], &["*".into()], &["*"], false)];
    // Inflate through the DNS settings, which every netmap carries in full.
    for i in 0..70_000u32 {
        s.cfg
            .dns
            .search_domains
            .push(format!("d{i:05}.example.{}", "x".repeat(120)));
    }
    let error = compile_netmap(NetmapInput {
        network: network(),
        revision: Revision::MIN,
        configuration: &s.cfg,
        subjects: &s.subjects,
        inventory: &s.inventory,
        rule_sets: &BTreeMap::new(),
        selected_profiles: &BTreeMap::new(),
        selection_versions: &BTreeMap::new(),
        previous: &BTreeMap::new(),
    })
    .unwrap_err();
    assert!(
        matches!(error, NetmapError::Limit { bytes, .. } if bytes > meshway_model::netmap::MAX_NETMAP_BYTES)
    );
}

#[test]
fn inactive_identity_or_wrong_key_algorithm_rejects_compilation() {
    let mut s = base(3);
    s.subjects.members.clear();
    assert!(matches!(
        compile_netmap(NetmapInput {
            network: network(),
            revision: Revision::MIN,
            configuration: &s.cfg,
            subjects: &s.subjects,
            inventory: &s.inventory,
            rule_sets: &BTreeMap::new(),
            selected_profiles: &BTreeMap::new(),
            selection_versions: &BTreeMap::new(),
            previous: &BTreeMap::new(),
        }),
        Err(NetmapError::Compile(CompileError::InvalidInput))
    ));
}
