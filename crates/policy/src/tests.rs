use super::*;
fn id(i: usize) -> String {
    format!("{i:064x}")
}
fn selector(i: usize) -> String {
    format!("node:{}", id(i))
}
fn facts(n: usize) -> Snapshot {
    Snapshot {
        rule_sets: Default::default(),
        nodes: (0..n)
            .map(|i| Node {
                id: id(i),
                tags: if i == 1 || i == 2 {
                    BTreeSet::from(["tag:relay".into()])
                } else {
                    BTreeSet::new()
                },
                listener: true,
                exit_enabled: true,
            })
            .collect(),
        tags: BTreeSet::from(["tag:relay".into()]),
        reachable: (0..n)
            .flat_map(|a| {
                (0..n).filter(move |b| *b != a).map(move |b| Dial {
                    from: id(a),
                    to: id(b),
                })
            })
            .collect(),
    }
}
fn compile_ok(p: &Policy, f: &Snapshot) -> Compiled {
    compile(p, f, CompileOptions::default()).unwrap()
}
fn path(c: &Compiled, a: u16, b: u16) -> Vec<Vec<u16>> {
    c.mesh
        .iter()
        .find(|f| f.src == a && f.dst == b)
        .unwrap()
        .paths
        .iter()
        .map(|i| c.paths[*i as usize].clone())
        .collect()
}
fn block(a: usize, b: usize) -> ConnectionRule {
    ConnectionRule {
        id: format!("block-{a}-{b}"),
        a: selector(a),
        b: selector(b),
        action: ConnectionAction::Block,
    }
}
fn mesh(a: usize, b: usize, via: Vec<String>) -> MeshRule {
    MeshRule {
        id: format!("mesh-{a}-{b}"),
        src: selector(a),
        dst: selector(b),
        via,
    }
}
fn exit(id: &str, d: ExitMatch, candidates: Vec<String>) -> ExitRule {
    ExitRule {
        id: id.into(),
        src: "*".into(),
        destinations: vec![d],
        target: ExitTarget::Nodes,
        candidates,
        via: vec![],
    }
}
fn internet() -> Grant {
    Grant {
        id: "internet".into(),
        src: vec!["*".into()],
        dst: vec!["autogroup:internet".into()],
        bidirectional: false,
    }
}

#[test]
fn automatic_routing_json_map_cases_match_wire_acceptance_and_preserve_keys() {
    use serde_json::Value;

    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../scripts/fixtures/policy-map-cases.json"
    ))
    .unwrap();
    for case in cases {
        let automatic = &case["automatic"];
        let expected = case["valid"].as_bool().unwrap();
        let parsed = serde_json::from_value::<AutomaticRouting>(automatic.clone());
        assert_eq!(parsed.is_ok(), expected, "{}", case["label"]);
        if let Ok(parsed) = parsed {
            assert_eq!(serde_json::to_value(parsed).unwrap(), *automatic);
        }
        let mut policy = serde_json::to_value(NetworkPolicy::default_template()).unwrap();
        policy["automatic"] = automatic.clone();
        let parsed = serde_json::from_value::<NetworkPolicy>(policy.clone());
        assert_eq!(parsed.is_ok(), expected, "{}", case["label"]);
        if let Ok(parsed) = parsed {
            assert_eq!(serde_json::to_value(parsed).unwrap(), policy);
        }
    }
}

#[test]
fn exit_match_wire_and_schema_reject_unknown_or_missing_variant_fields() {
    use serde_json::json;
    use utoipa::OpenApi;

    #[derive(OpenApi)]
    #[openapi(components(schemas(ExitMatch)))]
    struct Schema;

    let document = serde_json::to_value(Schema::openapi()).unwrap();
    let variants = document["components"]["schemas"]["ExitMatch"]["oneOf"]
        .as_array()
        .unwrap();
    assert_eq!(variants.len(), 3);
    for (kind, value, expected) in [
        (
            "domain_suffix",
            "example.com",
            ExitMatch::DomainSuffix("example.com".into()),
        ),
        ("cidr", "10.0.0.0/8", ExitMatch::Cidr("10.0.0.0/8".into())),
        (
            "rule_set",
            "regional",
            ExitMatch::RuleSet("regional".into()),
        ),
    ] {
        let wire = serde_json::to_value(&expected).unwrap();
        assert_eq!(wire, json!({"kind": kind, "value": value}));
        assert_eq!(
            serde_json::to_string(&expected).unwrap(),
            format!(r#"{{"kind":"{kind}","value":"{value}"}}"#)
        );
        assert_eq!(
            serde_json::from_value::<ExitMatch>(wire.clone()).unwrap(),
            expected
        );
        let variant = variants
            .iter()
            .find(|variant| variant["properties"]["kind"]["enum"] == json!([kind]))
            .unwrap();
        assert_eq!(variant["type"], "object");
        assert_eq!(variant["additionalProperties"], false);
        assert_eq!(variant["maxProperties"], 2);
        assert_eq!(variant["properties"].as_object().unwrap().len(), 2);
        assert_eq!(variant["properties"]["kind"]["type"], "string");
        assert_eq!(variant["properties"]["value"], json!({"type": "string"}));
        let required = variant["required"].as_array().unwrap();
        assert_eq!(required.len(), 2);
        assert!(required.contains(&json!("kind")));
        assert!(required.contains(&json!("value")));

        let mut unknown = wire.clone();
        unknown["extra"] = json!(true);
        assert!(serde_json::from_value::<ExitMatch>(unknown).is_err());
        for missing in ["kind", "value"] {
            let mut invalid = wire.clone();
            invalid.as_object_mut().unwrap().remove(missing);
            assert!(serde_json::from_value::<ExitMatch>(invalid).is_err());
        }
        assert!(serde_json::from_value::<ExitMatch>(json!({"kind": kind, "value": null})).is_err());
        assert!(serde_json::from_value::<ExitMatch>(json!({"kind": kind, "value": 42})).is_err());
    }
    assert!(serde_json::from_value::<ExitMatch>(json!({"kind": "unknown", "value": "x"})).is_err());
    assert!(serde_json::from_value::<ExitMatch>(json!({"kind": null, "value": "x"})).is_err());
}

#[test]
fn legal_small_cidr_input_cannot_publish_oversized_flattened_table() {
    // 1024 isolated host exceptions before /0 create 17416 disjoint prefixes,
    // despite only 1025 input destinations. Count with a separate interval
    // CIDR-cover oracle, independent of the compiler's trie partitioning.
    let hosts: Vec<u32> = (256..1280).map(|i| (i << 16) + 1).collect();
    let count_cover = |mut start: u64, end: u64| {
        let mut count = 0;
        while start < end {
            let bits = start
                .trailing_zeros()
                .min(32)
                .min(63 - (end - start).leading_zeros());
            start += 1 << bits;
            count += 1;
        }
        count
    };
    let mut count = hosts.len();
    let mut start = 0;
    for host in &hosts {
        count += count_cover(start, u64::from(*host));
        start = u64::from(*host) + 1;
    }
    count += count_cover(start, 1 << 32);
    assert_eq!(count, 17416);
    let mut exceptions = exit("hosts", ExitMatch::Cidr("0.0.0.1/32".into()), vec![]);
    exceptions.target = ExitTarget::Direct;
    exceptions.destinations = hosts
        .into_iter()
        .map(|ip| ExitMatch::Cidr(format!("{}/32", std::net::Ipv4Addr::from(ip))))
        .collect();
    let mut rest = exit("rest", ExitMatch::Cidr("0.0.0.0/0".into()), vec![]);
    rest.target = ExitTarget::Direct;
    let p = Policy {
        exits: vec![exceptions.clone(), rest.clone()],
        ..Policy::default()
    };
    let f = facts(2);
    assert!(validate(&p, &f).is_empty());
    let errors = compile(&p, &f, CompileOptions::default()).unwrap_err();
    assert_eq!(errors[0].field, "compiled.source_ip_table[0]");
    assert!(errors[0].message.contains("16384; no partial output"));

    let p = NetworkPolicy {
        public_rules: vec![exceptions, rest],
        ..NetworkPolicy::default_template()
    };
    let errors = compile_network(&p, &f, CompileOptions::default()).unwrap_err();
    assert!(errors[0].field.starts_with("compiled.sources[0].profiles["));
    assert!(errors[0].message.contains("16384; no partial output"));
}

#[test]
fn block_priority_automatic_relay_and_order_independence() {
    let mut p = Policy::default_template();
    p.connections.push(block(0, 3));
    p.connections.push(block(0, 1));
    let f = facts(4);
    let c = compile_ok(&p, &f);
    assert_eq!(path(&c, 0, 3)[0], vec![0, 2, 3]);
    assert!(!c.connections.contains(&(0, 3)));
    assert!(!c.connections.contains(&(1, 0)));
    p.connections.reverse();
    p.grants.reverse();
    assert_eq!(c, compile_ok(&p, &f));
}
#[test]
fn manual_graph_and_directional_listener_facts() {
    let mut p = Policy::default_template();
    p.connections.clear();
    let mut f = facts(3);
    assert!(path(&compile_ok(&p, &f), 0, 2).is_empty());
    p.connections = vec![
        ConnectionRule {
            id: "edge1".into(),
            a: selector(0),
            b: selector(1),
            action: ConnectionAction::Allow,
        },
        ConnectionRule {
            id: "edge2".into(),
            a: selector(1),
            b: selector(2),
            action: ConnectionAction::Allow,
        },
    ];
    f.nodes[0].listener = false;
    f.nodes[2].listener = false;
    let c = compile_ok(&p, &f);
    assert_eq!(path(&c, 2, 0), vec![vec![2, 1, 0]]);
    assert_eq!(c.connections, BTreeSet::from([(0, 1), (2, 1)]));
}
#[test]
fn mandatory_hops_tag_candidates_backups_and_unreachable() {
    let f = facts(4);
    let mut p = Policy::default_template();
    p.mesh.push(mesh(0, 3, vec!["tag:relay".into()]));
    let c = compile_ok(&p, &f);
    assert_eq!(path(&c, 0, 3), vec![vec![0, 1, 3], vec![0, 2, 3]]);
    p.mesh[0].via = vec![selector(2), selector(1)];
    assert_eq!(path(&compile_ok(&p, &f), 0, 3)[0], vec![0, 2, 1, 3]);
    for a in [0, 1, 3] {
        p.connections.push(block(a, 2));
    }
    let c = compile_ok(&p, &f);
    assert!(path(&c, 0, 3).is_empty());
    assert!(
        c.failures
            .iter()
            .any(|e| e.src == 0 && e.destination == id(3))
    );
}
#[test]
fn grants_union_bidirectional_and_routing_never_authorizes() {
    let f = facts(4);
    let mut p = Policy::default_template();
    p.grants.clear();
    p.mesh.push(mesh(0, 3, vec![selector(1)]));
    assert!(compile_ok(&p, &f).mesh.is_empty());
    p.grants.push(Grant {
        id: "pair".into(),
        src: vec![selector(0)],
        dst: vec![selector(3)],
        bidirectional: true,
    });
    p.grants.push(Grant {
        id: "second".into(),
        src: vec![selector(1)],
        dst: vec![selector(2)],
        bidirectional: false,
    });
    let c = compile_ok(&p, &f);
    assert_eq!(c.mesh.len(), 3);
    assert_eq!(path(&c, 0, 3)[0], vec![0, 1, 3]);
    assert_eq!(path(&c, 3, 0)[0], vec![3, 0]);
    let exact = c.exact_authorization(1);
    assert!(
        exact
            .grants
            .iter()
            .any(|g| g.path == vec![id(0), id(1), id(3)])
    );
    assert!(!exact.grants.iter().any(|g| g.path == vec![id(0), id(1)]));
    p.grants.reverse();
    assert_eq!(c, compile_ok(&p, &f));
}
#[test]
fn mesh_first_match_preserves_order_without_specificity_or_conflict_rejection() {
    let mut p = Policy::default_template();
    p.mesh = vec![
        MeshRule {
            id: "general".into(),
            src: "*".into(),
            dst: selector(3),
            via: vec![selector(1)],
        },
        MeshRule {
            id: "specific".into(),
            src: selector(0),
            dst: "*".into(),
            via: vec![selector(2)],
        },
    ];
    let f = facts(4);
    let c = compile_ok(&p, &f);
    assert_eq!(path(&c, 0, 3)[0], vec![0, 1, 3]);
    assert_eq!(c.mesh_lints.len(), 1);
    let lint = &c.mesh_lints[0];
    assert_eq!(lint.rule_id, "specific");
    assert_eq!((lint.shadowed_flows, lint.total_flows), (1, 3));
    assert!(!lint.fully_shadowed);
    assert!(lint.inventory_dependent);
    assert_eq!((lint.example.src, lint.example.dst), (0, 3));
    assert_eq!(lint.example.winning_rule_id, "general");
    assert_eq!(lint.example.winning_via, vec![selector(1)]);
    assert_eq!(lint.example.shadowed_via, vec![selector(2)]);
    p.mesh.reverse();
    let reversed = compile_ok(&p, &f);
    assert_eq!(path(&reversed, 0, 3)[0], vec![0, 2, 3]);
    assert_ne!(c, reversed);
    p.mesh.push(MeshRule {
        id: "conflict".into(),
        src: selector(0),
        dst: "*".into(),
        via: vec![selector(1)],
    });
    assert!(validate(&p, &f).is_empty());
    let c = compile_ok(&p, &f);
    let lint = c
        .mesh_lints
        .iter()
        .find(|l| l.rule_id == "conflict")
        .unwrap();
    assert!(lint.fully_shadowed);
    assert_eq!((lint.shadowed_flows, lint.total_flows), (3, 3));
    p.mesh.clear();
    p.mesh.push(mesh(0, 3, vec![selector(1)]));
    p.mesh.push(MeshRule {
        id: "duplicate-action".into(),
        ..p.mesh[0].clone()
    });
    assert!(validate(&p, &f).is_empty());
    assert!(compile_ok(&p, &f).mesh_lints[0].fully_shadowed);

    // An unreachable first match must never try the later rule or automatic path.
    p.mesh[0].via = vec![selector(1)];
    p.mesh[1].via.clear();
    p.connections.push(block(0, 1));
    p.connections.push(block(1, 2));
    p.connections.push(block(1, 3));
    assert!(path(&compile_ok(&p, &f), 0, 3).is_empty());
}
#[test]
fn mesh_lints_expand_current_tags_and_recompute_after_inventory_changes() {
    let mut f = facts(4);
    f.tags.insert("tag:laptop".into());
    f.nodes[0].tags.insert("tag:laptop".into());
    let mut p = Policy::default_template();
    p.mesh = vec![
        MeshRule {
            id: "first".into(),
            src: selector(0),
            dst: selector(3),
            via: vec![selector(1)],
        },
        MeshRule {
            id: "tag".into(),
            src: "tag:laptop".into(),
            dst: selector(3),
            via: vec![selector(2)],
        },
    ];
    let c = compile_ok(&p, &f);
    assert!(c.mesh_lints[0].fully_shadowed);
    f.nodes[1].tags.insert("tag:laptop".into());
    let c = compile_ok(&p, &f);
    assert!(!c.mesh_lints[0].fully_shadowed);
    assert_eq!(
        (c.mesh_lints[0].shadowed_flows, c.mesh_lints[0].total_flows),
        (1, 2)
    );
    assert_eq!(path(&c, 1, 3)[0], vec![1, 2, 3]);
    f.nodes.reverse();
    assert_eq!(c, compile_ok(&p, &f));
    for node in &mut f.nodes {
        node.tags.remove("tag:laptop");
    }
    assert!(compile_ok(&p, &f).mesh_lints.is_empty());
}
#[test]
fn exits_ordered_boundary_defaults_and_grant_fence() {
    let mut p = Policy::default_template();
    let f = facts(4);
    p.exits = vec![
        exit(
            "domain",
            ExitMatch::DomainSuffix("example.com".into()),
            vec![selector(1)],
        ),
        exit(
            "longer",
            ExitMatch::DomainSuffix("api.example.com".into()),
            vec![selector(2)],
        ),
        exit(
            "ip",
            ExitMatch::Cidr("192.0.2.0/24".into()),
            vec![selector(3)],
        ),
    ];
    let c = compile_ok(&p, &f);
    assert_eq!(
        c.exit_for(0, Some("example.com"), None).unwrap(),
        ExitDecision::Denied
    );
    assert_eq!(
        c.exit_for(0, Some("badexample.com"), None).unwrap(),
        ExitDecision::Local
    );
    p.grants.push(internet());
    let c = compile_ok(&p, &f);
    let chosen = |c: &Compiled, domain: Option<&str>, ip: Option<&str>| match c
        .exit_for(0, domain, ip.map(|s| s.parse().unwrap()))
        .unwrap()
    {
        ExitDecision::Paths(ps) => *c.paths[ps[0] as usize].last().unwrap(),
        d => panic!("{d:?}"),
    };
    assert_eq!(
        chosen(&c, Some("www.api.example.com"), Some("192.0.2.1")),
        1
    );
    assert_eq!(chosen(&c, None, Some("192.0.2.1")), 3);
    p.exits.push(exit(
        "default4",
        ExitMatch::Cidr("0.0.0.0/0".into()),
        vec![selector(1)],
    ));
    p.exits.push(exit(
        "default6",
        ExitMatch::Cidr("::/0".into()),
        vec![selector(2)],
    ));
    let c = compile_ok(&p, &f);
    assert_eq!(chosen(&c, None, Some("203.0.113.1")), 1);
    assert_eq!(chosen(&c, None, Some("2001:db8::1")), 2);
    assert_eq!(chosen(&c, None, Some("192.0.2.1")), 3);
    p.exits.reverse();
    assert_ne!(c, compile_ok(&p, &f));
}
#[test]
fn exit_primary_backup_candidates_and_conflicts() {
    let mut p = Policy::default_template();
    p.grants.push(internet());
    p.exits.push(exit(
        "exit",
        ExitMatch::Cidr("0.0.0.0/0".into()),
        vec![selector(2), selector(1)],
    ));
    p.connections.push(block(0, 2));
    let c = compile_ok(&p, &facts(3));
    let e = &c.exits[0][0];
    assert_eq!(c.paths[e.paths[0] as usize], vec![0, 1, 2]);
    assert!(e.paths.iter().any(|i| c.paths[*i as usize] == vec![0, 1]));
    p.exits.push(exit(
        "conflict",
        ExitMatch::Cidr("0.0.0.0/0".into()),
        vec![selector(0)],
    ));
    let c = compile_ok(&p, &facts(3));
    assert!(
        c.lints
            .iter()
            .any(|l| l.rule_id == "conflict" && l.fully_shadowed)
    );
}
#[test]
fn unsupported_fields_selectors_and_limits_fail_closed() {
    assert!(
        serde_json::from_str::<Policy>(
            r#"{"connections":[],"grants":[],"mesh":[],"exits":[],"unknown":1}"#
        )
        .is_err()
    );
    let mut p = Policy::default_template();
    p.grants[0].src = vec!["user:alice".into()];
    assert!(
        validate(&p, &facts(2))
            .iter()
            .any(|e| e.field == "grants[0].src[0]" && e.message.contains("unsupported"))
    );
    p.grants[0].src = vec!["group:ops".into()];
    assert!(compile(&p, &facts(2), CompileOptions::default()).is_err());
    p = Policy::default_template();
    p.mesh.push(mesh(0, 1, vec!["*".into()]));
    assert!(!validate(&p, &facts(2)).is_empty());
    let schema = <Policy as utoipa::PartialSchema>::schema();
    assert!(
        serde_json::to_string(&schema)
            .unwrap()
            .contains("connections")
    );
}
/// Run explicitly in release mode to avoid noisy CI timing assertions.
#[test]
#[ignore]
fn thousand_node_timing() {
    let f = facts(1000);
    let p = Policy::default_template();
    for paths in [1, 2] {
        let start = std::time::Instant::now();
        let c = compile(&p, &f, CompileOptions { paths }).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(c.mesh.len(), 999_000);
        assert_eq!(c.paths.len(), 999_000 * paths);
        eprintln!(
            "1000 nodes, dense graph, allow-all, {paths} paths: {elapsed:?}; {} paths",
            c.paths.len()
        );
    }
    let mut sparse = f;
    sparse
        .reachable
        .retain(|d| d.from == id(0) || d.to == id(0));
    let start = std::time::Instant::now();
    let c = compile(&p, &sparse, CompileOptions::default()).unwrap();
    assert_eq!(c.mesh.len(), 999_000);
    assert_eq!(c.paths.len(), 999_000);
    eprintln!(
        "1000 nodes, star graph, allow-all, primary+backup search: {:?}",
        start.elapsed()
    );
}

#[test]
fn all_four_node_graphs_match_exhaustive_simple_path_oracle() {
    fn enumerate(
        f: &Snapshot,
        current: usize,
        dst: usize,
        path: &mut Vec<u16>,
        out: &mut Vec<Vec<u16>>,
    ) {
        if current == dst {
            out.push(path.clone());
            return;
        }
        for next in 0..f.nodes.len() {
            if !path.contains(&(next as u16))
                && (f.reachable.contains(&Dial {
                    from: id(current),
                    to: id(next),
                }) || f.reachable.contains(&Dial {
                    from: id(next),
                    to: id(current),
                }))
            {
                path.push(next as u16);
                enumerate(f, next, dst, path, out);
                path.pop();
            }
        }
    }
    let edges = [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)];
    for mask in 0..64 {
        let mut f = facts(4);
        f.reachable = edges
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, (a, b))| Dial {
                from: id(*a),
                to: id(*b),
            })
            .collect();
        for via in [
            vec![],
            vec![selector(1)],
            vec!["tag:relay".into()],
            vec![selector(2), selector(1)],
        ] {
            let mut p = Policy::default_template();
            p.grants = vec![Grant {
                id: "pair".into(),
                src: vec![selector(0)],
                dst: vec![selector(3)],
                bidirectional: false,
            }];
            p.mesh = vec![mesh(0, 3, via.clone())];
            let c = compile_ok(&p, &f);
            let mut all = vec![];
            enumerate(&f, 0, 3, &mut vec![0], &mut all);
            all.retain(|path| {
                let mut stage = 0;
                for &n in &path[1..path.len() - 1] {
                    if stage < via.len() && super::matches(&via[stage], &f.nodes[usize::from(n)]) {
                        stage += 1;
                    }
                }
                stage == via.len()
            });
            all.sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));
            all.truncate(2);
            assert_eq!(path(&c, 0, 3), all, "mask={mask}, via={via:?}");
        }
    }
}

#[test]
fn thousand_nodes_with_required_hop_do_not_exhaust_search_on_dense_graph() {
    let f = facts(1000);
    let mut p = Policy::default_template();
    p.grants = vec![Grant {
        id: "pair".into(),
        src: vec![selector(0)],
        dst: vec![selector(999)],
        bidirectional: false,
    }];
    p.mesh = vec![mesh(0, 999, vec![selector(500)])];
    let c = compile_ok(&p, &f);
    assert_eq!(path(&c, 0, 999)[0], vec![0, 500, 999]);
    assert!(c.failures.is_empty());
}

fn default_rule() -> DefaultExitRule {
    DefaultExitRule {
        id: "user-defaults".into(),
        src: "*".into(),
        families: BTreeSet::from([AddressFamily::Ipv4, AddressFamily::Ipv6]),
        candidates: vec![selector(1), selector(2)],
        via: vec![],
        locked: false,
        target: ExitTarget::Nodes,
    }
}
#[test]
fn runtime_default_exit_selection_specific_priority_lock_and_invalid_selection() {
    let f = facts(4);
    let mut p = Policy::default_template();
    p.grants.push(internet());
    p.defaults.push(default_rule());
    let ip = Some("192.0.2.1".parse().unwrap());
    let c = compile_ok(&p, &f);
    assert_eq!(
        c.exit_for_selected(0, None, ip, None).unwrap(),
        ExitDecision::Local
    );
    assert_eq!(
        c.exit_for_selected(0, None, ip, Some(3)).unwrap(),
        ExitDecision::Denied
    );
    let choice = c.exit_for_selected(0, None, ip, Some(2)).unwrap();
    let ExitDecision::Paths(paths) = choice else {
        panic!("{choice:?}");
    };
    assert_eq!(c.paths[paths[0] as usize], vec![0, 2]);
    // Every allowed source/exit combination is projected before runtime selection.
    assert!(
        c.external_path_authorization(2)
            .grants
            .iter()
            .any(|g| g.path == vec![id(0), id(2)])
    );
    p.exits.push(exit(
        "specific",
        ExitMatch::Cidr("192.0.2.0/24".into()),
        vec![selector(3)],
    ));
    let c = compile_ok(&p, &f);
    let ExitDecision::Paths(paths) = c.exit_for_selected(0, None, ip, Some(2)).unwrap() else {
        panic!();
    };
    assert_eq!(c.paths[paths[0] as usize], vec![0, 3]);
    p.defaults[0].locked = true;
    let c = compile_ok(&p, &f);
    let other = Some("203.0.113.1".parse().unwrap());
    for selected in [None, Some(2), Some(999)] {
        let ExitDecision::Paths(paths) = c.exit_for_selected(0, None, other, selected).unwrap()
        else {
            panic!();
        };
        assert_eq!(c.paths[paths[0] as usize], vec![0, 1]);
    }
    p.defaults.reverse();
    assert_eq!(c, compile_ok(&p, &f));
    p.grants.retain(|g| g.id != "internet");
    let c = compile_ok(&p, &f);
    assert_eq!(
        c.exit_for_selected(0, None, other, Some(1)).unwrap(),
        ExitDecision::Denied
    );
}
#[test]
fn default_exit_conflicts_aliases_specificity_and_required_hops() {
    let f = facts(4);
    let mut p = Policy::default_template();
    p.grants.push(internet());
    p.defaults.push(default_rule());
    p.exits.push(exit(
        "zero-alias",
        ExitMatch::Cidr("0.0.0.0/0".into()),
        vec![selector(3)],
    ));
    assert!(validate(&p, &f).is_empty());
    p.exits[0].src = selector(0);
    let c = compile_ok(&p, &f);
    let ExitDecision::Paths(paths) = c
        .exit_for_selected(0, None, Some("192.0.2.1".parse().unwrap()), Some(1))
        .unwrap()
    else {
        panic!();
    };
    assert_eq!(c.paths[paths[0] as usize], vec![0, 3]);
    assert!(!c.defaults[1][0].locked);
    p.defaults[0].via = vec![selector(3)];
    p.exits.clear();
    let c = compile_ok(&p, &f);
    let e = &c.defaults[0][0];
    let candidate = e.candidates.iter().find(|c| c.node == 1).unwrap();
    assert_eq!(c.paths[candidate.paths[0] as usize], vec![0, 3, 1]);
    let mut seen = BTreeSet::new();
    assert!(
        c.exact_authorization(3)
            .grants
            .iter()
            .all(|g| seen.insert(g.path.clone()))
    );
}

#[test]
fn administrator_exit_enablement_is_required_but_never_limits_relays() {
    let mut f = facts(4);
    f.nodes[1].exit_enabled = false;
    let mut p = Policy::default_template();
    p.grants.push(internet());
    p.defaults.push(default_rule());
    assert!(
        validate(&p, &f)
            .iter()
            .any(|d| d.field == "defaults[0].candidates[0]" && d.message.contains("not enabled"))
    );
    p.defaults[0].candidates = vec!["tag:relay".into()];
    p.mesh.push(mesh(0, 3, vec![selector(1)]));
    let c = compile_ok(&p, &f);
    assert_eq!(path(&c, 0, 3)[0], vec![0, 1, 3]);
    assert_eq!(
        c.defaults[0][0]
            .candidates
            .iter()
            .map(|c| c.node)
            .collect::<Vec<_>>(),
        vec![2]
    );
    f.nodes[2].exit_enabled = false;
    assert!(
        validate(&p, &f)
            .iter()
            .any(|d| d.message.contains("candidate unavailable"))
    );
    p.defaults.clear();
    p.exits.push(exit(
        "node-disabled",
        ExitMatch::DomainSuffix("example.com".into()),
        vec![selector(1)],
    ));
    assert!(compile(&p, &f, CompileOptions::default()).is_err());
}

#[test]
fn unavailable_specific_exit_never_uses_default_or_local_fallback() {
    let mut f = facts(4);
    f.reachable.retain(|d| d.from != id(3) && d.to != id(3));
    let mut p = Policy::default_template();
    p.grants.push(internet());
    p.defaults.push(default_rule());
    p.exits.push(exit(
        "specific",
        ExitMatch::DomainSuffix("example.com".into()),
        vec![selector(3)],
    ));
    let c = compile_ok(&p, &f);
    assert_eq!(
        c.exit_for_selected(
            0,
            Some("example.com"),
            Some("192.0.2.1".parse().unwrap()),
            Some(1)
        )
        .unwrap(),
        ExitDecision::Unreachable
    );
}

#[test]
fn external_range_grants_direct_and_mixed_domain_ip_first_match() {
    let f = facts(3);
    let mut p = Policy::default_template();
    p.grants = vec![Grant {
        id: "ranges".into(),
        src: vec!["*".into()],
        dst: vec!["ip:192.168.1.0/24".into(), "domain:openai.com".into()],
        bidirectional: false,
    }];
    p.defaults.push(DefaultExitRule {
        locked: true,
        ..default_rule()
    });
    let c = compile_ok(&p, &f);
    assert!(c.mesh.is_empty());
    assert!(c.exact_authorization(1).grants.is_empty());
    assert!(!c.external_path_authorization(1).grants.is_empty());
    assert!(matches!(
        c.exit_for(0, None, Some("192.168.1.1".parse().unwrap()))
            .unwrap(),
        ExitDecision::Paths(_)
    ));
    assert_eq!(
        c.exit_for(0, None, Some("192.168.2.1".parse().unwrap()))
            .unwrap(),
        ExitDecision::Denied
    );
    assert!(matches!(
        c.exit_for(
            0,
            Some("api.openai.com"),
            Some("203.0.113.1".parse().unwrap())
        )
        .unwrap(),
        ExitDecision::Paths(_)
    ));
    assert_eq!(
        c.exit_for(0, None, Some("203.0.113.1".parse().unwrap()))
            .unwrap(),
        ExitDecision::Denied
    );
    assert_eq!(
        c.exit_for(
            0,
            Some("badopenai.com"),
            Some("203.0.113.1".parse().unwrap())
        )
        .unwrap(),
        ExitDecision::Denied
    );
    let mut direct = exit("direct", ExitMatch::Cidr("203.0.113.0/24".into()), vec![]);
    direct.target = ExitTarget::Direct;
    p.exits = vec![
        direct,
        exit(
            "domain",
            ExitMatch::DomainSuffix("openai.com".into()),
            vec![selector(2)],
        ),
    ];
    let c = compile_ok(&p, &f);
    assert_eq!(
        c.exit_for(0, Some("openai.com"), Some("203.0.113.1".parse().unwrap()))
            .unwrap(),
        ExitDecision::Local
    );
    p.exits.reverse();
    let c = compile_ok(&p, &f);
    let ExitDecision::Paths(ps) = c
        .exit_for(0, Some("openai.com"), Some("203.0.113.1".parse().unwrap()))
        .unwrap()
    else {
        panic!();
    };
    assert_eq!(c.paths[ps[0] as usize], vec![0, 2]);
}

#[test]
fn randomized_ipv4_ipv6_partition_equals_sequential_oracle_and_is_disjoint() {
    let mut rng = 13u128;
    let next = |state: &mut u128| {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *state
    };
    let f = facts(2);
    for round in 0..80 {
        let mut p = Policy::default_template();
        p.grants = vec![internet()];
        for i in 0..12 {
            let bits = next(&mut rng);
            let prefix = if i == 11 {
                0
            } else {
                (bits % if round % 2 == 0 { 33 } else { 129 }) as u8
            };
            let net = if round % 2 == 0 {
                ipnet::Ipv4Net::new(std::net::Ipv4Addr::from(bits as u32), prefix)
                    .unwrap()
                    .trunc()
                    .to_string()
            } else {
                ipnet::Ipv6Net::new(std::net::Ipv6Addr::from(bits), prefix)
                    .unwrap()
                    .trunc()
                    .to_string()
            };
            let mut r = exit(&format!("r{i}"), ExitMatch::Cidr(net), vec![selector(1)]);
            if i % 3 == 0 {
                r.src = selector(0);
            }
            if i % 2 == 0 {
                r.target = ExitTarget::Direct;
                r.candidates.clear();
            }
            p.exits.push(r);
        }
        let c = compile_ok(&p, &f);
        for src in 0..2 {
            let flat: &Vec<FlattenedPrefix> = &c.ip_tables[usize::from(c.source_ip_table[src])];
            let parsed: Vec<_> = flat
                .iter()
                .map(|p| (p.network.parse::<ipnet::IpNet>().unwrap(), p.rule))
                .collect();
            for (i, (a, _)) in parsed.iter().enumerate() {
                for (b, _) in parsed.iter().skip(i + 1) {
                    assert!(!a.contains(&b.network()) && !b.contains(&a.network()));
                }
            }
            for _ in 0..200 {
                let bits = next(&mut rng);
                let ip = if round % 2 == 0 {
                    std::net::IpAddr::V4(std::net::Ipv4Addr::from(bits as u32))
                } else {
                    std::net::IpAddr::V6(std::net::Ipv6Addr::from(bits))
                };
                let expected=p.exits.iter().enumerate().find(|(_,r)|super::matches(&r.src,&f.nodes[src])&&r.destinations.iter().any(|d|matches!(d,ExitMatch::Cidr(s) if s.parse::<ipnet::IpNet>().unwrap().contains(&ip)))).map(|(i,_)|i as u16);
                let actual = parsed
                    .iter()
                    .find(|(n, _)| n.contains(&ip))
                    .map(|(_, r)| *r);
                assert_eq!(actual, expected, "round {round}, src {src}, ip {ip}");
            }
        }
    }
}

#[test]
fn rule_set_expansion_reports_unsupported_predicates_and_lints_union_coverage() {
    let mut f = facts(2);
    let mut p = Policy::default_template();
    p.exits = vec![exit(
        "set",
        ExitMatch::RuleSet("cn".into()),
        vec![selector(1)],
    )];
    assert!(
        validate(&p, &f)
            .iter()
            .any(|d| d.message.contains("missing"))
    );
    f.rule_sets.insert(
        "cn".into(),
        ParsedRuleSet {
            entries: vec![
                RuleSetEntry::ExactDomain("example.com".into()),
                RuleSetEntry::Keyword("example".into()),
                RuleSetEntry::Regex(".*".into()),
            ],
        },
    );
    assert_eq!(validate(&p, &f).len(), 3);
    f.rule_sets.insert(
        "cn".into(),
        ParsedRuleSet {
            entries: vec![
                RuleSetEntry::Cidr("192.0.2.0/24".into()),
                RuleSetEntry::Cidr("198.51.100.0/24".into()),
            ],
        },
    );
    p.exits.insert(
        0,
        exit(
            "left",
            ExitMatch::Cidr("192.0.2.0/25".into()),
            vec![selector(1)],
        ),
    );
    p.exits.insert(
        1,
        exit(
            "right",
            ExitMatch::Cidr("192.0.2.128/25".into()),
            vec![selector(0)],
        ),
    );
    let c = compile_ok(&p, &f);
    let lint = c.lints.iter().find(|l| l.rule_id == "set").unwrap();
    assert_eq!(lint.shadowed_destinations, 1);
    assert_eq!(lint.total_destinations, 2);
    assert!(!lint.fully_shadowed);
    p.exits.insert(
        2,
        exit(
            "rest",
            ExitMatch::Cidr("198.51.100.0/24".into()),
            vec![selector(1)],
        ),
    );
    let c = compile_ok(&p, &f);
    assert!(
        c.lints
            .iter()
            .any(|l| l.rule_id == "set" && l.fully_shadowed)
    );
}

#[test]
fn documented_policy_deserializes_and_compiles_with_authoritative_facts() {
    let doc = include_str!("../../../docs/policy-v3.md");
    let json = doc
        .split("```jsonc\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let p: NetworkPolicy = serde_json::from_str(json).unwrap();
    let ids = [
        "ffb3c08f662759e7983e943aeaf33b0bd3c27f34c7a22cbf5d7c8a3f2e41adfe",
        "403d8c0cde9cfd8818bd3dafea3a4601d0539e80ad9488702b0aa6754b2013f8",
        "fafdea6da7d1f12de735632175c40a97012bd4abe36a167718aa5f92583a2426",
    ];
    let f = Snapshot {
        nodes: ids
            .iter()
            .map(|id| Node {
                id: (*id).into(),
                tags: BTreeSet::from(["tag:exit".into()]),
                listener: true,
                exit_enabled: true,
            })
            .collect(),
        tags: BTreeSet::from(["tag:exit".into(), "tag:managed".into()]),
        reachable: ids
            .iter()
            .flat_map(|a| {
                ids.iter().filter(move |b| a != *b).map(move |b| Dial {
                    from: (*a).into(),
                    to: (*b).into(),
                })
            })
            .collect(),
        rule_sets: BTreeMap::from([(
            "cn".into(),
            ParsedRuleSet {
                entries: vec![RuleSetEntry::Cidr("192.0.2.0/24".into())],
            },
        )]),
    };
    assert!(compile_network(&p, &f, CompileOptions::default()).is_ok());
}

#[test]
fn expansion_errors_preserve_original_jsonc_field_not_expanded_or_sorted_index() {
    let mut f = facts(2);
    f.rule_sets.insert(
        "set".into(),
        ParsedRuleSet {
            entries: vec![
                RuleSetEntry::Cidr("192.0.2.0/24".into()),
                RuleSetEntry::Cidr("198.51.100.0/24".into()),
            ],
        },
    );
    let mut p = Policy::default_template();
    let mut r = exit("mixed", ExitMatch::RuleSet("set".into()), vec![selector(1)]);
    r.destinations
        .push(ExitMatch::DomainSuffix("INVALID.Example.".into()));
    p.exits.push(r);
    assert!(
        validate(&p, &f)
            .iter()
            .any(|d| d.field == "exits[0].destinations[1]")
    );
    p.exits[0].destinations.pop();
    f.rule_sets
        .get_mut("set")
        .unwrap()
        .entries
        .push(RuleSetEntry::Cidr("::ffff:192.0.2.0/120".into()));
    let d = validate(&p, &f);
    assert!(
        d.iter()
            .any(|d| d.field == "exits[0].destinations[0]" && d.message.contains("entries[2]"))
    );
}

#[test]
#[ignore]
fn eight_thousand_cidr_timing() {
    let mut f = facts(1000);
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
    let mut p = Policy::default_template();
    p.grants = vec![internet()];
    p.grants.push(Grant {
        id: "lab-lan".into(),
        src: vec!["*".into()],
        dst: vec!["ip:10.0.0.0/8".into()],
        bidirectional: false,
    });
    let mut direct = exit("direct", ExitMatch::Cidr("10.0.0.0/16".into()), vec![]);
    direct.target = ExitTarget::Direct;
    p.exits = vec![
        direct,
        exit("set", ExitMatch::RuleSet("cn".into()), vec![selector(999)]),
    ];
    let start = std::time::Instant::now();
    let c = compile_ok(&p, &f);
    let compile_time = start.elapsed();
    assert_eq!(c.exit_rules[1].destinations.len(), 8000);
    assert_eq!(c.ip_tables.len(), 1);
    let lookups = 100_000;
    let start = std::time::Instant::now();
    for i in 0..lookups {
        let ip = std::net::Ipv4Addr::new(
            10,
            ((i * 7919) % 65536 / 256) as u8,
            ((i * 7919) % 256) as u8,
            1,
        );
        std::hint::black_box(c.exit_for(0, None, Some(ip.into())).unwrap());
    }
    let lookup_time = start.elapsed();
    eprintln!(
        "1000 nodes, 8000 rule-set CIDRs: compile={compile_time:?}, {lookups} lookups={lookup_time:?}, flattened prefixes={}, shared tables={}",
        c.ip_tables[0].len(),
        c.ip_tables.len()
    );
}
