use super::{references::*, *};
use crate::internet_ranges;
use ipnet::IpNet;
use meshway_configuration::AddressSet;
use std::sync::Arc;

mod access;
mod sets;
use access::validate_assignments_access;
use sets::*;

#[derive(Clone, Copy)]
struct PriorRules<'a> {
    configuration: &'a NetworkConfiguration,
    rule_sets: &'a BTreeMap<String, Arc<meshway_rules::RuleSet>>,
    rules: &'a [EgressRule],
}

fn retained_egress(rule: &EgressRule, sets: Sets<'_>, previous: Option<PriorRules<'_>>) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    let old_sets = Sets {
        configuration: previous.configuration,
        loaded: previous.rule_sets,
    };
    previous.rules.iter().any(|old| {
        if rule.action != old.action {
            return false;
        }
        // Equal matchers over equal referenced content are the same grant
        // without any expansion.
        if rule.matcher == old.matcher
            && rule.matcher.rule_sets.iter().all(|key| {
                content_of(key, sets).is_some()
                    && content_of(key, sets) == content_of(key, old_sets)
            })
        {
            return true;
        }
        match (
            expanded_match(&rule.matcher, sets),
            expanded_match(&old.matcher, old_sets),
        ) {
            (Some(current), Some(old)) => {
                let grants = AddressSet::new(old.ips.iter().copied());
                let domains = DomainSet::new(
                    old.suffixes.iter().map(String::as_str),
                    old.exact.iter().map(String::as_str),
                );
                current.covered_by(&grants, &domains)
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
                ConfigurationIssue::new(path, IssueCode::UnknownNodeSelector).with("node", value),
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
    for (index, value) in values.iter().enumerate() {
        let path = format!("{path}/{index}");
        if let Some(id) = node_selector(value, config, &path, check, retained)
            && !ids.insert(id)
        {
            check.issue(
                ConfigurationIssue::new(path, IssueCode::RepeatedRequiredNode).with("node", id),
            );
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
            check.issue(ConfigurationIssue::new(
                format!("{path}/nodes"),
                IssueCode::EmptyExitSet,
            ));
        }
        let required = via(hops, config, &format!("{path}/via"), check, retained);
        let mut seen = BTreeSet::new();
        for (index, value) in nodes.iter().enumerate() {
            let path = format!("{path}/nodes/{index}");
            if let Some(id) = node_selector(value, config, &path, check, retained) {
                if !seen.insert(id) || required.contains(&id) {
                    check.issue(
                        ConfigurationIssue::new(&path, IssueCode::RepeatedExit).with("node", id),
                    );
                }
                if !config.nodes[&id].exit_enabled {
                    check.dependency(
                        ConfigurationIssue::new(&path, IssueCode::ExitNotEnabled).with("node", id),
                        retained,
                    );
                }
            }
        }
    }
}
/// `path` is the object holding `ipCIDR`, `domainSuffix`, `domain` and
/// (inline sets) `domainSubdomain`.
fn destinations(
    cidr: &mut [String],
    suffixes: &mut [String],
    exact: &mut [String],
    subdomains: &mut [String],
    path: &str,
    check: &mut Check,
) {
    for (index, text) in cidr.iter_mut().enumerate() {
        match text.parse::<IpNet>() {
            Ok(net) if net.addr() == net.network() => *text = net.to_string(),
            _ => check.issue(
                ConfigurationIssue::new(
                    format!("{path}/ipCIDR/{index}"),
                    IssueCode::InvalidDestinationCidr,
                )
                .with("value", &*text),
            ),
        }
    }
    for (field, code, names) in [
        ("domainSuffix", IssueCode::InvalidDomainSuffix, suffixes),
        ("domain", IssueCode::InvalidDomain, exact),
        ("domainSubdomain", IssueCode::InvalidDomain, subdomains),
    ] {
        for (index, text) in names.iter_mut().enumerate() {
            match domain_name(text) {
                Ok(domain) => *text = domain,
                Err(()) => check.issue(
                    ConfigurationIssue::new(format!("{path}/{field}/{index}"), code)
                        .with("value", &*text),
                ),
            }
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
    sets: Sets<'_>,
    path: &str,
    check: &mut Check,
    previous: Option<PriorRules<'_>>,
) {
    let config = sets.configuration;
    let providers =
        meshway_configuration::providers::effective_rule_set_providers(&config.rule_set_providers);
    for (i, rule) in rules.iter_mut().enumerate() {
        let path = format!("{path}/{i}");
        destinations(
            &mut rule.matcher.ip_cidr,
            &mut rule.matcher.domain_suffix,
            &mut rule.matcher.domain,
            &mut [],
            &format!("{path}/match"),
            check,
        );
        if rule.matcher.ip_cidr.is_empty()
            && rule.matcher.domain_suffix.is_empty()
            && rule.matcher.domain.is_empty()
            && rule.matcher.rule_sets.is_empty()
        {
            check.issue(ConfigurationIssue::new(
                format!("{path}/match"),
                IssueCode::EmptyRouteMatch,
            ));
        }
        for (index, key) in rule.matcher.rule_sets.iter().enumerate() {
            let at = format!("{path}/match/ruleSets/{index}");
            match meshway_configuration::providers::rule_set_reference(key) {
                Some((provider, name)) => {
                    if !providers.contains_key(provider) {
                        check.issue(
                            ConfigurationIssue::new(&at, IssueCode::UnknownRuleSetProvider)
                                .with("provider", provider),
                        );
                    } else if !meshway_configuration::providers::valid_set_name(name) {
                        check.issue(
                            ConfigurationIssue::new(&at, IssueCode::UnknownRuleSet)
                                .with("key", key),
                        );
                    } else if !sets.loaded.contains_key(key) {
                        check.require(at.clone(), PreparationKind::RuleSetContent);
                    }
                }
                None => match config.rule_sets.get(key) {
                    None => check.issue(
                        ConfigurationIssue::new(&at, IssueCode::UnknownRuleSet).with("key", key),
                    ),
                    Some(RuleSet::Remote { .. }) if !sets.loaded.contains_key(key) => {
                        check.require(
                            format!("/ruleSets/{}", pointer(key)),
                            PreparationKind::RuleSetContent,
                        );
                    }
                    Some(_) => {}
                },
            }
        }
        action(
            &rule.action.clone().into(),
            config,
            &format!("{path}/action"),
            check,
            retained_egress(rule, sets, previous),
        );
    }
    shadowed(rules, sets, path, check);
}

/// A rule whose every destination earlier rules already match never fires.
/// Rules with unresolved content are not judged either way; earlier subdomain
/// and keyword members never count as cover.
fn shadowed(rules: &[EgressRule], sets: Sets<'_>, path: &str, check: &mut Check) {
    let mut ranges = Vec::new();
    let mut suffixes = Vec::new();
    let mut exact = Vec::new();
    let mut complete = true;
    let covered = |ranges: &[IpNet], suffixes: &[String], exact: &[String], m: &Expanded| {
        let r = AddressSet::new(ranges.iter().copied());
        let d = DomainSet::new(
            suffixes.iter().map(String::as_str),
            exact.iter().map(String::as_str),
        );
        m.covered_by(&r, &d)
    };
    for (index, rule) in rules.iter().enumerate() {
        let Some(matcher) = expanded_match(&rule.matcher, sets) else {
            complete = false;
            continue;
        };
        if complete
            && index > 0
            && matcher.has_destinations()
            && covered(&ranges, &suffixes, &exact, &matcher)
        {
            // The earlier rule after which the cover became complete.
            let mut partial_ranges = Vec::new();
            let mut partial_suffixes = Vec::new();
            let mut partial_exact = Vec::new();
            let by = (0..index)
                .find(|i| {
                    if let Some(m) = expanded_match(&rules[*i].matcher, sets) {
                        partial_ranges.extend(m.ips);
                        partial_suffixes.extend(m.suffixes);
                        partial_exact.extend(m.exact);
                    }
                    covered(&partial_ranges, &partial_suffixes, &partial_exact, &matcher)
                })
                .unwrap_or(index - 1);
            check.warn(
                ConfigurationIssue::new(format!("{path}/{index}"), IssueCode::ShadowedRule)
                    .with("by", by),
            );
        }
        ranges.extend(matcher.ips);
        suffixes.extend(matcher.suffixes);
        exact.extend(matcher.exact);
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
    for (key, set) in &config.rule_sets {
        if let RuleSet::Remote { url, .. } = set
            && !https_source(url)
        {
            check.issue(ConfigurationIssue::new(
                format!("/ruleSets/{}/url", pointer(key)),
                IssueCode::InvalidRuleSource,
            ));
        }
    }
    for (name, provider) in &config.rule_set_providers {
        let path = format!("/ruleSetProviders/{}", pointer(name));
        let invalid = |field: &str| {
            ConfigurationIssue::new(&path, IssueCode::InvalidRuleSetProvider).with("field", field)
        };
        if !meshway_configuration::providers::valid_provider_name(name) {
            check.issue(invalid("name"));
        }
        let Some(provider) = provider else {
            continue;
        };
        // Signature and artifacts are named beside the catalog document.
        if !https_source(&provider.catalog) || provider.catalog.contains('?') {
            check.issue(invalid("catalog"));
        }
        if !meshway_configuration::providers::valid_collection(&provider.collection) {
            check.issue(invalid("collection"));
        }
        let keys = &provider.public_keys;
        if keys.is_empty()
            || keys.len() > 4
            || keys
                .iter()
                .any(|key| !meshway_configuration::providers::valid_public_key(key))
            || keys
                .iter()
                .enumerate()
                .any(|(i, key)| keys[..i].contains(key))
        {
            check.issue(invalid("publicKeys"));
        }
    }
    validate_resolved(config, context, check);
}

/// HTTPS without credentials or fragment, as a download source.
/// Every assignable profile's egress partition must fit what a Node
/// installs; otherwise that profile's sources have no egress.
fn partition_limits(
    config: &NetworkConfiguration,
    loaded: &BTreeMap<String, Arc<meshway_rules::RuleSet>>,
    check: &mut Check,
) {
    let mut names: BTreeSet<&str> = BTreeSet::from(["mesh"]);
    for assignment in &config.routing.egress.assignments {
        names.extend(assignment.allowed.iter().map(String::as_str));
        names.insert(&assignment.default);
    }
    names.extend(config.routing.egress.profiles.keys().map(String::as_str));
    // Inline definitions judge as the artifacts they become.
    let mut sets = loaded.clone();
    for (key, set) in &config.rule_sets {
        if let RuleSet::Inline {
            ip_cidr,
            domain_suffix,
            domain,
            domain_subdomain,
        } = set
            && !sets.contains_key(key)
            && let Ok(built) = meshway_rules::RuleSet::from_inline(&meshway_rules::inline::Inline {
                domain,
                domain_suffix,
                domain_subdomain,
                ip_cidr,
            })
        {
            sets.insert(key.clone(), Arc::new(built));
        }
    }
    for name in names {
        let summary = crate::compile::capture_summary(config, &sets, name);
        let path = if config.routing.egress.profiles.contains_key(name) {
            format!("/routing/egress/profiles/{}", pointer(name))
        } else {
            "/routing/egress/rules".to_owned()
        };
        if let Some(issue) = limit_warning(path, name, &summary) {
            check.warn(issue);
        }
    }
}

/// The `egress_partition_limit` warning of one profile: its counts beyond
/// the node-side limits, or only the work budget when the computation ran
/// out of it (then no counts exist; the Node fails closed all the same).
fn limit_warning(
    path: String,
    name: &str,
    summary: &meshway_configuration::evaluation::CaptureSummary,
) -> Option<ConfigurationIssue> {
    let ranges = summary.managed_ipv4 as usize + summary.sink_ipv6 as usize;
    let oversized = ranges > crate::compile::MAX_CAPTURE_RANGES
        || summary.routes as usize > crate::compile::MAX_CAPTURE_ROUTES;
    let exhausted = summary.unavailable
        == Some(meshway_configuration::execution::UnavailableReason::RoutingLimit);
    let issue =
        ConfigurationIssue::new(path, IssueCode::EgressPartitionLimit).with("profile", name);
    if oversized {
        Some(
            issue
                .with("ranges", ranges)
                .with("maxRanges", crate::compile::MAX_CAPTURE_RANGES)
                .with("routes", summary.routes)
                .with("maxRoutes", crate::compile::MAX_CAPTURE_ROUTES),
        )
    } else if exhausted {
        Some(issue.with("work", crate::compile::MAX_CAPTURE_WORK))
    } else {
        None
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    use meshway_configuration::{evaluation::CaptureSummary, execution::UnavailableReason};

    #[test]
    fn an_exhausted_work_budget_warns_without_counts_and_other_failures_do_not() {
        let summary = |n: u32, unavailable| CaptureSummary {
            managed_ipv4: n,
            sink_ipv6: n,
            routes: n,
            unavailable,
        };
        // A computation over its work budget reports the fail-closed 1/1/1.
        let exhausted = limit_warning(
            "/p".into(),
            "work",
            &summary(1, Some(UnavailableReason::RoutingLimit)),
        )
        .unwrap();
        assert_eq!(
            exhausted.params.get("work").map(String::as_str),
            Some(crate::compile::MAX_CAPTURE_WORK.to_string().as_str())
        );
        assert!(!exhausted.params.contains_key("ranges"));
        let oversized = limit_warning(
            "/p".into(),
            "work",
            &summary(17_000, Some(UnavailableReason::RoutingLimit)),
        )
        .unwrap();
        assert_eq!(
            oversized.params.get("ranges").map(String::as_str),
            Some("34000")
        );
        for unavailable in [None, Some(UnavailableReason::MissingDependency)] {
            assert!(limit_warning("/p".into(), "work", &summary(1, unavailable)).is_none());
        }
    }
}

fn https_source(url: &str) -> bool {
    let authority = url
        .strip_prefix("https://")
        .and_then(|s| s.split(['/', '?', '#']).next());
    authority.is_some_and(https_authority)
        && url.len() <= 2048
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
        && !url.contains('#')
}

fn validate_resolved(
    config: &mut NetworkConfiguration,
    context: &ValidationContext<'_>,
    check: &mut Check,
) {
    for (key, set) in &mut config.rule_sets {
        let path = format!("/ruleSets/{}", pointer(key));
        if key.is_empty() {
            check.issue(ConfigurationIssue::new(&path, IssueCode::EmptyRuleSetKey));
        } else if key.contains(':') {
            check.issue(
                ConfigurationIssue::new(&path, IssueCode::InvalidRuleSetKey).with("key", key),
            );
        }
        match set {
            RuleSet::Inline {
                ip_cidr,
                domain_suffix,
                domain,
                domain_subdomain,
            } => {
                if ip_cidr.is_empty()
                    && domain_suffix.is_empty()
                    && domain.is_empty()
                    && domain_subdomain.is_empty()
                {
                    check.issue(ConfigurationIssue::new(
                        &path,
                        IssueCode::EmptyInlineRuleSet,
                    ));
                }
                destinations(
                    ip_cidr,
                    domain_suffix,
                    domain,
                    domain_subdomain,
                    &path,
                    check,
                );
            }
            // Sources were checked before the routing rules.
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
        Sets {
            configuration: config,
            loaded: &context.rule_sets,
        },
        "/routing/egress/rules",
        check,
        context.previous.map(|old| PriorRules {
            configuration: old,
            rule_sets: &context.previous_rule_sets,
            rules: &old.routing.egress.rules,
        }),
    );
    config.routing.egress.rules = public;
    let mut profiles = std::mem::take(&mut config.routing.egress.profiles);
    for (key, profile) in &mut profiles {
        let path = format!("/routing/egress/profiles/{}", pointer(key));
        if key.is_empty() || matches!(key.as_str(), "mesh" | "auto") || key.starts_with("global:") {
            check.issue(
                ConfigurationIssue::new(&path, IssueCode::ReservedProfileName).with("name", key),
            );
        }
        let previous = context
            .previous
            .and_then(|old| old.routing.egress.profiles.get(key));
        rules(
            &mut profile.rules,
            Sets {
                configuration: config,
                loaded: &context.rule_sets,
            },
            &format!("{path}/rules"),
            check,
            context.previous.and_then(|old| {
                previous.map(|p| PriorRules {
                    configuration: old,
                    rule_sets: &context.previous_rule_sets,
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
    partition_limits(config, &context.rule_sets, check);
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
        let named = assignment
            .allowed
            .iter()
            .enumerate()
            .map(|(index, profile)| (format!("{path}/allowed/{index}"), profile))
            .chain(std::iter::once((
                format!("{path}/default"),
                &assignment.default,
            )));
        for (path, profile) in named {
            if !profile_exists(profile, config, true) {
                let retained = profile.starts_with("global:")
                    && context
                        .previous
                        .is_some_and(|old| old.routing.egress.assignments.contains(assignment));
                check.dependency(
                    ConfigurationIssue::new(path, IssueCode::UnknownProfile).with("name", profile),
                    retained,
                );
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
                check.issue(
                    ConfigurationIssue::new(
                        format!("{path}/default"),
                        IssueCode::DefaultNotAllowed,
                    )
                    .with("profile", &assignment.default),
                );
            }
            if guaranteed.iter().any(|p| p.as_str() == "auto")
                && let Input::Value(auto) = &config.routing.egress.auto
                && let Some(outside) = auto
                    .rules
                    .iter()
                    .map(|r| &r.profile)
                    .chain(std::iter::once(&auto.fallback))
                    .find(|p| !guaranteed.contains(p))
            {
                check.issue(
                    ConfigurationIssue::new(&path, IssueCode::AutoTargetNotAllowed)
                        .with("profile", outside),
                );
            }
        }
        for (other, previous) in assignments[..i].iter().enumerate() {
            if assignment.target.iter().any(|a| {
                previous
                    .target
                    .iter()
                    .any(|b| selector_overlap(a, b, config, &subjects))
            }) && (assignment.default != previous.default
                || assignment.locked != previous.locked)
            {
                check.issue(
                    ConfigurationIssue::new(&path, IssueCode::ConflictingAssignments)
                        .with("other", other),
                );
            }
        }
    }
    if let Input::Value(auto) = &config.routing.egress.auto {
        let targets = auto
            .rules
            .iter()
            .enumerate()
            .map(|(index, r)| {
                (
                    format!("/routing/egress/auto/rules/{index}/profile"),
                    &r.profile,
                )
            })
            .chain(std::iter::once((
                "/routing/egress/auto/fallback".to_owned(),
                &auto.fallback,
            )));
        for (path, value) in targets {
            if !profile_exists(value, config, false) || value == "auto" {
                let retained = value.starts_with("global:")
                    && context
                        .previous
                        .is_some_and(|old| old.routing.egress.auto == config.routing.egress.auto);
                check.dependency(
                    ConfigurationIssue::new(path, IssueCode::AutoTargetUnknown)
                        .with("profile", value),
                    retained,
                );
            }
        }
        for (index, rule) in auto.rules.iter().enumerate() {
            let path = format!("/routing/egress/auto/rules/{index}/countries");
            if rule.countries.is_empty() {
                check.issue(ConfigurationIssue::new(path, IssueCode::InvalidCountryCode));
            } else if let Some(bad) = rule
                .countries
                .iter()
                .find(|s| s.len() != 2 || !s.bytes().all(|b| b.is_ascii_uppercase()))
            {
                check.issue(
                    ConfigurationIssue::new(path, IssueCode::InvalidCountryCode).with("value", bad),
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
    meshway_transport::spec::normalize_host(host).is_ok()
        && port.is_none_or(|p| p.parse::<u16>().is_ok_and(|p| p > 0))
}
