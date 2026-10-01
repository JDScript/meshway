//! Shared matcher/table catalog and per-source authorization/path request planning.
use super::profile_validation::{Catalog, report};
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct PathRequest {
    pub src: usize,
    pub dst: usize,
    pub via: Vec<String>,
}
pub(super) type PathPlan = Vec<Vec<PathRequest>>;
pub(super) struct PlannedProfile {
    pub id: String,
    pub matcher: u16,
    pub routes: Vec<(u16, ExitTarget, PathPlan)>,
    pub fallback: Option<(bool, PathPlan)>,
    pub table: u32,
    pub lints: Vec<ExitLint>,
}
pub(super) struct PlannedSource {
    pub allowed: Vec<String>,
    pub default_profile: String,
    pub locked: bool,
    pub profiles: Vec<PlannedProfile>,
}
pub(super) struct Plan {
    pub sources: Vec<PlannedSource>,
    pub tables: Vec<Vec<FlattenedPrefix>>,
    pub requests: BTreeSet<PathRequest>,
}
fn all_external(base: &Compiled, src: usize) -> bool {
    let a = &base.external_grants[src];
    a.internet
        || ["0.0.0.0/0", "::/0"].iter().all(|s| {
            base.grant_index
                .covered(&ExitMatch::Cidr((*s).into()), &a.range_grants)
        })
}
fn requests(src: usize, candidates: &[String], via: &[String], nodes: &[&Node]) -> PathPlan {
    let mut seen = BTreeSet::new();
    let mut paths = Vec::new();
    for s in candidates {
        let mut group = Vec::new();
        for (dst, _) in nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.exit_enabled && matches(s, n))
        {
            if seen.insert(dst) {
                group.push(PathRequest {
                    src,
                    dst,
                    via: via.to_vec(),
                });
            }
        }
        paths.push(group);
    }
    paths
}
pub(super) fn matchers(catalog: &Catalog) -> Result<Vec<ProfileMatcher>, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let mut matchers = Vec::new();
    // One public matcher serves mesh-only and every global-exit built-in.
    let mut public_index = DestinationIndex::default();
    for (i, r) in catalog.public.iter().enumerate() {
        for d in &r.destinations {
            public_index.insert(d, i as u16);
        }
    }
    matchers.push(ProfileMatcher {
        rules: catalog.public.clone(),
        index: public_index,
    });
    for (id, (rules, _)) in &catalog.definitions {
        let mut combined = catalog.public.clone();
        combined.extend(rules.iter().cloned());
        let mut ids = BTreeSet::new();
        for r in &combined {
            if !ids.insert(&r.id) {
                report(
                    &mut diagnostics,
                    format!("profiles[{id}].rules"),
                    "Rule IDs must be unique within public + profile rules",
                );
            }
        }
        let mut index = DestinationIndex::default();
        for (i, r) in combined.iter().enumerate() {
            for d in &r.destinations {
                index.insert(d, i as u16);
            }
        }
        matchers.push(ProfileMatcher {
            rules: combined,
            index,
        });
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    Ok(matchers)
}
pub(super) fn plan(
    p: &NetworkPolicy,
    facts: &Snapshot,
    base: &Compiled,
    catalog: &Catalog,
    matchers: &[ProfileMatcher],
) -> Result<Plan, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let mut nodes: Vec<_> = facts.nodes.iter().collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    let matcher_ids: BTreeMap<_, _> = catalog
        .definitions
        .keys()
        .enumerate()
        .map(|(i, id)| (id.clone(), (i + 1) as u16))
        .collect();
    let mut planned = Vec::new();
    let mut all_requests = BTreeSet::new();
    let mut tables = Vec::new();
    let mut table_cache = BTreeMap::new();
    let mut lint_cache = BTreeMap::<(u16, Vec<u16>), Vec<ExitLint>>::new();
    for (src, node) in nodes.iter().enumerate() {
        let assignment = p
            .assignments
            .iter()
            .filter(|a| matches(&a.src, node))
            .max_by_key(|a| specificity(&a.src));
        let assignment_field = assignment
            .and_then(|a| p.assignments.iter().position(|b| std::ptr::eq(a, b)))
            .map_or_else(|| "assignments".into(), |i| format!("assignments[{i}]"));
        let refs = assignment.map_or_else(
            || vec![MESH_ONLY.to_owned(), "global:*".into(), AUTO.into()],
            |a| a.allowed.clone(),
        );
        let mut allowed = BTreeSet::new();
        for id in refs {
            if id == "global:*" {
                if all_external(base, src) {
                    for n in nodes.iter().filter(|n| n.exit_enabled) {
                        allowed.insert(format!("global:{}", n.id));
                    }
                }
            } else {
                allowed.insert(id);
            }
        }
        let default_profile = assignment
            .map_or(MESH_ONLY, |a| a.default_profile.as_str())
            .to_owned();
        if !allowed.contains(&default_profile) {
            report(
                &mut diagnostics,
                format!("{assignment_field}.default_profile"),
                "Default profile must be in source allowed set",
            );
        }
        if allowed.contains(AUTO) {
            for target in p
                .automatic
                .regions
                .values()
                .chain(std::iter::once(&p.automatic.fallback))
            {
                if !allowed.contains(target) {
                    report(
                        &mut diagnostics,
                        format!("{assignment_field}.allowed"),
                        format!(
                            "Automatic target {target} must also be explicitly allowed for this source"
                        ),
                    );
                }
            }
        }
        let mut source_profiles = Vec::new();
        let unrestricted = all_external(base, src);
        for id in allowed.iter().filter(|id| id.as_str() != AUTO) {
            let (matcher, fallback) = if id == MESH_ONLY {
                (0, ProfileFallback::None)
            } else if let Some(exit) = id.strip_prefix("global:") {
                if !nodes.iter().any(|n| n.id == exit && n.exit_enabled) || !all_external(base, src)
                {
                    report(
                        &mut diagnostics,
                        format!("{assignment_field}.allowed"),
                        format!("Global exit {exit} is disabled, missing, or not fully authorized"),
                    );
                    continue;
                }
                (
                    0,
                    ProfileFallback::Nodes {
                        candidates: vec![format!("node:{exit}")],
                        via: vec![],
                    },
                )
            } else if let Some((_, fallback)) = catalog.definitions.get(id) {
                (matcher_ids[id], fallback.clone())
            } else {
                report(
                    &mut diagnostics,
                    format!("{assignment_field}.allowed"),
                    format!("Unknown profile {id}"),
                );
                continue;
            };
            let m = &matchers[usize::from(matcher)];
            let mut routes = Vec::new();
            let mut enabled = BTreeSet::new();
            let key = (
                matcher,
                m.rules
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| matches(&r.src, node))
                    .map(|(i, _)| i as u16)
                    .collect::<Vec<_>>(),
            );
            let cached_lint = lint_cache.contains_key(&key);
            let mut lints = lint_cache.get(&key).cloned().unwrap_or_default();
            let grant = &base.external_grants[src];
            for (rule, r) in m
                .rules
                .iter()
                .enumerate()
                .filter(|(_, r)| matches(&r.src, node))
            {
                if r.target == ExitTarget::Nodes
                    && !unrestricted
                    && !r
                        .destinations
                        .iter()
                        .all(|d| base.grant_index.covered(d, &grant.range_grants))
                {
                    report(
                        &mut diagnostics,
                        format!("{assignment_field}.allowed"),
                        format!(
                            "Profile {id}, rule {} has destinations not fully covered by source grants",
                            r.id
                        ),
                    );
                }
                let shadowed = if cached_lint {
                    0
                } else {
                    r.destinations
                        .iter()
                        .filter(|d| m.index.covered(d, &enabled))
                        .count()
                };
                if shadowed > 0 {
                    lints.push(ExitLint {
                        rule_id: r.id.clone(),
                        shadowed_destinations: shadowed,
                        total_destinations: r.destinations.len(),
                        fully_shadowed: shadowed == r.destinations.len(),
                    });
                }
                enabled.insert(rule as u16);
                let paths = if r.target == ExitTarget::Nodes {
                    requests(src, &r.candidates, &r.via, &nodes)
                } else {
                    vec![]
                };
                all_requests.extend(paths.iter().flatten().cloned());
                routes.push((rule as u16, r.target, paths));
            }
            lint_cache
                .entry(key.clone())
                .or_insert_with(|| lints.clone());
            let table = if let Some(i) = table_cache.get(&key) {
                *i
            } else {
                let i = tables.len() as u32;
                tables.push(m.index.flatten(&enabled));
                table_cache.insert(key, i);
                i
            };
            let fallback = match fallback {
                ProfileFallback::None => None,
                ProfileFallback::Direct => Some((true, vec![])),
                ProfileFallback::Nodes { candidates, via } => {
                    if !unrestricted {
                        report(
                            &mut diagnostics,
                            format!("{assignment_field}.allowed"),
                            format!(
                                "Profile {id} final exit requires grants covering all non-Mesh destinations"
                            ),
                        );
                    }
                    let paths = requests(src, &candidates, &via, &nodes);
                    all_requests.extend(paths.iter().flatten().cloned());
                    Some((false, paths))
                }
            };
            source_profiles.push(PlannedProfile {
                id: id.clone(),
                matcher,
                routes,
                fallback,
                table,
                lints,
            });
        }
        planned.push(PlannedSource {
            allowed: allowed.into_iter().collect(),
            default_profile,
            locked: assignment.is_some_and(|a| a.locked),
            profiles: source_profiles,
        });
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    Ok(Plan {
        sources: planned,
        tables,
        requests: all_requests,
    })
}
