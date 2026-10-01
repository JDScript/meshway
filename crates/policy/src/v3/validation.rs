use super::*;
use ipnet::IpNet;
use std::collections::BTreeSet;

pub fn validate(p: &Policy, facts: &Snapshot) -> Vec<Diagnostic> {
    match expand_rule_sets(p, &facts.rule_sets) {
        Ok(p) => validate_expanded(&p, facts),
        Err(d) => d,
    }
}
pub(crate) fn validate_expanded(p: &Policy, facts: &Snapshot) -> Vec<Diagnostic> {
    let mut d = Vec::new();
    let mut error = |field: String, message: String| {
        if d.len() < 128 {
            d.push(Diagnostic { field, message });
        }
    };
    if facts.nodes.len() > MAX_NODES {
        error("nodes".into(), "At most 1024 nodes".into());
        return d;
    }
    if serde_json::to_vec(p).map_or(true, |v| v.len() > 2 * 1024 * 1024) {
        error("policy".into(), "Policy exceeds 2 MiB".into());
        return d;
    }
    let mut nodes = BTreeSet::new();
    for (i, n) in facts.nodes.iter().enumerate() {
        if !valid_id(&n.id) || !nodes.insert(&n.id) {
            error(
                format!("nodes[{i}].id"),
                "Unique lowercase 64-hex identity required".into(),
            );
        }
        for t in &n.tags {
            if !facts.tags.contains(t) {
                error(format!("nodes[{i}].tags"), "Unknown tag".into());
            }
        }
    }
    for t in &facts.tags {
        if !crate::valid_tag_selector(t) {
            error("tags".into(), "Invalid tag".into());
        }
    }
    for dial in &facts.reachable {
        if dial.from == dial.to || !nodes.contains(&dial.from) || !nodes.contains(&dial.to) {
            error(
                "reachable".into(),
                "Dial references invalid endpoints".into(),
            );
        }
    }
    let mut ids = BTreeSet::new();
    let mut check = |section: &str,
                     i: usize,
                     id: &str,
                     selectors: Vec<(String, &str, bool)>,
                     via: &[String]| {
        let base = format!("{section}[{i}]");
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || !ids.insert(id.to_owned())
        {
            error(
                format!("{base}.id"),
                "Unique 1..64 ASCII rule ID required".into(),
            );
        }
        for (field, s, internet) in selectors {
            let reason = if s.starts_with("user:") || s.starts_with("group:") {
                Some("User/group selectors are reserved but unsupported")
            } else if internet && super::index::external_selector(s).is_some() {
                (!valid_destination(&super::index::external_selector(s).expect("checked range")))
                    .then_some("Require canonical external IP/domain range")
            } else if s == "*" || (internet && s == "autogroup:internet") {
                None
            } else if let Some(id) = s.strip_prefix("node:") {
                (!nodes.contains(&id.to_owned())).then_some("Unknown node")
            } else {
                (!facts.tags.contains(s)).then_some("Unknown or unsupported selector")
            };
            if let Some(reason) = reason {
                error(format!("{base}.{field}"), reason.into());
            }
        }
        if via.len() > MAX_PATH_NODES - 2 {
            error(format!("{base}.via"), "At most six mandatory hops".into());
        }
        let mut unique = BTreeSet::new();
        for (j, s) in via.iter().enumerate() {
            if s == "*" || !unique.insert(s) {
                error(
                    format!("{base}.via[{j}]"),
                    "Hops must be distinct node/tag selectors".into(),
                );
            }
        }
    };
    for (i, r) in p.connections.iter().enumerate() {
        check(
            "connections",
            i,
            &r.id,
            vec![("a".into(), &r.a, false), ("b".into(), &r.b, false)],
            &[],
        );
    }
    for (i, g) in p.grants.iter().enumerate() {
        check(
            "grants",
            i,
            &g.id,
            g.src
                .iter()
                .enumerate()
                .map(|(j, s)| (format!("src[{j}]"), s.as_str(), false))
                .chain(
                    g.dst
                        .iter()
                        .enumerate()
                        .map(|(j, s)| (format!("dst[{j}]"), s.as_str(), true)),
                )
                .collect(),
            &[],
        );
    }
    for (i, r) in p.mesh.iter().enumerate() {
        check(
            "mesh",
            i,
            &r.id,
            vec![
                ("src".into(), r.src.as_str(), false),
                ("dst".into(), r.dst.as_str(), false),
            ]
            .into_iter()
            .chain(
                r.via
                    .iter()
                    .enumerate()
                    .map(|(j, s)| (format!("via[{j}]"), s.as_str(), false)),
            )
            .collect(),
            &r.via,
        );
    }
    for (i, r) in p.exits.iter().enumerate() {
        check(
            "exits",
            i,
            &r.id,
            vec![("src".into(), r.src.as_str(), false)]
                .into_iter()
                .chain(
                    r.candidates
                        .iter()
                        .enumerate()
                        .map(|(j, s)| (format!("candidates[{j}]"), s.as_str(), false)),
                )
                .chain(
                    r.via
                        .iter()
                        .enumerate()
                        .map(|(j, s)| (format!("via[{j}]"), s.as_str(), false)),
                )
                .collect(),
            &r.via,
        );
    }
    for (i, r) in p.defaults.iter().enumerate() {
        check(
            "defaults",
            i,
            &r.id,
            vec![("src".into(), r.src.as_str(), false)]
                .into_iter()
                .chain(
                    r.candidates
                        .iter()
                        .enumerate()
                        .map(|(j, s)| (format!("candidates[{j}]"), s.as_str(), false)),
                )
                .chain(
                    r.via
                        .iter()
                        .enumerate()
                        .map(|(j, s)| (format!("via[{j}]"), s.as_str(), false)),
                )
                .collect(),
            &r.via,
        );
    }
    for (section, len) in [
        ("connections", p.connections.len()),
        ("grants", p.grants.len()),
        ("mesh", p.mesh.len()),
        ("exits", p.exits.len()),
        ("defaults", p.defaults.len()),
    ] {
        if len > MAX_RULES {
            error(section.into(), "At most 128 rules per layer".into());
        }
    }
    for (i, r) in p.defaults.iter().enumerate() {
        if r.target == ExitTarget::Nodes
            && (r.candidates.is_empty()
                || r.candidates.len() > 32
                || r.candidates.iter().any(|s| s == "*"))
        {
            error(
                format!("defaults[{i}].candidates"),
                "Require 1..32 node/tag candidates".into(),
            );
        }
        if r.families.is_empty() {
            error(
                format!("defaults[{i}].families"),
                "At least one IP family required".into(),
            );
        }
        if r.target == ExitTarget::Direct
            && (!r.locked || !r.candidates.is_empty() || !r.via.is_empty())
        {
            error(
                format!("defaults[{i}].target"),
                "Direct default must be locked with empty candidates/via".into(),
            );
        }
    }
    for (i, g) in p.grants.iter().enumerate() {
        if g.src.is_empty()
            || g.dst.is_empty()
            || g.src.len() > 32
            || g.dst.len() > MAX_DESTINATIONS
        {
            error(
                format!("grants[{i}]"),
                "Source requires 1..32 and destination 1..16384 entries".into(),
            );
        }
        if g.bidirectional
            && g.dst
                .iter()
                .any(|s| s == "autogroup:internet" || super::index::external_selector(s).is_some())
        {
            error(
                format!("grants[{i}].bidirectional"),
                "Internet grants cannot be bidirectional".into(),
            );
        }
    }
    for (i, r) in p.exits.iter().enumerate() {
        if r.target == ExitTarget::Nodes
            && (r.candidates.is_empty()
                || r.candidates.len() > 32
                || r.candidates.iter().any(|s| s == "*"))
        {
            error(
                format!("exits[{i}].candidates"),
                "Require 1..32 node/tag candidates".into(),
            );
        }
        if r.target == ExitTarget::Direct && (!r.candidates.is_empty() || !r.via.is_empty()) {
            error(
                format!("exits[{i}].target"),
                "Direct target requires empty candidates/via".into(),
            );
        }
        if r.destinations.is_empty() || r.destinations.len() > MAX_DESTINATIONS {
            error(
                format!("exits[{i}].destinations"),
                "Require 1..16384 destinations".into(),
            );
        }
        for (j, d) in r.destinations.iter().enumerate() {
            if !valid_destination(d) {
                error(
                    format!("exits[{i}].destinations[{j}]"),
                    "Require canonical IDNA suffix or network CIDR".into(),
                );
            }
        }
    }
    if p.exits.iter().map(|r| r.destinations.len()).sum::<usize>()
        + p.grants.iter().map(|g| g.dst.len()).sum::<usize>()
        > MAX_DESTINATIONS
    {
        error(
            "destinations".into(),
            "At most 16384 total routing and grant destination entries".into(),
        );
    }
    for (section, i, candidates) in p
        .exits
        .iter()
        .enumerate()
        .map(|(i, r)| ("exits", i, &r.candidates))
        .chain(
            p.defaults
                .iter()
                .enumerate()
                .map(|(i, r)| ("defaults", i, &r.candidates)),
        )
    {
        for (j, s) in candidates.iter().enumerate() {
            if let Some(id) = s.strip_prefix("node:") {
                if facts.nodes.iter().any(|n| n.id == id && !n.exit_enabled) {
                    error(
                        format!("{section}[{i}].candidates[{j}]"),
                        "Referenced node is not enabled as an exit by the administrator".into(),
                    );
                }
            } else if s.starts_with("tag:")
                && !facts.nodes.iter().any(|n| n.exit_enabled && matches(s, n))
            {
                error(
                    format!("{section}[{i}].candidates[{j}]"),
                    "Tag has no administrator-enabled exit nodes; candidate unavailable".into(),
                );
            }
        }
    }
    let overlap = |a: &str, b: &str| -> Option<String> {
        facts
            .nodes
            .iter()
            .find(|n| matches(a, n) && matches(b, n))
            .map(|n| n.id.clone())
            .or_else(|| {
                if a == "*"
                    || b == "*"
                    || (a.starts_with("tag:") && b.starts_with("tag:"))
                    || a == b
                {
                    Some(format!("hypothetical({a},{b})"))
                } else {
                    None
                }
            })
    };
    let defaults = default_rules(p);
    let field = |r: &DefaultExitRule| {
        p.defaults.iter().position(|d| d.id == r.id).map_or_else(
            || {
                format!(
                    "exits[{}]",
                    p.exits.iter().position(|d| d.id == r.id).unwrap_or(0)
                )
            },
            |i| format!("defaults[{i}]"),
        )
    };
    for (i, a) in defaults.iter().enumerate() {
        for b in defaults.iter().skip(i + 1) {
            if !a.families.is_disjoint(&b.families)
                && specificity(&a.src) == specificity(&b.src)
                && (a.target != b.target
                    || a.locked != b.locked
                    || a.candidates != b.candidates
                    || a.via != b.via)
                && let Some(src) = overlap(&a.src, &b.src)
            {
                error(
                    field(b),
                    format!(
                        "Conflicts {} / {}; example {src} default exit ({:?})",
                        a.id,
                        b.id,
                        a.families.intersection(&b.families).collect::<Vec<_>>()
                    ),
                );
            }
        }
    }
    d
}
pub(crate) fn valid_destination(d: &ExitMatch) -> bool {
    match d {
        ExitMatch::RuleSet(_) => false,
        ExitMatch::DomainSuffix(s) => {
            meshway_protocol::normalize_dns_name(s).is_ok_and(|n| n == *s)
        }
        ExitMatch::Cidr(s) => s.parse::<IpNet>().is_ok_and(|n| {
            n.trunc().to_string() == *s
                && !matches!(n.network(),std::net::IpAddr::V6(v) if v.to_ipv4_mapped().is_some())
        }),
    }
}
pub(crate) fn valid_id(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
