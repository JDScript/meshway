use super::*;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque},
    net::IpAddr,
};

#[derive(Clone, Copy, Debug)]
pub struct CompileOptions {
    /// One primary plus at most one logical backup.
    pub paths: usize,
}
impl Default for CompileOptions {
    fn default() -> Self {
        Self { paths: 2 }
    }
}
/// Node indexes refer to the sorted `nodes` table; path indexes to `paths`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Compiled {
    pub nodes: Vec<String>,
    pub paths: Vec<Vec<u16>>,
    pub path_scopes: Vec<PathScope>,
    pub mesh: Vec<MeshFlow>,
    pub connections: BTreeSet<(u16, u16)>,
    pub per_node: Vec<Vec<Forwarding>>,
    pub exits: Vec<Vec<ExitEntry>>,
    pub defaults: Vec<Vec<DefaultEntry>>,
    pub exit_rules: Vec<ExitRule>,
    pub exit_index: DestinationIndex,
    pub external_grants: Vec<ExternalAuthorization>,
    pub grant_index: DestinationIndex,
    pub ip_tables: Vec<Vec<super::index::FlattenedPrefix>>,
    pub source_ip_table: Vec<u16>,
    pub lints: Vec<ExitLint>,
    pub failures: Vec<FlowFailure>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PathScope {
    Mesh,
    External,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshFlow {
    pub src: u16,
    pub dst: u16,
    pub paths: Vec<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Forwarding {
    pub path: u32,
    pub previous: Option<u16>,
    pub next: Option<u16>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExitEntry {
    pub rule_id: String,
    pub rule: u16,
    pub target: ExitTarget,
    pub specificity: u8,
    pub authorized: bool,
    pub paths: Vec<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DefaultCandidate {
    pub node: u16,
    pub paths: Vec<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DefaultEntry {
    pub family: AddressFamily,
    pub locked: bool,
    pub authorized: bool,
    pub target: ExitTarget,
    pub candidates: Vec<DefaultCandidate>,
    /// Ordered primary/standby paths used when locked; selection uses one candidate.
    pub paths: Vec<u32>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExternalAuthorization {
    pub internet: bool,
    pub range_grants: BTreeSet<u16>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExitLint {
    pub rule_id: String,
    pub shadowed_destinations: usize,
    pub total_destinations: usize,
    pub fully_shadowed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowFailure {
    pub src: u16,
    pub destination: String,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExitDecision<'a> {
    Local,
    Denied,
    Unreachable,
    Paths(&'a [u32]),
}
impl Compiled {
    /// Caller supplies trusted domain context, never inferred from an arbitrary IP packet.
    pub fn exit_for(
        &self,
        src: usize,
        domain: Option<&str>,
        ip: Option<IpAddr>,
    ) -> Result<ExitDecision<'_>, &'static str> {
        self.exit_for_selected(src, domain, ip, None)
    }
    pub fn exit_for_selected(
        &self,
        src: usize,
        domain: Option<&str>,
        ip: Option<IpAddr>,
        selection: Option<u16>,
    ) -> Result<ExitDecision<'_>, &'static str> {
        let domain = domain
            .map(meshway_protocol::normalize_dns_name)
            .transpose()?;
        if matches!(ip,Some(IpAddr::V6(v)) if v.to_ipv4_mapped().is_some()) {
            return Err("Mapped IPv6 unsupported");
        }
        let entries = self.exits.get(src).ok_or("Unknown source index")?;
        let grants = &self.external_grants[src];
        let authorized = grants.internet
            || self
                .grant_index
                .matching(domain.as_deref(), ip)
                .iter()
                .any(|(_, g)| grants.range_grants.contains(g));
        let best = self
            .exit_index
            .matching(domain.as_deref(), ip)
            .into_iter()
            .filter_map(|((kind, len), rule)| {
                let i = entries.binary_search_by_key(&rule, |e| e.rule).ok()?;
                let e = &entries[i];
                Some(((kind, len, e.specificity), e))
            })
            .min_by_key(|(_, e)| e.rule);
        Ok(match best {
            None => {
                let family = match ip {
                    Some(IpAddr::V4(_)) => Some(AddressFamily::Ipv4),
                    Some(IpAddr::V6(_)) => Some(AddressFamily::Ipv6),
                    None => None,
                };
                let defaults = &self.defaults[src];
                let entry = family.and_then(|family| defaults.iter().find(|e| e.family == family));
                if family.is_none() && (selection.is_some() || defaults.iter().any(|e| e.locked)) {
                    return Err("Resolved IP family required for default exit");
                }
                match entry {
                    None if selection.is_some() => ExitDecision::Denied,
                    None => ExitDecision::Local,
                    Some(e) if !e.locked && selection.is_none() => ExitDecision::Local,
                    Some(e) if e.target == ExitTarget::Direct => ExitDecision::Local,
                    Some(_) if !authorized => ExitDecision::Denied,
                    Some(e) if e.locked => {
                        if e.paths.is_empty() {
                            ExitDecision::Unreachable
                        } else {
                            ExitDecision::Paths(&e.paths)
                        }
                    }
                    Some(e) => match e.candidates.iter().find(|c| Some(c.node) == selection) {
                        None => ExitDecision::Denied,
                        Some(c) if c.paths.is_empty() => ExitDecision::Unreachable,
                        Some(c) => ExitDecision::Paths(&c.paths),
                    },
                }
            }
            Some((_, e)) if e.target == ExitTarget::Direct => ExitDecision::Local,
            Some(_) if !authorized => ExitDecision::Denied,
            Some((_, e)) if e.paths.is_empty() => ExitDecision::Unreachable,
            Some((_, e)) => ExitDecision::Paths(&e.paths),
        })
    }
    /// Same complete, directional path semantics as MeshPathAuthorization. Legacy
    /// publication budgets and v2 routing intersection still require integration.
    pub fn exact_authorization(&self, node: usize) -> meshway_protocol::MeshPathAuthorization {
        self.exact_paths(node, PathScope::Mesh)
    }
    /// This projection must be installed together with external destination grants
    /// and route context. It does not authorize Mesh access to the exit node itself.
    pub fn external_path_authorization(
        &self,
        node: usize,
    ) -> meshway_protocol::MeshPathAuthorization {
        self.exact_paths(node, PathScope::External)
    }
    fn exact_paths(
        &self,
        node: usize,
        scope: PathScope,
    ) -> meshway_protocol::MeshPathAuthorization {
        let mut unique = BTreeSet::new();
        meshway_protocol::MeshPathAuthorization {
            grants: self.per_node[node]
                .iter()
                .filter(|f| self.path_scopes[f.path as usize] == scope)
                .filter(|f| self.paths[f.path as usize].len() > 1)
                .filter(|f| unique.insert(&self.paths[f.path as usize]))
                .map(|f| meshway_protocol::MeshAccessGrant {
                    id: format!("v3-{}", f.path),
                    path: self.paths[f.path as usize]
                        .iter()
                        .map(|i| self.nodes[usize::from(*i)].clone())
                        .collect(),
                })
                .collect(),
        }
    }
}

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
    let mut nodes: Vec<_> = facts.nodes.iter().collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    let n = nodes.len();
    let index: BTreeMap<_, _> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let mut dial = vec![vec![false; n]; n];
    for d in &facts.reachable {
        let a = index[d.from.as_str()];
        let b = index[d.to.as_str()];
        if nodes[b].listener {
            dial[a][b] = true;
        }
    }
    let mut graph = vec![Vec::new(); n];
    for a in 0..n {
        for b in a + 1..n {
            if !dial[a][b] && !dial[b][a] {
                continue;
            }
            let mut allow = false;
            let mut block = false;
            for r in &p.connections {
                if (matches(&r.a, nodes[a]) && matches(&r.b, nodes[b]))
                    || (matches(&r.b, nodes[a]) && matches(&r.a, nodes[b]))
                {
                    match r.action {
                        ConnectionAction::Allow => allow = true,
                        ConnectionAction::Block => block = true,
                    }
                }
            }
            if allow && !block {
                graph[a].push(b as u16);
                graph[b].push(a as u16);
            }
        }
    }
    let bridges = graph_bridges(&graph);
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
                .max_by_key(|r| (specificity(&r.src), specificity(&r.dst)));
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
fn find_paths(
    graph: &[Vec<u16>],
    bridges: &BTreeSet<(u16, u16)>,
    nodes: &[&Node],
    src: usize,
    dst: usize,
    via: &[String],
    count: usize,
) -> Result<Vec<Vec<u16>>, String> {
    let primary = search(graph, nodes, src, dst, via, None)?;
    let Some(primary) = primary else {
        return Err("Unreachable mandatory path within eight-node budget".into());
    };
    let mut paths = vec![primary.clone()];
    if count > 1 {
        let mut alternatives = Vec::new();
        for e in primary.windows(2) {
            if bridges.contains(&(e[0].min(e[1]), e[0].max(e[1]))) {
                continue;
            }
            if let Some(p) = search(graph, nodes, src, dst, via, Some((e[0], e[1])))? {
                alternatives.push(p);
            }
        }
        alternatives.sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));
        if let Some(p) = alternatives.into_iter().next() {
            paths.push(p);
        }
    }
    Ok(paths)
}
fn graph_bridges(graph: &[Vec<u16>]) -> BTreeSet<(u16, u16)> {
    struct Search<'a> {
        graph: &'a [Vec<u16>],
        time: usize,
        entered: Vec<usize>,
        low: Vec<usize>,
        bridges: BTreeSet<(u16, u16)>,
    }
    impl Search<'_> {
        fn visit(&mut self, a: usize, parent: usize) {
            self.time += 1;
            self.entered[a] = self.time;
            self.low[a] = self.time;
            for &b in &self.graph[a] {
                let b = usize::from(b);
                if b == parent {
                    continue;
                }
                if self.entered[b] == 0 {
                    self.visit(b, a);
                    self.low[a] = self.low[a].min(self.low[b]);
                    if self.low[b] > self.entered[a] {
                        self.bridges.insert((a.min(b) as u16, a.max(b) as u16));
                    }
                } else {
                    self.low[a] = self.low[a].min(self.entered[b]);
                }
            }
        }
    }
    let mut search = Search {
        graph,
        time: 0,
        entered: vec![0; graph.len()],
        low: vec![0; graph.len()],
        bridges: BTreeSet::new(),
    };
    for a in 0..graph.len() {
        if search.entered[a] == 0 {
            search.visit(a, usize::MAX);
        }
    }
    search.bridges
}

pub(crate) fn append_external_requests(
    p: &Policy,
    facts: &Snapshot,
    options: CompileOptions,
    out: &mut Compiled,
    requests: BTreeSet<super::profiles::PathRequest>,
) -> Result<BTreeMap<super::profiles::PathRequest, Vec<u32>>, Vec<Diagnostic>> {
    if requests.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut nodes: Vec<_> = facts.nodes.iter().collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    let n = nodes.len();
    let index: BTreeMap<_, _> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let mut dial = vec![vec![false; n]; n];
    for d in &facts.reachable {
        let a = index[d.from.as_str()];
        let b = index[d.to.as_str()];
        if nodes[b].listener {
            dial[a][b] = true;
        }
    }
    let mut graph = vec![Vec::new(); n];
    for a in 0..n {
        for b in a + 1..n {
            if !dial[a][b] && !dial[b][a] {
                continue;
            }
            let matching = |r: &ConnectionRule| {
                (matches(&r.a, nodes[a]) && matches(&r.b, nodes[b]))
                    || (matches(&r.b, nodes[a]) && matches(&r.a, nodes[b]))
            };
            if p.connections
                .iter()
                .any(|r| r.action == ConnectionAction::Allow && matching(r))
                && !p
                    .connections
                    .iter()
                    .any(|r| r.action == ConnectionAction::Block && matching(r))
            {
                graph[a].push(b as u16);
                graph[b].push(a as u16);
            }
        }
    }
    let bridges = graph_bridges(&graph);
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
fn search(
    graph: &[Vec<u16>],
    nodes: &[&Node],
    src: usize,
    dst: usize,
    via: &[String],
    banned: Option<(u16, u16)>,
) -> Result<Option<Vec<u16>>, String> {
    if src == dst {
        return Ok(via.is_empty().then_some(vec![src as u16]));
    }
    if via.is_empty() {
        let edge_allowed = |a: usize, b: usize| {
            !banned.is_some_and(|(x, y)| {
                (x == a as u16 && y == b as u16) || (y == a as u16 && x == b as u16)
            })
        };
        if graph[src].binary_search(&(dst as u16)).is_ok() && edge_allowed(src, dst) {
            return Ok(Some(vec![src as u16, dst as u16]));
        }
        for &mid in &graph[src] {
            let m = usize::from(mid);
            if edge_allowed(src, m)
                && edge_allowed(m, dst)
                && graph[m].binary_search(&(dst as u16)).is_ok()
            {
                return Ok(Some(vec![src as u16, mid, dst as u16]));
            }
        }
        let mut previous = vec![usize::MAX; nodes.len()];
        previous[src] = src;
        let mut queue = VecDeque::from([src]);
        while let Some(a) = queue.pop_front() {
            for &b in &graph[a] {
                let b = usize::from(b);
                if banned.is_some_and(|(x, y)| {
                    (x == a as u16 && y == b as u16) || (y == a as u16 && x == b as u16)
                }) || previous[b] != usize::MAX
                {
                    continue;
                }
                previous[b] = a;
                if b == dst {
                    let mut p = vec![dst as u16];
                    let mut i = dst;
                    while i != src {
                        i = previous[i];
                        p.push(i as u16);
                    }
                    p.reverse();
                    return Ok((p.len() <= MAX_PATH_NODES).then_some(p));
                }
                queue.push_back(b);
            }
        }
        return Ok(None);
    }
    // Mandatory ordered hops plus simple paths is constrained search; never prune
    // by (node,stage) alone, which would lose distinct visited-node histories.
    let remaining = remaining_distances(graph, nodes, src, dst, via, banned);
    if remaining[0][src] == usize::MAX {
        return Ok(None);
    }
    let mut queue = BinaryHeap::from([Reverse((remaining[0][src] + 1, vec![src as u16], 0usize))]);
    let mut states = 0;
    while let Some(Reverse((_, path, stage))) = queue.pop() {
        states += 1;
        if states > MAX_SEARCH_STATES {
            return Err("Search budget exhausted; no bypass installed".into());
        }
        let a = usize::from(*path.last().expect("nonempty"));
        if a == dst {
            if stage == via.len() {
                return Ok(Some(path));
            }
            continue;
        }
        if path.len() == MAX_PATH_NODES {
            continue;
        }
        for &b in &graph[a] {
            if path.contains(&b)
                || banned
                    .is_some_and(|(x, y)| (x == a as u16 && y == b) || (y == a as u16 && x == b))
            {
                continue;
            }
            let bi = usize::from(b);
            let advance = stage < via.len() && matches(&via[stage], nodes[bi]) && bi != dst;
            // A concrete later mandatory hop cannot be consumed early.
            if !advance
                && via[stage..]
                    .iter()
                    .any(|s| s.starts_with("node:") && matches(s, nodes[bi]))
            {
                continue;
            }
            let mut p = path.clone();
            p.push(b);
            let next_stage = stage + usize::from(advance);
            let lower_bound = p.len().saturating_add(remaining[next_stage][bi]);
            if lower_bound <= MAX_PATH_NODES {
                queue.push(Reverse((lower_bound, p, next_stage)));
            }
            if queue.len() > MAX_SEARCH_STATES {
                return Err("Search budget exhausted; no bypass installed".into());
            }
        }
    }
    Ok(None)
}

/// Relaxed ordered walk distances are an admissible A* heuristic for simple paths.
fn remaining_distances(
    graph: &[Vec<u16>],
    nodes: &[&Node],
    src: usize,
    dst: usize,
    via: &[String],
    banned: Option<(u16, u16)>,
) -> Vec<Vec<usize>> {
    let mut distances = vec![vec![usize::MAX; nodes.len()]; via.len() + 1];
    for stage in (0..=via.len()).rev() {
        let mut heap = BinaryHeap::new();
        if stage == via.len() {
            distances[stage][dst] = 0;
            heap.push(Reverse((0usize, dst)));
        } else {
            for (i, node) in nodes.iter().enumerate() {
                if i != src
                    && i != dst
                    && matches(&via[stage], node)
                    && distances[stage + 1][i] != usize::MAX
                {
                    let cost = distances[stage + 1][i];
                    distances[stage][i] = cost;
                    heap.push(Reverse((cost, i)));
                }
            }
        }
        while let Some(Reverse((cost, a))) = heap.pop() {
            if distances[stage][a] != cost {
                continue;
            }
            for &b in &graph[a] {
                if banned
                    .is_some_and(|(x, y)| (x == a as u16 && y == b) || (y == a as u16 && x == b))
                {
                    continue;
                }
                let b = usize::from(b);
                let next = cost + 1;
                if next < distances[stage][b] {
                    distances[stage][b] = next;
                    heap.push(Reverse((next, b)));
                }
            }
        }
    }
    distances
}
