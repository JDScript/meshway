use super::*;

fn run(
    s: &Scenario,
    previous: &BTreeMap<NodeId, PreviousNetmap>,
    inputs: &BTreeMap<NodeId, Digest>,
    revision: Revision,
) -> IncrementalNetmaps {
    compile_incremental(
        NetmapInput {
            network: network(),
            revision,
            configuration: &s.cfg,
            subjects: &s.subjects,
            inventory: &s.inventory,
            rule_sets: &BTreeMap::new(),
            selected_profiles: &BTreeMap::new(),
            selection_versions: &s.selection_versions,
            previous,
        },
        inputs,
    )
    .unwrap()
}

#[test]
fn unrelated_components_skip_materialization_and_revision_only_edits_skip_every_output() {
    let mut s = base(6);
    s.cfg.defaults.topology = TopologyDefault::Deny;
    s.cfg.topology = vec![TopologyRule {
        remark: String::new(),
        action: TopologyAction::Allow,
        a: sel(1),
        b: sel(2),
        transports: Input::Omitted,
        keepalive: false,
    }];
    let first = run(&s, &BTreeMap::new(), &BTreeMap::new(), Revision::MIN);
    let previous = previous(&first.compiled);
    let revision = Revision::new(2).unwrap();
    assert!(
        run(&s, &previous, &first.inputs, revision)
            .compiled
            .nodes
            .is_empty()
    );
    s.cfg.nodes.get_mut(&node(1)).unwrap().hostname = "changed".into();
    let next = run(&s, &previous, &first.inputs, revision);
    assert_eq!(
        next.compiled.nodes.keys().copied().collect::<Vec<_>>(),
        [node(1), node(2)]
    );
    let full = compile_with(&s, &previous, revision);
    let mut actual = first.compiled.nodes;
    actual.extend(next.compiled.nodes);
    assert_eq!(actual, full.nodes);
}

#[test]
fn incremental_outputs_equal_full_compilation_across_dependency_changes() {
    for seed in 0..32 {
        let initial = generated(seed);
        let baseline = run(&initial, &BTreeMap::new(), &BTreeMap::new(), Revision::MIN);
        let previous = previous(&baseline.compiled);
        for mutation in 0..15 {
            let mut s = generated(seed);
            match mutation {
                0 => s.cfg.nodes.get_mut(&node(1)).unwrap().hostname = "renamed".into(),
                1 => s.cfg.nodes.get_mut(&node(3)).unwrap().enabled = false,
                2 => {
                    s.inventory.remove(&node(3));
                }
                3 => s.inventory.get_mut(&node(4)).unwrap().capabilities.clear(),
                4 => {
                    s.inventory
                        .get_mut(&node(1))
                        .unwrap()
                        .control_key
                        .generation = Version::new(2).unwrap()
                }
                5 => {
                    s.cfg.topology.clear();
                    s.cfg.defaults.topology = TopologyDefault::Deny;
                }
                6 => {
                    s.selection_versions
                        .insert(node(1), Version::new(2).unwrap());
                }
                7 => {
                    s.subjects
                        .users
                        .insert("alias@example.test".into(), s.subjects.users[ALICE].clone());
                }
                8 => s.cfg.dns.magic_dns = !s.cfg.dns.magic_dns,
                // Exit setting of one Node.
                9 => {
                    let node = s.cfg.nodes.get_mut(&node(4)).unwrap();
                    node.exit_enabled = !node.exit_enabled;
                }
                // Public egress rules.
                10 => s
                    .cfg
                    .routing
                    .egress
                    .rules
                    .push(egress_rule("8.8.4.0/24", RuleAction::Direct)),
                // A profile's rules and its assignment.
                11 => {
                    if let Some(profile) = s.cfg.routing.egress.profiles.get_mut("exit") {
                        profile.rules.clear();
                    }
                }
                12 => {
                    for assignment in &mut s.cfg.routing.egress.assignments {
                        assignment.default = if assignment.default == "mesh" {
                            "exit".into()
                        } else {
                            "mesh".into()
                        };
                    }
                }
                // Tag owners and group members.
                13 => {
                    s.cfg
                        .tags_owners
                        .insert("tag:server".into(), vec![BOB.into()]);
                }
                _ => {
                    s.cfg
                        .groups
                        .insert("group:ops".into(), vec![ALICE.into(), BOB.into()]);
                }
            }
            let revision = Revision::new(2).unwrap();
            let updated = run(&s, &previous, &baseline.inputs, revision);
            let full = compile_with(&s, &previous, revision);
            let mut combined = baseline.compiled.nodes.clone();
            combined.retain(|node, _| updated.inputs.contains_key(node));
            combined.extend(updated.compiled.nodes);
            assert_eq!(combined, full.nodes, "seed={seed}, mutation={mutation}");
        }
    }
}
