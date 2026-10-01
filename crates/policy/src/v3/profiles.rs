//! Product-level routing schemes, built on one network-global three-layer plan.
use super::*;
pub const MESH_ONLY: &str = "mesh-only";
pub const AUTO: &str = "auto";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NetworkPolicy {
    pub connections: Vec<ConnectionRule>,
    pub grants: Vec<Grant>,
    pub mesh: Vec<MeshRule>,
    pub public_rules: Vec<ExitRule>,
    pub profiles: Vec<RoutingProfile>,
    #[serde(default)]
    pub automatic: AutomaticRouting,
    pub assignments: Vec<ProfileAssignment>,
}
impl NetworkPolicy {
    pub fn default_template() -> Self {
        let p = Policy::default_template();
        Self {
            connections: p.connections,
            grants: p.grants,
            mesh: p.mesh,
            ..Self::default()
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RoutingProfile {
    pub id: String,
    pub rules: Vec<ExitRule>,
    pub fallback: ProfileFallback,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProfileFallback {
    None,
    Direct,
    Nodes {
        candidates: Vec<String>,
        via: Vec<String>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AutomaticRouting {
    pub regions: BTreeMap<String, String>,
    pub fallback: String,
}
impl Default for AutomaticRouting {
    fn default() -> Self {
        Self {
            regions: BTreeMap::new(),
            fallback: MESH_ONLY.into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileAssignment {
    pub src: String,
    pub allowed: Vec<String>,
    pub default_profile: String,
    pub locked: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileMatcher {
    pub rules: Vec<ExitRule>,
    pub index: DestinationIndex,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRoute {
    pub rule: u16,
    pub target: ExitTarget,
    pub paths: Vec<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(
    tag = "kind",
    content = "paths",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CompiledFinal {
    None,
    Direct,
    Nodes(Vec<u32>),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CompiledProfile {
    pub id: String,
    pub matcher: u16,
    pub routes: Vec<ProfileRoute>,
    pub ip_table: u32,
    pub fallback: CompiledFinal,
    pub lints: Vec<ExitLint>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceProfiles {
    pub allowed: Vec<String>,
    pub default_profile: String,
    pub locked: bool,
    pub profiles: Vec<CompiledProfile>,
    pub automatic: AutomaticRouting,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NetworkCompilation {
    pub base: Compiled,
    pub matchers: Vec<ProfileMatcher>,
    pub ip_tables: Vec<Vec<FlattenedPrefix>>,
    pub sources: Vec<SourceProfiles>,
}
/// Only the caller's STUN-observed public-IP geolocation, never controller peer IP.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StunLocation {
    pub region: Option<String>,
}
impl NetworkCompilation {
    pub fn select_profile(
        &self,
        src: usize,
        requested: Option<&str>,
        location: &StunLocation,
    ) -> Result<&CompiledProfile, &'static str> {
        let source = self.sources.get(src).ok_or("Unknown source")?;
        let selected = if source.locked {
            source.default_profile.as_str()
        } else {
            requested.unwrap_or(&source.default_profile)
        };
        if !source.allowed.iter().any(|p| p == selected) {
            return Err("Profile not allowed for source");
        }
        let selected = if selected == AUTO {
            location
                .region
                .as_ref()
                .and_then(|s| source.automatic.regions.get(&s.to_ascii_uppercase()))
                .unwrap_or(&source.automatic.fallback)
                .as_str()
        } else {
            selected
        };
        source
            .profiles
            .iter()
            .find(|p| p.id == selected)
            .ok_or("Automatic target not allowed for source")
    }
    pub fn exit_for_profile(
        &self,
        src: usize,
        requested: Option<&str>,
        location: &StunLocation,
        domain: Option<&str>,
        ip: Option<std::net::IpAddr>,
    ) -> Result<ExitDecision<'_>, &'static str> {
        let profile = self.select_profile(src, requested, location)?;
        let domain = domain
            .map(meshway_protocol::normalize_dns_name)
            .transpose()?;
        if matches!(ip,Some(std::net::IpAddr::V6(v)) if v.to_ipv4_mapped().is_some()) {
            return Err("Mapped IPv6 unsupported");
        }
        let matcher = &self.matchers[usize::from(profile.matcher)];
        let rule = matcher
            .index
            .matching(domain.as_deref(), ip)
            .into_iter()
            .filter_map(|(_, rule)| {
                profile
                    .routes
                    .binary_search_by_key(&rule, |r| r.rule)
                    .ok()
                    .map(|i| &profile.routes[i])
            })
            .min_by_key(|r| r.rule);
        let auth = &self.base.external_grants[src];
        let authorized = auth.internet
            || self
                .base
                .grant_index
                .matching(domain.as_deref(), ip)
                .iter()
                .any(|(_, r)| auth.range_grants.contains(r));
        match rule {
            Some(r) if r.target == ExitTarget::Direct => Ok(ExitDecision::Local),
            Some(_) if !authorized => Ok(ExitDecision::Denied),
            Some(r) if r.paths.is_empty() => Ok(ExitDecision::Unreachable),
            Some(r) => Ok(ExitDecision::Paths(&r.paths)),
            None => match &profile.fallback {
                CompiledFinal::None | CompiledFinal::Direct => Ok(ExitDecision::Local),
                CompiledFinal::Nodes(_) if ip.is_none() => {
                    Err("Resolved IP required for profile final")
                }
                CompiledFinal::Nodes(_) if !authorized => Ok(ExitDecision::Denied),
                CompiledFinal::Nodes(paths) if paths.is_empty() => Ok(ExitDecision::Unreachable),
                CompiledFinal::Nodes(paths) => Ok(ExitDecision::Paths(paths)),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PathRequest {
    pub src: usize,
    pub dst: usize,
    pub via: Vec<String>,
}
type PathPlan = Vec<Vec<PathRequest>>;
struct PlannedProfile {
    id: String,
    matcher: u16,
    routes: Vec<(u16, ExitTarget, PathPlan)>,
    fallback: Option<(bool, PathPlan)>,
    table: u32,
    lints: Vec<ExitLint>,
}
struct PlannedSource {
    allowed: Vec<String>,
    default_profile: String,
    locked: bool,
    profiles: Vec<PlannedProfile>,
}
fn report(d: &mut Vec<Diagnostic>, field: impl Into<String>, message: impl Into<String>) {
    if d.len() < 128 {
        d.push(Diagnostic {
            field: field.into(),
            message: message.into(),
        });
    }
}
fn field_at(field: &str, prefix: &str) -> String {
    field
        .strip_prefix("exits")
        .map_or_else(|| prefix.into(), |rest| format!("{prefix}{rest}"))
}
fn valid_reference(id: &str, wildcard: bool) -> bool {
    (wildcard && id == "global:*")
        || id
            .strip_prefix("global:")
            .is_some_and(super::validation::valid_id)
        || (!id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)))
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
pub fn compile_network(
    p: &NetworkPolicy,
    facts: &Snapshot,
    options: CompileOptions,
) -> Result<NetworkCompilation, Vec<Diagnostic>> {
    let intent = Policy {
        connections: p.connections.clone(),
        grants: p.grants.clone(),
        mesh: p.mesh.clone(),
        ..Policy::default()
    };
    let mut base = compile(&intent, facts, options)?;
    let mut diagnostics = Vec::new();
    if p.profiles.len() > 64
        || p.assignments.len() > 128
        || serde_json::to_vec(p).map_or(true, |v| v.len() > 2 * 1024 * 1024)
    {
        return Err(vec![Diagnostic {
            field: "profiles".into(),
            message: "At most 64 custom profiles, 128 assignments and 2 MiB network intent".into(),
        }]);
    }
    let mut nodes: Vec<_> = facts.nodes.iter().collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    let mut definitions = BTreeMap::new();
    let public = expand_rule_sets(
        &Policy {
            exits: p.public_rules.clone(),
            ..Policy::default()
        },
        &facts.rule_sets,
    )?;
    let errors = validate(
        &Policy {
            exits: public.exits.clone(),
            ..Policy::default()
        },
        facts,
    );
    for e in errors {
        report(
            &mut diagnostics,
            field_at(&e.field, "public_rules"),
            e.message,
        );
    }
    for (i, profile) in p.profiles.iter().enumerate() {
        if profile.id.is_empty()
            || profile.id.len() > 64
            || !profile
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || [MESH_ONLY, AUTO].contains(&profile.id.as_str())
            || definitions.contains_key(&profile.id)
        {
            report(
                &mut diagnostics,
                format!("profiles[{i}].id"),
                "Unique non-reserved ASCII profile ID required",
            );
        }
        let policy = Policy {
            exits: profile.rules.clone(),
            ..Policy::default()
        };
        let expanded = match expand_rule_sets(&policy, &facts.rule_sets) {
            Ok(p) => p,
            Err(d) => {
                for e in d {
                    report(
                        &mut diagnostics,
                        field_at(&e.field, &format!("profiles[{i}].rules")),
                        e.message,
                    );
                }
                continue;
            }
        };
        for e in validate(&expanded, facts) {
            report(
                &mut diagnostics,
                field_at(&e.field, &format!("profiles[{i}].rules")),
                e.message,
            );
        }
        if let ProfileFallback::Nodes { candidates, via } = &profile.fallback {
            let fallback = Policy {
                exits: vec![ExitRule {
                    id: "final".into(),
                    src: "*".into(),
                    destinations: vec![ExitMatch::Cidr("0.0.0.0/0".into())],
                    target: ExitTarget::Nodes,
                    candidates: candidates.clone(),
                    via: via.clone(),
                }],
                ..Policy::default()
            };
            for e in validate(&fallback, facts) {
                report(
                    &mut diagnostics,
                    format!(
                        "profiles[{i}].fallback.{}",
                        e.field.strip_prefix("exits[0].").unwrap_or(&e.field)
                    ),
                    e.message,
                );
            }
        }
        definitions.insert(
            profile.id.clone(),
            (expanded.exits, profile.fallback.clone()),
        );
    }
    let public_count = public
        .exits
        .iter()
        .map(|r| r.destinations.len())
        .sum::<usize>();
    if public_count
        + definitions
            .values()
            .map(|(rules, _)| rules.iter().map(|r| r.destinations.len()).sum::<usize>())
            .sum::<usize>()
        > 65536
    {
        report(
            &mut diagnostics,
            "profiles",
            "At most 65536 total expanded profile/public destinations",
        );
    }
    for (id, (rules, _)) in &definitions {
        if public_count + rules.iter().map(|r| r.destinations.len()).sum::<usize>()
            > MAX_DESTINATIONS
        {
            report(
                &mut diagnostics,
                format!("profiles[{id}].rules"),
                "Combined public + profile destinations exceed 16384",
            );
        }
    }
    for (region, target) in &p.automatic.regions {
        if !valid_reference(target, false) {
            report(
                &mut diagnostics,
                "automatic.regions",
                "Invalid profile reference",
            );
        }
        if region.len() != 2 || !region.bytes().all(|b| b.is_ascii_uppercase()) {
            report(
                &mut diagnostics,
                format!("automatic.regions[{region}]"),
                "STUN location region requires two uppercase ISO country/region letters",
            );
        }
        if target == AUTO {
            report(
                &mut diagnostics,
                format!("automatic.regions[{region}]"),
                "Automatic routing cannot reference automatic routing; cycles forbidden",
            );
        }
    }
    if p.automatic.fallback == AUTO {
        report(
            &mut diagnostics,
            "automatic.fallback",
            "Automatic routing cannot reference automatic routing; cycles forbidden",
        );
    }
    if p.automatic.regions.len() > 256 || !valid_reference(&p.automatic.fallback, false) {
        report(
            &mut diagnostics,
            "automatic",
            "At most 256 locations and a valid fallback profile",
        );
    }
    for target in p
        .automatic
        .regions
        .values()
        .chain(std::iter::once(&p.automatic.fallback))
    {
        if target != MESH_ONLY
            && target != AUTO
            && !definitions.contains_key(target)
            && !target
                .strip_prefix("global:")
                .is_some_and(|id| nodes.iter().any(|n| n.id == id && n.exit_enabled))
        {
            report(
                &mut diagnostics,
                "automatic",
                format!("Unknown or disabled automatic target {target}"),
            );
        }
    }
    for (i, a) in p.assignments.iter().enumerate() {
        if !valid_reference(&a.default_profile, false)
            || a.allowed.iter().any(|id| !valid_reference(id, true))
        {
            report(
                &mut diagnostics,
                format!("assignments[{i}]"),
                "Invalid profile reference syntax",
            );
        }
        if a.allowed.is_empty() || a.allowed.len() > 128 {
            report(
                &mut diagnostics,
                format!("assignments[{i}].allowed"),
                "Require 1..128 profile references",
            );
        }
        if a.src != "*"
            && !a
                .src
                .strip_prefix("node:")
                .is_some_and(|id| nodes.iter().any(|n| n.id == id))
            && !facts.tags.contains(&a.src)
        {
            report(
                &mut diagnostics,
                format!("assignments[{i}].src"),
                "Unsupported user/group or unknown node/tag selector",
            );
        }
        for (j, b) in p.assignments.iter().enumerate().skip(i + 1) {
            let overlap = a.src == b.src
                || (a.src.starts_with("tag:") && b.src.starts_with("tag:"))
                || nodes
                    .iter()
                    .any(|n| matches(&a.src, n) && matches(&b.src, n));
            if specificity(&a.src) == specificity(&b.src)
                && overlap
                && (a.allowed.iter().collect::<BTreeSet<_>>()
                    != b.allowed.iter().collect::<BTreeSet<_>>()
                    || a.default_profile != b.default_profile
                    || a.locked != b.locked)
            {
                report(
                    &mut diagnostics,
                    format!("assignments[{j}]"),
                    format!(
                        "Conflicts assignments {i}/{j}; example source matching {} and {}",
                        a.src, b.src
                    ),
                );
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    let mut matchers = Vec::new();
    let mut matcher_ids = BTreeMap::new();
    // One public matcher serves mesh-only and every global-exit built-in.
    let mut public_index = DestinationIndex::default();
    for (i, r) in public.exits.iter().enumerate() {
        for d in &r.destinations {
            public_index.insert(d, i as u16);
        }
    }
    matchers.push(ProfileMatcher {
        rules: public.exits.clone(),
        index: public_index,
    });
    for (id, (rules, _)) in &definitions {
        let mut combined = public.exits.clone();
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
        matcher_ids.insert(id.clone(), matchers.len() as u16);
        matchers.push(ProfileMatcher {
            rules: combined,
            index,
        });
    }
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
                if all_external(&base, src) {
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
        let unrestricted = all_external(&base, src);
        for id in allowed.iter().filter(|id| id.as_str() != AUTO) {
            let (matcher, fallback) = if id == MESH_ONLY {
                (0, ProfileFallback::None)
            } else if let Some(exit) = id.strip_prefix("global:") {
                if !nodes.iter().any(|n| n.id == exit && n.exit_enabled)
                    || !all_external(&base, src)
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
            } else if let Some((_, fallback)) = definitions.get(id) {
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
    let path_ids =
        super::engine::append_external_requests(&intent, facts, options, &mut base, all_requests)?;
    let resolve = |groups: PathPlan| {
        groups
            .into_iter()
            .flat_map(|mut group| {
                group.sort_by(|a, b| {
                    let rank = |r: &PathRequest| {
                        path_ids[r]
                            .first()
                            .map_or(usize::MAX, |i| base.paths[*i as usize].len())
                    };
                    rank(a).cmp(&rank(b)).then(a.dst.cmp(&b.dst))
                });
                group.into_iter().flat_map(|r| path_ids[&r].clone())
            })
            .collect::<Vec<_>>()
    };
    let sources = planned
        .into_iter()
        .map(|s| SourceProfiles {
            allowed: s.allowed,
            default_profile: s.default_profile,
            locked: s.locked,
            automatic: p.automatic.clone(),
            profiles: s
                .profiles
                .into_iter()
                .map(|p| CompiledProfile {
                    id: p.id,
                    matcher: p.matcher,
                    routes: p
                        .routes
                        .into_iter()
                        .map(|(rule, target, paths)| ProfileRoute {
                            rule,
                            target,
                            paths: resolve(paths),
                        })
                        .collect(),
                    ip_table: p.table,
                    fallback: match p.fallback {
                        None => CompiledFinal::None,
                        Some((true, _)) => CompiledFinal::Direct,
                        Some((false, paths)) => CompiledFinal::Nodes(resolve(paths)),
                    },
                    lints: p.lints,
                })
                .collect(),
        })
        .collect();
    Ok(NetworkCompilation {
        base,
        matchers,
        ip_tables: tables,
        sources,
    })
}
#[cfg(test)]
#[path = "profiles_tests.rs"]
mod tests;
