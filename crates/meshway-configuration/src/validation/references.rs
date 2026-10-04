use super::*;
use ipnet::IpNet;

fn named(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|s| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    })
}
pub(super) fn user(email: &str, context: &ValidationContext<'_>, path: &str, check: &mut Check) {
    // A retained reference remains bound to its original membership even after
    // removal/rejoin or email reuse. This preserves an inert dependency, never
    // creates a new authorization. Restore callers separately require every
    // historical membership to still be active.
    if let Some(expected) = context.expected_user_bindings.get(email)
        && (context.previous_user_bindings.get(email) == Some(expected)
            || context
                .previous_user_bindings
                .values()
                .any(|old| old == expected))
    {
        if context.members.get(email) != Some(expected) {
            check.dependency(path, "retained user membership is inactive", true);
        }
        check.bindings.insert(email.to_owned(), expected.clone());
        return;
    }
    match context.members.get(email) {
        Some(binding)
            if !context
                .expected_user_bindings
                .get(email)
                .is_some_and(|expected| expected != binding) =>
        {
            check.bindings.insert(email.to_owned(), binding.clone());
        }
        _ => check.issue(path, "unknown, inactive, or rebound user reference"),
    }
}
pub(super) fn definitions(
    groups: &mut BTreeMap<String, Vec<String>>,
    tags: &mut BTreeMap<String, Vec<String>>,
    context: &ValidationContext<'_>,
    check: &mut Check,
) {
    for (key, values) in groups.iter_mut() {
        let path = format!("/groups/{}", pointer(key));
        if !named(key, "group:") {
            check.issue(&path, "invalid group name");
        }
        for email in values.iter() {
            user(email, context, &path, check);
        }
        values.sort();
        values.dedup();
    }
    for (key, owners) in tags {
        let path = format!("/tagsOwners/{}", pointer(key));
        if !named(key, "tag:") {
            check.issue(&path, "invalid tag name");
        }
        for owner in owners.iter() {
            if owner.starts_with("group:") {
                if !groups.contains_key(owner) {
                    check.issue(&path, "unknown group");
                }
            } else {
                user(owner, context, &path, check);
            }
        }
        owners.sort();
        owners.dedup();
    }
}
pub(super) fn identity(
    id: NodeId,
    identity: &NodeIdentity,
    tags: &BTreeMap<String, Vec<String>>,
    context: &ValidationContext<'_>,
    path: &str,
    check: &mut Check,
) {
    match identity {
        NodeIdentity::User { user: email } => {
            let current = context.members.get(email);
            let expected = context.expected_node_user_bindings.get(&id);
            if current.is_none() || expected.is_some_and(|expected| Some(expected) != current) {
                check.issue(path, "Node identity requires an active bound membership");
            } else if let Some(binding) = current {
                check.node_bindings.insert(id, binding.clone());
                if context.previous_node_user_bindings.get(&id) != Some(binding)
                    && context.approved_identities.get(&id) != Some(identity)
                {
                    check.require(path, PreparationKind::IdentityProof);
                }
            }
        }
        NodeIdentity::Tag { tags: values } => {
            if values.is_empty() {
                check.issue(path, "Tag identity must have at least one tag");
            }
            let mut seen = BTreeSet::new();
            for tag in values {
                if !tags.contains_key(tag) {
                    check.issue(path, "unknown tag");
                }
                if !seen.insert(tag) {
                    check.issue(path, "duplicate identity tag");
                }
            }
        }
    }
}

pub(super) fn selector(
    value: &str,
    external: bool,
    config: &NetworkConfiguration,
    context: &ValidationContext<'_>,
    path: &str,
    check: &mut Check,
) {
    if value == "*" {
        return;
    }
    if value.starts_with("group:") {
        if !config.groups.contains_key(value) {
            check.issue(path, "unknown group");
        }
        return;
    }
    if value.starts_with("tag:") {
        if !config.tags_owners.contains_key(value) {
            check.issue(path, "unknown tag");
        }
        return;
    }
    if let Some(id) = value.strip_prefix("node:") {
        if !id
            .parse::<NodeId>()
            .is_ok_and(|id| config.nodes.contains_key(&id) && context.nodes.contains(&id))
        {
            let retained = id.parse::<NodeId>().is_ok()
                && context
                    .previous
                    .is_some_and(|old| retained_rule(config, old, path));
            check.dependency(path, "unknown Node selector", retained);
        }
        return;
    }
    if external {
        if value == "autogroup:internet" {
            return;
        }
        if let Some(domain) = value.strip_prefix("domain:") {
            if domain_name(domain).is_err() {
                check.issue(path, "invalid domain suffix");
            }
            return;
        }
        if let Ok(net) = value.parse::<IpNet>() {
            if net.addr() != net.network() {
                check.issue(path, "noncanonical CIDR");
            }
            return;
        }
    }
    user(value, context, path, check);
}
fn retained_rule(config: &NetworkConfiguration, old: &NetworkConfiguration, path: &str) -> bool {
    let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
    let at = |index: usize| parts.get(index).and_then(|s| s.parse::<usize>().ok());
    match parts.as_slice() {
        ["access", _, ..] => at(1).and_then(|i| config.access.get(i)).is_some_and(|r| {
            old.access.iter().any(|before| {
                before.src == r.src
                    && before.dst == r.dst
                    && before.ip == r.ip
                    && before.bidirectional == r.bidirectional
            })
        }),
        ["topology", _, ..] => at(1).and_then(|i| config.topology.get(i)).is_some_and(|r| {
            old.topology
                .iter()
                .any(|before| before.a == r.a && before.b == r.b && before.action == r.action)
        }),
        ["routing", "mesh", _, ..] => {
            at(2)
                .and_then(|i| config.routing.mesh.get(i))
                .is_some_and(|r| {
                    old.routing.mesh.iter().any(|before| {
                        before.src == r.src && before.dst == r.dst && before.via == r.via
                    })
                })
        }
        ["routing", "egress", "assignments", _, ..] => at(3)
            .and_then(|i| config.routing.egress.assignments.get(i))
            .is_some_and(|r| old.routing.egress.assignments.contains(r)),
        _ => false,
    }
}
pub(super) fn selectors(
    values: &[String],
    external: bool,
    config: &NetworkConfiguration,
    context: &ValidationContext<'_>,
    path: &str,
    check: &mut Check,
) {
    if values.is_empty() {
        check.issue(path, "selector set must not be empty");
    }
    for value in values {
        selector(value, external, config, context, path, check);
    }
}
pub(super) fn is_external(value: &str) -> bool {
    value == "autogroup:internet" || value.starts_with("domain:") || value.parse::<IpNet>().is_ok()
}
pub(super) fn domain_name(value: &str) -> Result<String, ()> {
    normalize_dns_name(value).map_err(|_| ())
}
pub(super) fn protocol(value: &str) -> bool {
    crate::access::TrafficGrant::parse(value).is_some()
}

pub(super) fn rules(
    config: &mut NetworkConfiguration,
    context: &ValidationContext<'_>,
    check: &mut Check,
) {
    for rule in &mut config.access {
        for dst in &mut rule.dst {
            if let Some(domain) = dst.strip_prefix("domain:") {
                if let Ok(value) = domain_name(domain) {
                    *dst = format!("domain:{value}");
                }
            } else if let Ok(net) = dst.parse::<IpNet>() {
                *dst = net.to_string();
            }
        }
        rule.src.sort();
        rule.src.dedup();
        rule.dst.sort();
        rule.dst.dedup();
        rule.ip.sort();
        rule.ip.dedup();
    }
    for (i, rule) in config.access.iter().enumerate() {
        let path = format!("/access/{i}");
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
            true,
            config,
            context,
            &format!("{path}/dst"),
            check,
        );
        if rule.bidirectional && rule.dst.iter().any(|v| is_external(v)) {
            check.issue(&path, "bidirectional access requires Mesh endpoints");
        }
        if rule.ip.is_empty() || !rule.ip.iter().all(|s| protocol(s)) {
            check.issue(format!("{path}/ip"), "invalid protocol/port set");
        }
    }
    for (i, rule) in config.topology.iter().enumerate() {
        selector(
            &rule.a,
            false,
            config,
            context,
            &format!("/topology/{i}/a"),
            check,
        );
        selector(
            &rule.b,
            false,
            config,
            context,
            &format!("/topology/{i}/b"),
            check,
        );
    }
    let dns = &mut config.dns;
    match domain_name(&dns.domain) {
        Ok(domain) => {
            dns.domain = domain;
            for (id, node) in &config.nodes {
                if valid_node_hostname(&node.hostname)
                    && node_dns_name(&node.hostname, &dns.domain).is_err()
                {
                    check.issue(
                        format!("/nodes/{id}/hostname"),
                        "hostname and DNS domain must form a DNS name of at most 253 bytes",
                    );
                }
            }
        }
        Err(()) => check.issue("/dns/domain", "invalid DNS domain"),
    }
    if dns.override_local && dns.default.is_empty() {
        check.issue("/dns/default", "overrideLocal requires default resolvers");
    }
    for key in &dns.default {
        if !dns.servers.contains_key(key) {
            check.issue("/dns/default", "unknown resolver");
        }
    }
    for (key, server) in &dns.servers {
        if key.is_empty() || server.server.is_unspecified() || server.server.is_multicast() {
            check.issue(format!("/dns/servers/{}", pointer(key)), "invalid resolver");
        }
    }
    let mut routes = BTreeMap::new();
    for (suffix, resolvers) in std::mem::take(&mut dns.routes) {
        let path = format!("/dns/routes/{}", pointer(&suffix));
        if resolvers.is_empty() || resolvers.iter().any(|key| !dns.servers.contains_key(key)) {
            check.issue(&path, "missing or unknown resolver");
        }
        match domain_name(&suffix) {
            Ok(suffix) => {
                if routes.insert(suffix, resolvers).is_some() {
                    check.issue(path, "duplicate normalized DNS suffix");
                }
            }
            Err(()) => check.issue(path, "invalid DNS suffix"),
        }
    }
    dns.routes = routes;
    for suffix in &mut dns.search_domains {
        match domain_name(suffix) {
            Ok(value) => *suffix = value,
            Err(()) => check.issue("/dns/searchDomains", "invalid DNS suffix"),
        }
    }
}

/// DNS suffix matching respects label boundaries. Inputs are normalized DNS
/// names; matching is case-insensitive and permits one trailing root dot.
pub fn domain_suffix_matches(name: &str, suffix: &str) -> bool {
    match (domain_name(name), domain_name(suffix)) {
        (Ok(name), Ok(suffix)) => {
            name == suffix || name.strip_suffix(&suffix).is_some_and(|p| p.ends_with('.'))
        }
        _ => false,
    }
}

/// Normalized domain suffixes with label-wise lookup: matching a name costs one
/// normalization plus one set probe per label, independent of the set size.
#[derive(Clone, Debug, Default)]
pub(super) struct SuffixSet {
    suffixes: BTreeSet<String>,
}

impl SuffixSet {
    pub(super) fn new<'a>(suffixes: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            suffixes: suffixes
                .into_iter()
                .filter_map(|suffix| domain_name(suffix).ok())
                .collect(),
        }
    }

    pub(super) fn matches(&self, name: &str) -> bool {
        let Ok(name) = domain_name(name) else {
            return false;
        };
        let mut rest = name.as_str();
        loop {
            if self.suffixes.contains(rest) {
                return true;
            }
            match rest.split_once('.') {
                Some((_, parent)) => rest = parent,
                None => return false,
            }
        }
    }
}

/// Pure topology decision only: block wins regardless of rule order.
pub fn topology_allows(
    config: &NetworkConfiguration,
    subjects: &SubjectContext,
    a: NodeId,
    b: NodeId,
) -> bool {
    let (Some(_), Some(_)) = (config.nodes.get(&a), config.nodes.get(&b)) else {
        return false;
    };
    let mut allow = config.defaults.topology == TopologyDefault::Allow;
    for rule in &config.topology {
        if (subjects.matches(config, &rule.a, a) && subjects.matches(config, &rule.b, b))
            || (subjects.matches(config, &rule.a, b) && subjects.matches(config, &rule.b, a))
        {
            if rule.action == TopologyAction::Block {
                return false;
            }
            allow = true;
        }
    }
    allow
}
