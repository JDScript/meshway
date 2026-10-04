use super::*;
use ipnet::IpNet;
use meshway_transport_spec::normalize_host;

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
            check.dependency(
                ConfigurationIssue::new(path, IssueCode::RetainedMembershipInactive)
                    .with("user", email),
                true,
            );
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
        _ => check.issue(ConfigurationIssue::new(path, IssueCode::UnknownUser).with("user", email)),
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
            check.issue(
                ConfigurationIssue::new(&path, IssueCode::InvalidGroupName).with("name", key),
            );
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
            check
                .issue(ConfigurationIssue::new(&path, IssueCode::InvalidTagName).with("name", key));
        }
        for owner in owners.iter() {
            if owner.starts_with("group:") {
                if !groups.contains_key(owner) {
                    check.issue(
                        ConfigurationIssue::new(&path, IssueCode::UnknownGroup).with("name", owner),
                    );
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
                check.issue(
                    ConfigurationIssue::new(path, IssueCode::IdentityMembershipMissing)
                        .with("user", email),
                );
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
                check.issue(ConfigurationIssue::new(path, IssueCode::EmptyTagIdentity));
            }
            let mut seen = BTreeSet::new();
            for tag in values {
                if !tags.contains_key(tag) {
                    check.issue(
                        ConfigurationIssue::new(path, IssueCode::UnknownTag).with("name", tag),
                    );
                }
                if !seen.insert(tag) {
                    check.issue(
                        ConfigurationIssue::new(path, IssueCode::DuplicateTag).with("name", tag),
                    );
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
            check.issue(ConfigurationIssue::new(path, IssueCode::UnknownGroup).with("name", value));
        }
        return;
    }
    if value.starts_with("tag:") {
        if !config.tags_owners.contains_key(value) {
            check.issue(ConfigurationIssue::new(path, IssueCode::UnknownTag).with("name", value));
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
            check.dependency(
                ConfigurationIssue::new(path, IssueCode::UnknownNodeSelector).with("node", id),
                retained,
            );
        }
        return;
    }
    if external {
        if value == "autogroup:internet" {
            return;
        }
        if let Some(domain) = value.strip_prefix("domain:") {
            if domain_name(domain).is_err() {
                check.issue(
                    ConfigurationIssue::new(path, IssueCode::InvalidDomainSuffix)
                        .with("value", domain),
                );
            }
            return;
        }
        if let Ok(net) = value.parse::<IpNet>() {
            if net.addr() != net.network() {
                check.issue(
                    ConfigurationIssue::new(path, IssueCode::NoncanonicalCidr).with("value", value),
                );
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
            // The new rule is not sorted yet at this point; compare as sets.
            let sorted = |values: &[String]| {
                let mut values = values.to_vec();
                values.sort();
                values.dedup();
                values
            };
            old.access.iter().any(|before| {
                before.src == sorted(&r.src)
                    && before.dst == sorted(&r.dst)
                    && before.ip == sorted(&r.ip)
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
        check.issue(ConfigurationIssue::new(path, IssueCode::EmptySelectorSet));
    }
    for (index, value) in values.iter().enumerate() {
        selector(
            value,
            external,
            config,
            context,
            &format!("{path}/{index}"),
            check,
        );
    }
}
pub(super) fn is_external(value: &str) -> bool {
    value == "autogroup:internet" || value.starts_with("domain:") || value.parse::<IpNet>().is_ok()
}
pub(super) fn domain_name(value: &str) -> Result<String, ()> {
    if value.parse::<std::net::IpAddr>().is_ok() {
        return Err(());
    }
    normalize_host(value).map_err(|_| ())
}
pub(super) fn protocol(value: &str) -> bool {
    if value == "*" {
        return true;
    }
    let Some((protocol, ports)) = value.split_once(':') else {
        return false;
    };
    if !matches!(protocol, "tcp" | "udp" | "icmp") {
        return false;
    }
    if ports == "*" {
        return true;
    }
    if protocol == "icmp" {
        return false;
    }
    fn port(s: &str) -> Option<u16> {
        s.parse::<u16>()
            .ok()
            .filter(|n| *n > 0 && n.to_string() == s)
    }
    if let Some((a, b)) = ports.split_once('-') {
        matches!((port(a),port(b)),(Some(a),Some(b)) if a <= b)
    } else {
        port(ports).is_some()
    }
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
    }
    // Pointers refer to the author's element order; canonical sorting follows.
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
            check.issue(ConfigurationIssue::new(
                &path,
                IssueCode::BidirectionalRequiresMesh,
            ));
        }
        if rule.ip.is_empty() || !rule.ip.iter().all(|s| protocol(s)) {
            check.issue(ConfigurationIssue::new(
                format!("{path}/ip"),
                IssueCode::InvalidPortSet,
            ));
        }
    }
    for rule in &mut config.access {
        rule.src.sort();
        rule.src.dedup();
        rule.dst.sort();
        rule.dst.dedup();
        rule.ip.sort();
        rule.ip.dedup();
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
        Ok(domain) => dns.domain = domain,
        Err(()) => check.issue(ConfigurationIssue::new(
            "/dns/domain",
            IssueCode::InvalidDnsDomain,
        )),
    }
    if dns.override_local && dns.default.is_empty() {
        check.issue(ConfigurationIssue::new(
            "/dns/default",
            IssueCode::DnsOverrideRequiresDefault,
        ));
    }
    for (index, key) in dns.default.iter().enumerate() {
        if !dns.servers.contains_key(key) {
            check.issue(
                ConfigurationIssue::new(
                    format!("/dns/default/{index}"),
                    IssueCode::UnknownResolver,
                )
                .with("name", key),
            );
        }
    }
    for (key, server) in &dns.servers {
        if key.is_empty() || server.server.is_unspecified() || server.server.is_multicast() {
            check.issue(ConfigurationIssue::new(
                format!("/dns/servers/{}", pointer(key)),
                IssueCode::InvalidResolver,
            ));
        }
    }
    let mut routes = BTreeMap::new();
    for (suffix, resolvers) in std::mem::take(&mut dns.routes) {
        let path = format!("/dns/routes/{}", pointer(&suffix));
        if resolvers.is_empty() {
            check.issue(ConfigurationIssue::new(&path, IssueCode::EmptyResolverSet));
        }
        for (index, key) in resolvers.iter().enumerate() {
            if !dns.servers.contains_key(key) {
                check.issue(
                    ConfigurationIssue::new(format!("{path}/{index}"), IssueCode::UnknownResolver)
                        .with("name", key),
                );
            }
        }
        match domain_name(&suffix) {
            Ok(suffix) => {
                if routes.insert(suffix.clone(), resolvers).is_some() {
                    check.issue(
                        ConfigurationIssue::new(path, IssueCode::DuplicateDnsSuffix)
                            .with("value", suffix),
                    );
                }
            }
            Err(()) => check.issue(
                ConfigurationIssue::new(path, IssueCode::InvalidDnsSuffix).with("value", &suffix),
            ),
        }
    }
    dns.routes = routes;
    for (index, suffix) in dns.search_domains.iter_mut().enumerate() {
        match domain_name(suffix) {
            Ok(value) => *suffix = value,
            Err(()) => check.issue(
                ConfigurationIssue::new(
                    format!("/dns/searchDomains/{index}"),
                    IssueCode::InvalidDnsSuffix,
                )
                .with("value", &*suffix),
            ),
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
