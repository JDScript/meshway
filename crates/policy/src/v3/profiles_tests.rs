use super::*;
fn id(i: usize) -> String {
    format!("{i:064x}")
}
fn selector(i: usize) -> String {
    format!("node:{}", id(i))
}
fn global(i: usize) -> String {
    format!("global:{}", id(i))
}
fn facts(n: usize) -> Snapshot {
    Snapshot {
        nodes: (0..n)
            .map(|i| Node {
                id: id(i),
                tags: BTreeSet::from(["tag:member".into()]),
                listener: true,
                exit_enabled: i != 0,
            })
            .collect(),
        tags: BTreeSet::from(["tag:member".into(), "tag:managed".into()]),
        reachable: (0..n)
            .flat_map(|a| {
                (0..n).filter(move |b| *b != a).map(move |b| Dial {
                    from: id(a),
                    to: id(b),
                })
            })
            .collect(),
        rule_sets: BTreeMap::new(),
    }
}
fn internet(src: &str) -> Grant {
    Grant {
        id: "internet".into(),
        src: vec![src.into()],
        dst: vec!["autogroup:internet".into()],
        bidirectional: false,
    }
}
fn rule(name: &str, d: ExitMatch, target: ExitTarget, exit: usize) -> ExitRule {
    ExitRule {
        id: name.into(),
        src: "*".into(),
        destinations: vec![d],
        target,
        candidates: if target == ExitTarget::Nodes {
            vec![selector(exit)]
        } else {
            vec![]
        },
        via: vec![],
    }
}
fn policy() -> NetworkPolicy {
    let mut p = NetworkPolicy::default_template();
    p.grants.push(internet("*"));
    p.profiles.push(RoutingProfile {
        id: "office".into(),
        rules: vec![rule(
            "office",
            ExitMatch::DomainSuffix("example.com".into()),
            ExitTarget::Nodes,
            2,
        )],
        fallback: ProfileFallback::Direct,
    });
    p.assignments.push(ProfileAssignment {
        src: "*".into(),
        allowed: vec![
            MESH_ONLY.into(),
            AUTO.into(),
            "global:*".into(),
            "office".into(),
        ],
        default_profile: MESH_ONLY.into(),
        locked: false,
    });
    p
}
fn unknown() -> StunLocation {
    StunLocation { region: None }
}
fn target(c: &NetworkCompilation, decision: ExitDecision<'_>) -> Option<u16> {
    match decision {
        ExitDecision::Local => None,
        ExitDecision::Paths(p) => c.base.paths[p[0] as usize].last().copied(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn public_rules_win_in_mesh_only_global_custom_and_automatic_profiles() {
    let f = facts(3);
    let mut p = policy();
    p.public_rules.push(rule(
        "public",
        ExitMatch::DomainSuffix("example.com".into()),
        ExitTarget::Direct,
        0,
    ));
    p.automatic.regions.insert("CN".into(), "office".into());
    let c = compile_network(&p, &f, CompileOptions::default()).unwrap();
    for profile in [MESH_ONLY, "office", AUTO, &global(1)] {
        assert_eq!(
            c.exit_for_profile(
                0,
                Some(profile),
                &StunLocation {
                    region: Some("CN".into())
                },
                Some("example.com"),
                Some("192.0.2.1".parse().unwrap())
            )
            .unwrap(),
            ExitDecision::Local
        );
    }
    assert!(
        c.sources[0]
            .profiles
            .iter()
            .find(|p| p.id == "office")
            .unwrap()
            .lints
            .iter()
            .any(|l| l.rule_id == "office" && l.fully_shadowed)
    );
    // Public routes still execute under the mesh-only built-in, even though its final is none.
    p.public_rules[0].target = ExitTarget::Nodes;
    p.public_rules[0].candidates = vec![selector(1)];
    let c = compile_network(&p, &f, CompileOptions::default()).unwrap();
    assert_eq!(
        target(
            &c,
            c.exit_for_profile(
                0,
                Some(MESH_ONLY),
                &unknown(),
                Some("example.com"),
                Some("192.0.2.1".parse().unwrap())
            )
            .unwrap()
        ),
        Some(1)
    );
}
#[test]
fn built_in_globals_filter_enabled_exits_and_source_authorization() {
    let f = facts(3);
    let mut p = NetworkPolicy::default_template();
    p.grants.push(internet(&selector(0)));
    let c = compile_network(&p, &f, CompileOptions::default()).unwrap();
    assert!(c.sources[0].allowed.contains(&global(1)));
    assert!(c.sources[0].allowed.contains(&global(2)));
    assert!(!c.sources[0].allowed.contains(&global(0)));
    assert!(
        !c.sources[1]
            .allowed
            .iter()
            .any(|id| id.starts_with("global:"))
    );
    assert_eq!(
        target(
            &c,
            c.exit_for_profile(
                0,
                Some(&global(2)),
                &unknown(),
                None,
                Some("192.0.2.1".parse().unwrap())
            )
            .unwrap()
        ),
        Some(2)
    );
    assert!(
        c.base
            .external_path_authorization(2)
            .grants
            .iter()
            .any(|g| g.path == vec![id(0), id(2)])
    );
    assert!(c.select_profile(1, Some(&global(2)), &unknown()).is_err());
}
#[test]
fn automatic_stun_region_unknown_fallback_and_cycle_rejection() {
    let f = facts(3);
    let mut p = policy();
    p.automatic.regions.insert("CN".into(), "office".into());
    p.automatic.regions.insert("US".into(), global(1));
    let c = compile_network(&p, &f, CompileOptions::default()).unwrap();
    assert_eq!(
        c.select_profile(
            0,
            Some(AUTO),
            &StunLocation {
                region: Some("cn".into())
            }
        )
        .unwrap()
        .id,
        "office"
    );
    assert_eq!(
        c.select_profile(
            0,
            Some(AUTO),
            &StunLocation {
                region: Some("US".into())
            }
        )
        .unwrap()
        .id,
        global(1)
    );
    assert_eq!(
        c.select_profile(0, Some(AUTO), &unknown()).unwrap().id,
        MESH_ONLY
    );
    assert_eq!(
        c.select_profile(
            0,
            Some(AUTO),
            &StunLocation {
                region: Some("ZZ".into())
            }
        )
        .unwrap()
        .id,
        MESH_ONLY
    );
    p.automatic.regions.insert("CN".into(), AUTO.into());
    assert!(
        compile_network(&p, &f, CompileOptions::default())
            .unwrap_err()
            .iter()
            .any(|d| d.message.contains("cycles forbidden"))
    );
    p.automatic.regions.clear();
    p.automatic.fallback = AUTO.into();
    assert!(compile_network(&p, &f, CompileOptions::default()).is_err());
}
#[test]
fn lock_specificity_conflicts_and_switch_never_recompile_mesh_layers() {
    let f = facts(3);
    let mut p = policy();
    p.assignments.push(ProfileAssignment {
        src: selector(0),
        allowed: vec![MESH_ONLY.into(), global(1)],
        default_profile: global(1),
        locked: true,
    });
    let c = compile_network(&p, &f, CompileOptions::default()).unwrap();
    let before = c.base.clone();
    assert_eq!(
        c.select_profile(0, Some("nonexistent"), &unknown())
            .unwrap()
            .id,
        global(1)
    );
    assert_eq!(
        c.select_profile(1, Some("office"), &unknown()).unwrap().id,
        "office"
    );
    assert_eq!(c.base, before);
    let mut shuffled = p.clone();
    shuffled.connections.reverse();
    shuffled.grants.reverse();
    shuffled.mesh.reverse();
    shuffled.profiles.reverse();
    shuffled.assignments.reverse();
    assert_eq!(
        c,
        compile_network(&shuffled, &f, CompileOptions::default()).unwrap()
    );
    p.assignments.push(ProfileAssignment {
        src: selector(0),
        allowed: vec![MESH_ONLY.into()],
        default_profile: MESH_ONLY.into(),
        locked: false,
    });
    assert!(
        compile_network(&p, &f, CompileOptions::default())
            .unwrap_err()
            .iter()
            .any(|d| d.message.contains("Conflicts assignments"))
    );
}
#[test]
fn unauthorized_profile_assignment_rejected_including_partial_ranges_and_final() {
    let f = facts(3);
    let mut p = NetworkPolicy::default_template();
    p.grants.push(Grant {
        id: "lan".into(),
        src: vec![selector(0)],
        dst: vec!["ip:192.168.1.0/25".into()],
        bidirectional: false,
    });
    p.profiles.push(RoutingProfile {
        id: "lan".into(),
        rules: vec![rule(
            "lan",
            ExitMatch::Cidr("192.168.1.0/24".into()),
            ExitTarget::Nodes,
            1,
        )],
        fallback: ProfileFallback::None,
    });
    p.assignments.push(ProfileAssignment {
        src: selector(0),
        allowed: vec![MESH_ONLY.into(), "lan".into()],
        default_profile: "lan".into(),
        locked: false,
    });
    let error = compile_network(&p, &f, CompileOptions::default()).unwrap_err();
    assert!(error.iter().any(|d|d.field=="assignments[0].allowed"&&d.message.contains("not fully covered")));
    p.grants
        .last_mut()
        .unwrap()
        .dst
        .push("ip:192.168.1.128/25".into());
    let c = compile_network(&p, &f, CompileOptions::default()).unwrap();
    assert_eq!(
        target(
            &c,
            c.exit_for_profile(
                0,
                None,
                &unknown(),
                None,
                Some("192.168.1.1".parse().unwrap())
            )
            .unwrap()
        ),
        Some(1)
    );
    p.profiles[0].fallback = ProfileFallback::Nodes {
        candidates: vec![selector(1)],
        via: vec![],
    };
    assert!(
        compile_network(&p, &f, CompileOptions::default())
            .unwrap_err()
            .iter()
            .any(|d| d.message.contains("final exit requires"))
    );
}
#[test]
fn automatic_target_cannot_escape_allowed_set_or_grants() {
    let f = facts(3);
    let mut p = policy();
    p.assignments[0].allowed = vec![MESH_ONLY.into(), AUTO.into()];
    p.automatic.regions.insert("CN".into(), "office".into());
    assert!(
        compile_network(&p, &f, CompileOptions::default())
            .unwrap_err()
            .iter()
            .any(|d| d.message.contains("must also be explicitly allowed"))
    );
}

#[test]
#[ignore]
fn profile_eight_thousand_cidr_timing() {
    let mut f = facts(1000);
    for (i, n) in f.nodes.iter_mut().enumerate() {
        n.exit_enabled = i == 999;
    }
    f.reachable.retain(|d| d.to == id(999));
    f.rule_sets.insert(
        "cn".into(),
        ParsedRuleSet {
            entries: (0..8000)
                .map(|i| {
                    let block = (i * 7919) % 65536;
                    RuleSetEntry::Cidr(format!("10.{}.{}.0/24", block / 256, block % 256))
                })
                .collect(),
        },
    );
    let mut p = NetworkPolicy::default_template();
    p.grants = vec![internet("*")];
    p.public_rules = vec![rule(
        "direct",
        ExitMatch::Cidr("10.0.0.0/16".into()),
        ExitTarget::Direct,
        0,
    )];
    p.profiles = vec![RoutingProfile {
        id: "regional".into(),
        rules: vec![rule(
            "cn",
            ExitMatch::RuleSet("cn".into()),
            ExitTarget::Nodes,
            999,
        )],
        fallback: ProfileFallback::None,
    }];
    p.assignments = vec![ProfileAssignment {
        src: "*".into(),
        allowed: vec![MESH_ONLY.into(), "regional".into(), "global:*".into()],
        default_profile: MESH_ONLY.into(),
        locked: false,
    }];
    let start = std::time::Instant::now();
    let c = compile_network(&p, &f, CompileOptions::default()).unwrap();
    let compile_time = start.elapsed();
    assert_eq!(c.ip_tables.len(), 2);
    assert_eq!(c.sources.len(), 1000);
    let start = std::time::Instant::now();
    for i in 0..100_000 {
        let ip = std::net::Ipv4Addr::new(
            10,
            ((i * 7919) % 65536 / 256) as u8,
            ((i * 7919) % 256) as u8,
            1,
        );
        std::hint::black_box(
            c.exit_for_profile(0, Some("regional"), &unknown(), None, Some(ip.into()))
                .unwrap(),
        );
    }
    eprintln!(
        "Profiles: 1000 sources, 8000 CIDRs, public+custom+global: compile={compile_time:?}; 100000 lookups={:?}; tables={}, external paths={}",
        start.elapsed(),
        c.ip_tables.len(),
        c.base.paths.len()
    );
}
