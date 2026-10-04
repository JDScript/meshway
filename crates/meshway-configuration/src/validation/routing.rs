use super::{references::*, *};
use crate::internet::{AddressSet, internet_ranges};
use ipnet::IpNet;

#[derive(Clone, Copy)]
struct PriorRules<'a> {
    configuration: &'a NetworkConfiguration,
    rules: &'a [EgressRule],
}
fn expanded_match(matcher: &RouteMatch, config: &NetworkConfiguration) -> Option<RouteMatch> {
    let mut result = matcher.clone();
    for key in &matcher.rule_sets {
        let RuleSet::Inline {
            ip_cidr,
            domain_suffix,
        } = config.rule_sets.get(key)?
        else {
            return None;
        };
        result.ip_cidr.extend(ip_cidr.iter().cloned());
        result.domain_suffix.extend(domain_suffix.iter().cloned());
    }
    result.rule_sets.clear();
    Some(result)
}
fn retained_egress(
    rule: &EgressRule,
    config: &NetworkConfiguration,
    previous: Option<PriorRules<'_>>,
) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    previous.rules.iter().any(|old| {
        if rule.action != old.action {
            return false;
        }
        // Resolved rule sets are inline content here, so equal matchers with
        // equal referenced content are the same grant without any expansion.
        if rule.matcher == old.matcher
            && rule
                .matcher
                .rule_sets
                .iter()
                .all(|key| config.rule_sets.get(key) == previous.configuration.rule_sets.get(key))
        {
            return true;
        }
        match (
            expanded_match(&rule.matcher, config),
            expanded_match(&old.matcher, previous.configuration),
        ) {
            (Some(current), Some(old)) => {
                let Ok(grants) = old
                    .ip_cidr
                    .iter()
                    .map(|s| s.parse::<IpNet>())
                    .collect::<Result<Vec<_>, _>>()
                else {
                    return false;
                };
                let grants = AddressSet::new(grants);
                let suffixes = SuffixSet::new(old.domain_suffix.iter().map(String::as_str));
                current
                    .ip_cidr
                    .iter()
                    .all(|s| s.parse::<IpNet>().is_ok_and(|net| grants.covers(&net)))
                    && current
                        .domain_suffix
                        .iter()
                        .all(|domain| suffixes.matches(domain))
            }
            _ => false,
        }
    })
}

fn node_selector(
    value: &str,
    config: &NetworkConfiguration,
    path: &str,
    check: &mut Check,
    retained: bool,
) -> Option<NodeId> {
    match value
        .strip_prefix("node:")
        .and_then(|s| s.parse::<NodeId>().ok())
        .filter(|id| config.nodes.contains_key(id))
    {
        Some(id) => Some(id),
        None => {
            let valid_id = value
                .strip_prefix("node:")
                .is_some_and(|s| s.parse::<NodeId>().is_ok());
            check.dependency(
                path,
                "expected a known concrete Node selector",
                retained && valid_id,
            );
            None
        }
    }
}
fn via(
    values: &[String],
    config: &NetworkConfiguration,
    path: &str,
    check: &mut Check,
    retained: bool,
) -> BTreeSet<NodeId> {
    let mut ids = BTreeSet::new();
    for value in values {
        if let Some(id) = node_selector(value, config, path, check, retained)
            && !ids.insert(id)
        {
            check.issue(path, "repeated required Node");
        }
    }
    ids
}
fn action(
    value: &RouteAction,
    config: &NetworkConfiguration,
    path: &str,
    check: &mut Check,
    retained: bool,
) {
    if let RouteAction::Exit { nodes, via: hops } = value {
        if nodes.is_empty() {
            check.issue(path, "exit candidate set must not be empty");
        }
        let required = via(hops, config, path, check, retained);
        let mut seen = BTreeSet::new();
        for value in nodes {
            if let Some(id) = node_selector(value, config, path, check, retained) {
                if !seen.insert(id) || required.contains(&id) {
                    check.issue(path, "repeated exit or exit included in via");
                }
                if !config.nodes[&id].exit_enabled {
                    check.dependency(path, "exit Node is not enabled as an exit", retained);
                }
            }
        }
    }
}
fn destinations(cidr: &mut [String], domains: &mut [String], path: &str, check: &mut Check) {
    for text in cidr {
        match text.parse::<IpNet>() {
            Ok(net) if net.addr() == net.network() => *text = net.to_string(),
            _ => check.issue(path, "invalid or noncanonical destination CIDR"),
        }
    }
    for text in domains {
        match domain_name(text) {
            Ok(domain) => *text = domain,
            Err(()) => check.issue(path, "invalid domain suffix"),
        }
    }
}
fn profile_exists(value: &str, config: &NetworkConfiguration, allow_auto: bool) -> bool {
    value == "mesh"
        || (allow_auto && value == "auto" && matches!(config.routing.egress.auto, Input::Value(_)))
        || config.routing.egress.profiles.contains_key(value)
        || value
            .strip_prefix("global:")
            .and_then(|s| s.parse::<NodeId>().ok())
            .is_some_and(|id| config.nodes.get(&id).is_some_and(|n| n.exit_enabled))
}
fn rules(
    rules: &mut [EgressRule],
    config: &NetworkConfiguration,
    path: &str,
    check: &mut Check,
    previous: Option<PriorRules<'_>>,
) {
    for (i, rule) in rules.iter_mut().enumerate() {
        let path = format!("{path}/{i}");
        destinations(
            &mut rule.matcher.ip_cidr,
            &mut rule.matcher.domain_suffix,
            &path,
            check,
        );
        if rule.matcher.ip_cidr.is_empty()
            && rule.matcher.domain_suffix.is_empty()
            && rule.matcher.rule_sets.is_empty()
        {
            check.issue(&path, "empty route match");
        }
        for key in &rule.matcher.rule_sets {
            if !config.rule_sets.contains_key(key) {
                check.issue(&path, "unknown rule set");
            } else if matches!(config.rule_sets.get(key), Some(RuleSet::Remote { .. })) {
                check.require(
                    format!("/ruleSets/{}", pointer(key)),
                    PreparationKind::RuleSetContent,
                );
            }
        }
        if rule.action == RouteAction::Unmanaged {
            check.issue(&path, "unmanaged is only valid as a fallback");
        }
        action(
            &rule.action,
            config,
            &path,
            check,
            retained_egress(rule, config, previous),
        );
    }
}

fn selector_overlap(
    a: &str,
    b: &str,
    config: &NetworkConfiguration,
    subjects: &SubjectContext,
) -> bool {
    if a == "*" || b == "*" || a == b {
        return true;
    }
    if let Some(id) = a
        .strip_prefix("node:")
        .and_then(|s| s.parse::<NodeId>().ok())
    {
        return config
            .nodes
            .get(&id)
            .is_some_and(|_| subjects.matches(config, b, id));
    }
    if b.starts_with("node:") {
        return selector_overlap(b, a, config, subjects);
    }
    if a.starts_with("tag:") || b.starts_with("tag:") {
        return a.starts_with("tag:") && b.starts_with("tag:");
    }
    let users = |s: &str| {
        config
            .groups
            .get(s)
            .cloned()
            .unwrap_or_else(|| vec![s.to_owned()])
    };
    let b = users(b);
    users(a).iter().any(|u| b.contains(u))
}
fn selector_covers(
    a: &str,
    b: &str,
    config: &NetworkConfiguration,
    subjects: &SubjectContext,
) -> bool {
    if a == "*" || a == b {
        return true;
    }
    if b == "*" {
        return false;
    }
    if let Some(id) = b
        .strip_prefix("node:")
        .and_then(|s| s.parse::<NodeId>().ok())
    {
        return config
            .nodes
            .get(&id)
            .is_some_and(|_| subjects.matches(config, a, id));
    }
    match (config.groups.get(a), config.groups.get(b)) {
        (Some(a), Some(b)) => b.iter().all(|u| a.contains(u)),
        (Some(a), None) => a.iter().any(|u| u == b),
        _ => false,
    }
}

pub(super) fn validate(
    config: &mut NetworkConfiguration,
    context: &ValidationContext<'_>,
    check: &mut Check,
) {
    // Validate declared sources separately; trusted content must not hide an
    // invalid URL. Resolved copies make retained-rule coverage compare content,
    // rather than treating an unchanged reference as unchanged authorization.
    let remote: BTreeMap<_, _> = config
        .rule_sets
        .iter()
        .filter(|(_, value)| matches!(value, RuleSet::Remote { .. }))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for (key, set) in &remote {
        if let RuleSet::Remote { url, .. } = set {
            let authority = url
                .strip_prefix("https://")
                .and_then(|s| s.split(['/', '?', '#']).next());
            if authority.is_none_or(|a| !https_authority(a))
                || url.len() > 2048
                || url.chars().any(|c| c.is_control() || c.is_whitespace())
                || url.contains('#')
            {
                check.issue(
                    format!("/ruleSets/{}", pointer(key)),
                    "rule source must be HTTPS without credentials or fragment",
                );
            }
        }
    }
    fn resolve(
        config: &mut NetworkConfiguration,
        contents: &BTreeMap<String, meshway_model::resources::RuleSetData>,
    ) {
        for (key, data) in contents {
            if matches!(config.rule_sets.get(key), Some(RuleSet::Remote { .. })) {
                config.rule_sets.insert(
                    key.clone(),
                    RuleSet::Inline {
                        ip_cidr: data.ip_cidr.clone(),
                        domain_suffix: data.domain_suffix.clone(),
                    },
                );
            }
        }
    }
    resolve(config, &context.rule_sets);
    let mut previous = context.previous.cloned();
    if let Some(previous) = &mut previous {
        resolve(previous, &context.previous_rule_sets);
    }
    let context = ValidationContext {
        previous: previous.as_ref(),
        ..context.clone()
    };
    validate_resolved(config, &context, check);
    config.rule_sets.extend(remote);
}

fn validate_resolved(
    config: &mut NetworkConfiguration,
    context: &ValidationContext<'_>,
    check: &mut Check,
) {
    for (key, set) in &mut config.rule_sets {
        let path = format!("/ruleSets/{}", pointer(key));
        if key.is_empty() {
            check.issue(&path, "empty rule set key");
        }
        match set {
            RuleSet::Inline {
                ip_cidr,
                domain_suffix,
            } => {
                if ip_cidr.is_empty() && domain_suffix.is_empty() {
                    check.issue(&path, "empty inline rule set");
                }
                destinations(ip_cidr, domain_suffix, &path, check);
            }
            // Sources were checked before resolving immutable contents.
            RuleSet::Remote { .. } => {}
        }
    }
    for (i, rule) in config.routing.mesh.iter().enumerate() {
        let path = format!("/routing/mesh/{i}");
        selectors(
            &rule.src,
            false,
            config,
            context,
            &format!("{path}/src"),
            check,
        );
        selectors(
            &rule.dst,
            false,
            config,
            context,
            &format!("{path}/dst"),
            check,
        );
        let retained = context.previous.is_some_and(|old| {
            old.routing.mesh.iter().any(|before| {
                before.src == rule.src && before.dst == rule.dst && before.via == rule.via
            })
        });
        via(&rule.via, config, &format!("{path}/via"), check, retained);
    }
    let mut public = std::mem::take(&mut config.routing.egress.rules);
    rules(
        &mut public,
        config,
        "/routing/egress/rules",
        check,
        context.previous.map(|old| PriorRules {
            configuration: old,
            rules: &old.routing.egress.rules,
        }),
    );
    config.routing.egress.rules = public;
    let mut profiles = std::mem::take(&mut config.routing.egress.profiles);
    for (key, profile) in &mut profiles {
        let path = format!("/routing/egress/profiles/{}", pointer(key));
        if key.is_empty() || matches!(key.as_str(), "mesh" | "auto") || key.starts_with("global:") {
            check.issue(&path, "reserved or empty profile name");
        }
        let previous = context
            .previous
            .and_then(|old| old.routing.egress.profiles.get(key));
        rules(
            &mut profile.rules,
            config,
            &format!("{path}/rules"),
            check,
            context.previous.and_then(|old| {
                previous.map(|p| PriorRules {
                    configuration: old,
                    rules: &p.rules,
                })
            }),
        );
        action(
            &profile.fallback,
            config,
            &format!("{path}/fallback"),
            check,
            previous.is_some_and(|p| p.fallback == profile.fallback),
        );
    }
    config.routing.egress.profiles = profiles;
    let assignments = &config.routing.egress.assignments;
    for (i, assignment) in assignments.iter().enumerate() {
        let path = format!("/routing/egress/assignments/{i}");
        selectors(
            &assignment.target,
            false,
            config,
            context,
            &format!("{path}/target"),
            check,
        );
    }
    let subjects = SubjectContext {
        users: check.bindings.clone(),
        node_users: check.node_bindings.clone(),
        members: context.members.clone(),
    };
    for (i, assignment) in assignments.iter().enumerate() {
        let path = format!("/routing/egress/assignments/{i}");
        for profile in assignment
            .allowed
            .iter()
            .chain(std::iter::once(&assignment.default))
        {
            if !profile_exists(profile, config, true) {
                let retained = profile.starts_with("global:")
                    && context
                        .previous
                        .is_some_and(|old| old.routing.egress.assignments.contains(assignment));
                check.dependency(&path, "unknown routing profile", retained);
            }
        }
        for target in &assignment.target {
            let guaranteed: BTreeSet<_> = assignments
                .iter()
                .filter(|a| {
                    a.target
                        .iter()
                        .any(|s| selector_covers(s, target, config, &subjects))
                })
                .flat_map(|a| a.allowed.iter())
                .collect();
            if !guaranteed.contains(&assignment.default) {
                check.issue(&path, "default is not in the merged allowed set");
            }
            if guaranteed.iter().any(|p| p.as_str() == "auto")
                && let Input::Value(auto) = &config.routing.egress.auto
                && auto
                    .rules
                    .iter()
                    .map(|r| &r.profile)
                    .chain(std::iter::once(&auto.fallback))
                    .any(|p| !guaranteed.contains(p))
            {
                check.issue(&path, "auto target outside merged allowed profiles");
            }
        }
        for previous in &assignments[..i] {
            if assignment.target.iter().any(|a| {
                previous
                    .target
                    .iter()
                    .any(|b| selector_overlap(a, b, config, &subjects))
            }) && (assignment.default != previous.default
                || assignment.locked != previous.locked)
            {
                check.issue(
                    &path,
                    "overlapping assignments have conflicting default or locked",
                );
            }
        }
    }
    if let Input::Value(auto) = &config.routing.egress.auto {
        for value in auto
            .rules
            .iter()
            .map(|r| &r.profile)
            .chain(std::iter::once(&auto.fallback))
        {
            if !profile_exists(value, config, false) || value == "auto" {
                let retained = value.starts_with("global:")
                    && context
                        .previous
                        .is_some_and(|old| old.routing.egress.auto == config.routing.egress.auto);
                check.dependency(
                    "/routing/egress/auto",
                    "unknown or recursive auto target",
                    retained,
                );
            }
        }
        for rule in &auto.rules {
            if rule.countries.is_empty()
                || rule
                    .countries
                    .iter()
                    .any(|s| s.len() != 2 || !s.bytes().all(|b| b.is_ascii_uppercase()))
            {
                check.issue(
                    "/routing/egress/auto/rules",
                    "expected uppercase two-letter country codes",
                );
            }
        }
    }
    validate_assignments_access(config, context, check);
}

fn https_authority(authority: &str) -> bool {
    if authority.contains('@') {
        return false;
    }
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let Some((host, suffix)) = rest.split_once(']') else {
            return false;
        };
        if host.parse::<std::net::Ipv6Addr>().is_err() {
            return false;
        }
        if suffix.is_empty() {
            (host, None)
        } else if let Some(port) = suffix.strip_prefix(':') {
            (host, Some(port))
        } else {
            return false;
        }
    } else if let Some((host, port)) = authority.split_once(':') {
        (host, Some(port))
    } else {
        (authority, None)
    };
    meshway_transport_spec::normalize_host(host).is_ok()
        && port.is_none_or(|p| p.parse::<u16>().is_ok_and(|p| p > 0))
}

/// Declared destination coverage for one Node: the union of every access
/// grant whose source matches it. Pure declaration, not path compilation or a
/// grant to any runtime; remote resources need resolved content before
/// publication. Built once per Node and queried per destination.
struct Permits {
    domains: SuffixSet,
    ranges: AddressSet,
}

impl Permits {
    fn for_node(config: &NetworkConfiguration, id: NodeId, subjects: &SubjectContext) -> Self {
        let allowed: Vec<&String> = config
            .access
            .iter()
            .filter(|r| r.src.iter().any(|s| subjects.matches(config, s, id)))
            .flat_map(|r| &r.dst)
            .collect();
        let domains = SuffixSet::new(
            allowed
                .iter()
                .filter_map(|value| value.strip_prefix("domain:")),
        );
        let mut ranges = Vec::new();
        for value in &allowed {
            if *value == "autogroup:internet" {
                ranges.extend_from_slice(internet_ranges());
            } else if let Ok(net) = value.parse::<IpNet>() {
                ranges.push(net);
            }
        }
        Self {
            domains,
            ranges: AddressSet::new(ranges),
        }
    }

    fn covers(&self, destination: &str) -> bool {
        if let Some(domain) = destination.strip_prefix("domain:") {
            return self.domains.matches(domain);
        }
        if destination == "autogroup:internet" {
            return internet_ranges()
                .iter()
                .all(|target| self.ranges.covers(target));
        }
        destination
            .parse::<IpNet>()
            .is_ok_and(|target| self.ranges.covers(&target))
    }
}
fn verify_rules(
    config: &NetworkConfiguration,
    permits: &Permits,
    rules: &[EgressRule],
    path: &str,
    check: &mut Check,
    previous: Option<PriorRules<'_>>,
) {
    for rule in rules {
        if !matches!(rule.action, RouteAction::Exit { .. }) {
            continue;
        }
        let mut destinations: Vec<_> = rule.matcher.ip_cidr.clone();
        destinations.extend(
            rule.matcher
                .domain_suffix
                .iter()
                .map(|s| format!("domain:{s}")),
        );
        for key in &rule.matcher.rule_sets {
            match config.rule_sets.get(key) {
                Some(RuleSet::Inline {
                    ip_cidr,
                    domain_suffix,
                }) => {
                    destinations.extend(ip_cidr.clone());
                    destinations.extend(domain_suffix.iter().map(|s| format!("domain:{s}")));
                }
                // Every rule reference already requires unresolved content.
                Some(RuleSet::Remote { .. }) | None => {}
            }
        }
        if destinations.iter().any(|d| !permits.covers(d)) {
            check.dependency(
                path,
                "assigned exit destination is outside access grants",
                retained_egress(rule, config, previous),
            );
        }
    }
}
fn validate_assignments_access(
    config: &NetworkConfiguration,
    context: &ValidationContext<'_>,
    check: &mut Check,
) {
    let subjects = SubjectContext {
        users: check.bindings.clone(),
        node_users: check.node_bindings.clone(),
        members: context.members.clone(),
    };
    let prior_subjects = SubjectContext {
        users: context.previous_user_bindings.clone(),
        node_users: context.previous_node_user_bindings.clone(),
        members: context.members.clone(),
    };
    for id in config.nodes.keys() {
        let allowed: BTreeSet<_> = config
            .routing
            .egress
            .assignments
            .iter()
            .filter(|a| a.target.iter().any(|s| subjects.matches(config, s, *id)))
            .flat_map(|a| a.allowed.iter())
            .collect();
        let path = format!("/nodes/{id}");
        let previous = context.previous.filter(|old| old.nodes.contains_key(id));
        let permits = Permits::for_node(config, *id, &subjects);
        verify_rules(
            config,
            &permits,
            &config.routing.egress.rules,
            &path,
            check,
            previous.map(|old| PriorRules {
                configuration: old,
                rules: &old.routing.egress.rules,
            }),
        );
        if allowed.contains(&"auto".to_owned())
            && let Input::Value(auto) = &config.routing.egress.auto
            && auto
                .rules
                .iter()
                .map(|r| &r.profile)
                .chain(std::iter::once(&auto.fallback))
                .any(|p| !allowed.contains(p))
        {
            check.issue(&path, "auto target outside allowed profiles");
        }
        for name in allowed {
            let retained = previous.is_some_and(|old| {
                old.routing.egress.assignments.iter().any(|a| {
                    a.allowed.contains(name)
                        && a.target.iter().any(|s| prior_subjects.matches(old, s, *id))
                })
            });
            if name.starts_with("global:") && !permits.covers("autogroup:internet") {
                check.dependency(
                    &path,
                    "global exit requires internet access grant",
                    retained,
                );
            }
            if let Some(profile) = config.routing.egress.profiles.get(name) {
                let old_profile = previous
                    .filter(|_| retained)
                    .and_then(|old| old.routing.egress.profiles.get(name));
                verify_rules(
                    config,
                    &permits,
                    &profile.rules,
                    &path,
                    check,
                    previous.and_then(|old| {
                        old_profile.map(|p| PriorRules {
                            configuration: old,
                            rules: &p.rules,
                        })
                    }),
                );
                if !retained {
                    for rule in &profile.rules {
                        action(&rule.action, config, &path, check, false);
                    }
                    action(&profile.fallback, config, &path, check, false);
                }
                if matches!(profile.fallback, RouteAction::Exit { .. })
                    && !permits.covers("autogroup:internet")
                {
                    check.dependency(
                        &path,
                        "exit fallback requires internet access grant",
                        old_profile.is_some_and(|p| p.fallback == profile.fallback),
                    );
                }
            }
        }
    }
}
