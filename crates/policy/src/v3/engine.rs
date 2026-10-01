//! Kernel compilation orchestration and shared path-pool registration.
use super::*;
use super::{graph::Graph, path_search::find_paths, profile_plan::PathRequest};

pub fn compile(
    p: &Policy,
    facts: &Snapshot,
    options: CompileOptions,
) -> Result<Compiled, Vec<Diagnostic>> {
    let expanded = expand_rule_sets(p, &facts.rule_sets)?;
    let p = &expanded;
    let mut diagnostics = super::validation::validate_expanded(p, facts);
    if !(1..=2).contains(&options.paths) {
        diagnostics.push(Diagnostic {
            field: "options.paths".into(),
            message: "Require 1 or 2 paths".into(),
        });
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    let topology = Graph::new(p, facts);
    let Graph {
        nodes,
        dial,
        edges: graph,
        bridges,
    } = topology;
    let n = nodes.len();
    let mut out = Compiled {
        nodes: nodes.iter().map(|n| n.id.clone()).collect(),
        paths: vec![],
        path_scopes: vec![],
        mesh: vec![],
        connections: BTreeSet::new(),
        per_node: vec![vec![]; n],
        exits: vec![vec![]; n],
        defaults: vec![vec![]; n],
        exit_rules: vec![],
        exit_index: DestinationIndex::default(),
        external_grants: vec![ExternalAuthorization::default(); n],
        grant_index: DestinationIndex::default(),
        ip_tables: vec![],
        source_ip_table: vec![],
        lints: vec![],
        failures: vec![],
    };
    // Rule sorting is only a stable duplicate-action tie break, never semantic precedence.
    let mut mesh: Vec<_> = p.mesh.iter().collect();
    mesh.sort_by(|a, b| a.id.cmp(&b.id));
    let exits: Vec<_> = p.exits.iter().collect();
    let mut grants: Vec<_> = p.grants.iter().collect();
    grants.sort_by(|a, b| a.id.cmp(&b.id));
    for (i, g) in grants.iter().enumerate() {
        let mut ranges = false;
        for s in &g.dst {
            if let Some(d) = super::index::external_selector(s) {
                out.grant_index.insert(&d, i as u16);
                ranges = true;
            }
        }
        for (a, node) in nodes.iter().enumerate() {
            if g.src.iter().any(|s| matches(s, node)) {
                out.external_grants[a].internet |= g.dst.iter().any(|s| s == "autogroup:internet");
                if ranges {
                    out.external_grants[a].range_grants.insert(i as u16);
                }
            }
        }
    }
    for (i, r) in exits.iter().enumerate() {
        let mut rule = (*r).clone();
        rule.destinations.sort();
        rule.destinations.dedup();
        for d in &rule.destinations {
            out.exit_index.insert(d, i as u16);
        }
        out.exit_rules.push(rule);
    }
    for a in 0..n {
        for b in 0..n {
            if a == b || !allowed(p, nodes[a], Some(nodes[b])) {
                continue;
            }
            let rule = mesh
                .iter()
                .filter(|r| matches(&r.src, nodes[a]) && matches(&r.dst, nodes[b]))
                .max_by_key(|r| mesh_specificity(r));
            let via = rule.map_or(&[][..], |r| r.via.as_slice());
            let paths = find_paths(&graph, &bridges, &nodes, a, b, via, options.paths);
            let ids = record(&mut out, &dial, a, &nodes[b].id, paths, PathScope::Mesh)?;
            out.mesh.push(MeshFlow {
                src: a as u16,
                dst: b as u16,
                paths: ids,
            });
        }
    }
    for (a, node) in nodes.iter().enumerate() {
        for (rule, r) in exits.iter().enumerate() {
            if !matches(&r.src, node) {
                continue;
            }
            let authorized =
                out.external_grants[a].internet || !out.external_grants[a].range_grants.is_empty();
            let mut paths = Vec::new();
            if authorized && r.target == ExitTarget::Nodes {
                for selector in &r.candidates {
                    let mut group = Vec::new();
                    for (b, node) in nodes
                        .iter()
                        .enumerate()
                        .filter(|(_, n)| n.exit_enabled && matches(selector, n))
                    {
                        match find_paths(&graph, &bridges, &nodes, a, b, &r.via, options.paths) {
                            Ok(mut ps) => group.append(&mut ps),
                            Err(reason) => out.failures.push(FlowFailure {
                                src: a as u16,
                                destination: format!("{}:{}", r.id, node.id),
                                reason,
                            }),
                        }
                    }
                    group.sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));
                    group.dedup();
                    for path in group {
                        if !paths.contains(&path) {
                            paths.push(path);
                        }
                    }
                }
            }
            let ids = record(&mut out, &dial, a, &r.id, Ok(paths), PathScope::External)?;
            out.exits[a].push(ExitEntry {
                rule_id: r.id.clone(),
                rule: rule as u16,
                target: r.target,
                specificity: specificity(&r.src),
                authorized,
                paths: ids,
            });
        }
    }
    let defaults = default_rules(p);
    for (a, node) in nodes.iter().enumerate() {
        for family in [AddressFamily::Ipv4, AddressFamily::Ipv6] {
            let Some(r) = defaults
                .iter()
                .filter(|r| r.families.contains(&family) && matches(&r.src, node))
                .max_by_key(|r| specificity(&r.src))
            else {
                continue;
            };
            let authorized =
                out.external_grants[a].internet || !out.external_grants[a].range_grants.is_empty();
            let mut candidates = Vec::new();
            let mut seen = BTreeSet::new();
            for selector in &r.candidates {
                let mut group = Vec::new();
                for (b, node) in nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| n.exit_enabled && matches(selector, n))
                {
                    if !seen.insert(b) {
                        continue;
                    }
                    let paths = if authorized {
                        find_paths(&graph, &bridges, &nodes, a, b, &r.via, options.paths)
                    } else {
                        Ok(vec![])
                    };
                    let ids = record(
                        &mut out,
                        &dial,
                        a,
                        &format!("{}:{}", r.id, node.id),
                        paths,
                        PathScope::External,
                    )?;
                    group.push(DefaultCandidate {
                        node: b as u16,
                        paths: ids,
                    });
                }
                group.sort_by(|a, b| {
                    let rank = |c: &DefaultCandidate| {
                        c.paths
                            .first()
                            .map_or(usize::MAX, |p| out.paths[*p as usize].len())
                    };
                    rank(a).cmp(&rank(b)).then(a.node.cmp(&b.node))
                });
                candidates.extend(group);
            }
            let paths = candidates
                .iter()
                .flat_map(|c| c.paths.iter().copied())
                .collect();
            out.defaults[a].push(DefaultEntry {
                family,
                locked: r.locked,
                authorized,
                target: r.target,
                candidates,
                paths,
            });
        }
    }
    let mut tables = BTreeMap::new();
    for entries in &out.exits {
        let enabled: BTreeSet<_> = entries.iter().map(|e| e.rule).collect();
        let key: Vec<_> = enabled.iter().copied().collect();
        let table = if let Some(table) = tables.get(&key) {
            *table
        } else {
            let i = out.ip_tables.len() as u16;
            out.ip_tables.push(out.exit_index.flatten(&enabled));
            tables.insert(key, i);
            i
        };
        out.source_ip_table.push(table);
    }
    for (i, r) in exits.iter().enumerate() {
        let contexts: BTreeSet<BTreeSet<u16>> = nodes
            .iter()
            .filter(|n| matches(&r.src, n))
            .map(|node| {
                exits[..i]
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| matches(&r.src, node))
                    .map(|(i, _)| i as u16)
                    .collect()
            })
            .collect();
        let shadowed = if contexts.is_empty() {
            0
        } else {
            r.destinations
                .iter()
                .filter(|d| {
                    contexts
                        .iter()
                        .all(|enabled| out.exit_index.covered(d, enabled))
                })
                .count()
        };
        if shadowed > 0 {
            out.lints.push(ExitLint {
                rule_id: r.id.clone(),
                shadowed_destinations: shadowed,
                total_destinations: r.destinations.len(),
                fully_shadowed: shadowed == r.destinations.len(),
            });
        }
    }
    out.failures.sort_by(|a, b| {
        a.src
            .cmp(&b.src)
            .then(a.destination.cmp(&b.destination))
            .then(a.reason.cmp(&b.reason))
    });
    Ok(out)
}

fn record(
    out: &mut Compiled,
    dial: &[Vec<bool>],
    src: usize,
    dst: &str,
    paths: Result<Vec<Vec<u16>>, String>,
    scope: PathScope,
) -> Result<Vec<u32>, Vec<Diagnostic>> {
    let paths = match paths {
        Ok(p) => p,
        Err(reason) => {
            out.failures.push(FlowFailure {
                src: src as u16,
                destination: dst.into(),
                reason,
            });
            return Ok(vec![]);
        }
    };
    if out.paths.len() + paths.len() > MAX_COMPILED_PATHS {
        return Err(vec![Diagnostic {
            field: "compiled.paths".into(),
            message: "Compiled path budget exceeds 4200000; no partial output returned".into(),
        }]);
    }
    let mut ids = Vec::new();
    for path in paths {
        let id = out.paths.len() as u32;
        for (i, node) in path.iter().enumerate() {
            out.per_node[usize::from(*node)].push(Forwarding {
                path: id,
                previous: i.checked_sub(1).map(|i| path[i]),
                next: path.get(i + 1).copied(),
            });
        }
        for edge in path.windows(2) {
            let a = usize::from(edge[0]);
            let b = usize::from(edge[1]);
            if dial[a][b] {
                out.connections.insert((edge[0], edge[1]));
            }
            if dial[b][a] {
                out.connections.insert((edge[1], edge[0]));
            }
        }
        out.paths.push(path);
        out.path_scopes.push(scope);
        ids.push(id);
    }
    Ok(ids)
}

pub(super) fn append_external_requests(
    p: &Policy,
    facts: &Snapshot,
    options: CompileOptions,
    out: &mut Compiled,
    requests: BTreeSet<PathRequest>,
) -> Result<BTreeMap<PathRequest, Vec<u32>>, Vec<Diagnostic>> {
    if requests.is_empty() {
        return Ok(BTreeMap::new());
    }
    let Graph {
        nodes,
        dial,
        edges: graph,
        bridges,
    } = Graph::new(p, facts);
    let mut ids = BTreeMap::new();
    for request in requests {
        let paths = find_paths(
            &graph,
            &bridges,
            &nodes,
            request.src,
            request.dst,
            &request.via,
            options.paths,
        );
        let paths = record(
            out,
            &dial,
            request.src,
            &format!("profile-exit:{}", nodes[request.dst].id),
            paths,
            PathScope::External,
        )?;
        ids.insert(request, paths);
    }
    Ok(ids)
}
