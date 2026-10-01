//! Static budgets, rule-set normalization and catalog/assignment validation.
use super::*;

pub(super) struct Catalog {
    pub public: Vec<ExitRule>,
    pub definitions: BTreeMap<String, (Vec<ExitRule>, ProfileFallback)>,
}
pub(super) fn report(
    d: &mut Vec<Diagnostic>,
    field: impl Into<String>,
    message: impl Into<String>,
) {
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
pub(super) fn check_budget(p: &NetworkPolicy) -> Result<(), Vec<Diagnostic>> {
    if p.profiles.len() > 64
        || p.assignments.len() > 128
        || serde_json::to_vec(p).map_or(true, |v| v.len() > 2 * 1024 * 1024)
    {
        return Err(vec![Diagnostic {
            field: "profiles".into(),
            message: "At most 64 custom profiles, 128 assignments and 2 MiB network intent".into(),
        }]);
    }
    Ok(())
}
pub(super) fn preflight(p: &NetworkPolicy, facts: &Snapshot) -> Result<Catalog, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
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
    Ok(Catalog {
        public: public.exits,
        definitions,
    })
}
