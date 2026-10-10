use super::*;
use ipnet::IpNet;

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
    bind_user_reference(value, context, path, check);
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
        // The list is the edge's carrier order, so a repeat is an authoring error.
        if let Input::Value(transports) = &rule.transports {
            for (j, protocol) in transports.iter().enumerate() {
                if transports[..j].contains(protocol) {
                    check.issue(
                        ConfigurationIssue::new(
                            format!("/topology/{i}/transports/{j}"),
                            IssueCode::InvalidTransport,
                        )
                        .with(
                            "reason",
                            meshway_transport::spec::DeclarationError::RepeatedTransport.code(),
                        ),
                    );
                }
            }
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

/// Exact DNS name equality after the same normalization as suffix matching.
pub fn domain_name_matches(name: &str, domain: &str) -> bool {
    match (domain_name(name), domain_name(domain)) {
        (Ok(name), Ok(domain)) => name == domain,
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

/// The domain destinations one or more matchers declare: exact names and
/// label-boundary suffixes. Answers whether another matcher's destinations
/// are all already matched here.
#[derive(Clone, Debug, Default)]
pub(super) struct DomainSet {
    suffixes: SuffixSet,
    exact: BTreeSet<String>,
}

impl DomainSet {
    pub(super) fn new<'a>(
        suffixes: impl IntoIterator<Item = &'a str>,
        exact: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        Self {
            suffixes: SuffixSet::new(suffixes),
            exact: exact
                .into_iter()
                .filter_map(|name| domain_name(name).ok())
                .collect(),
        }
    }

    /// An exact name is matched by the same exact entry or any parent suffix.
    pub(super) fn covers_exact(&self, name: &str) -> bool {
        self.suffixes.matches(name)
            || domain_name(name).is_ok_and(|name| self.exact.contains(&name))
    }

    /// A suffix (the name and all children) is only covered by a suffix.
    pub(super) fn covers_suffix(&self, suffix: &str) -> bool {
        self.suffixes.matches(suffix)
    }

    pub(super) fn covers_all(&self, suffixes: &[String], exact: &[String]) -> bool {
        suffixes.iter().all(|suffix| self.covers_suffix(suffix))
            && exact.iter().all(|name| self.covers_exact(name))
    }
}

/// Pure topology decision only: block wins regardless of rule order.
pub fn topology_allows(
    config: &NetworkConfiguration,
    subjects: &SubjectContext,
    a: NodeId,
    b: NodeId,
) -> bool {
    topology_decision(config, subjects, a, b).is_some()
}

/// Why an edge is allowed: `Some(Some(index))` names the first allowing rule,
/// `Some(None)` is `defaults.topology: allow`, `None` is blocked (a block rule
/// or the deny default). Block wins regardless of rule order.
pub fn topology_decision(
    config: &NetworkConfiguration,
    subjects: &SubjectContext,
    a: NodeId,
    b: NodeId,
) -> Option<Option<usize>> {
    let (Some(_), Some(_)) = (config.nodes.get(&a), config.nodes.get(&b)) else {
        return None;
    };
    let mut allowing = None;
    for (index, rule) in config.topology.iter().enumerate() {
        if (subjects.matches(config, &rule.a, a) && subjects.matches(config, &rule.b, b))
            || (subjects.matches(config, &rule.a, b) && subjects.matches(config, &rule.b, a))
        {
            if rule.action == TopologyAction::Block {
                return None;
            }
            allowing.get_or_insert(index);
        }
    }
    match allowing {
        Some(index) => Some(Some(index)),
        None => (config.defaults.topology == TopologyDefault::Allow).then_some(None),
    }
}
