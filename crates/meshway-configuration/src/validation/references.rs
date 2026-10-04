use super::*;

fn named(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|s| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    })
}

/// Resolve a declaration to its stable membership, retaining withdrawn
/// references as inert dependencies rather than rebinding reused names.
pub fn bind_user_reference(
    email: &str,
    context: &ValidationContext<'_>,
    path: &str,
    check: &mut ValidationReport,
) {
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
            bind_user_reference(email, context, &path, check);
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
                bind_user_reference(owner, context, &path, check);
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

pub(super) fn dns(config: &mut NetworkConfiguration, check: &mut Check) {
    let domain_name = |value: &str| normalize_dns_name(value).map_err(|_| ());
    let dns = &mut config.dns;
    match domain_name(&dns.domain) {
        Ok(domain) => {
            dns.domain = domain;
            for (id, node) in &config.nodes {
                if valid_node_hostname(&node.hostname)
                    && node_dns_name(&node.hostname, &dns.domain).is_err()
                {
                    check.issue(ConfigurationIssue::new(
                        format!("/nodes/{id}/hostname"),
                        IssueCode::InvalidNodeDnsName,
                    ));
                }
            }
        }
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
