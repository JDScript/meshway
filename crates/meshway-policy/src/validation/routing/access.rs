use super::*;

/// Declared destination coverage for one Node: the union of every access
/// grant whose source matches it. Pure declaration, not path compilation or a
/// grant to any runtime; remote resources need resolved content before
/// publication. Built once per Node and queried per destination.
///
/// Domain grants have address-set semantics: a name is authorized through the
/// addresses it resolves to, which `autogroup:internet` covers for any public
/// resolution. Declaration therefore accepts a domain destination under an
/// Internet grant; compilation still checks each resolved address.
struct Permits {
    domains: SuffixSet,
    ranges: AddressSet,
    internet: bool,
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
        let mut internet = false;
        for value in &allowed {
            if *value == "autogroup:internet" {
                ranges.extend_from_slice(internet_ranges());
                internet = true;
            } else if let Ok(net) = value.parse::<IpNet>() {
                ranges.push(net);
            }
        }
        Self {
            domains,
            ranges: AddressSet::new(ranges),
            internet,
        }
    }

    fn covers(&self, destination: &str) -> bool {
        if let Some(domain) = destination.strip_prefix("domain:") {
            return self.internet || self.domains.matches(domain);
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
/// Exit rules of one Node must only send destinations its access grants
/// cover. Reported at the rule, with the Node as a parameter.
fn verify_rules(
    sets: Sets<'_>,
    permits: &Permits,
    node: NodeId,
    rules: &[EgressRule],
    rules_path: &str,
    check: &mut Check,
    previous: Option<PriorRules<'_>>,
) {
    for (index, rule) in rules.iter().enumerate() {
        if !matches!(rule.action, RuleAction::Exit { .. }) {
            continue;
        }
        // A rule without loaded content already requires its preparation.
        let Some(expanded) = expanded_match(&rule.matcher, sets) else {
            continue;
        };
        // An exact name or a subdomain member needs the same coverage as the
        // suffix equal to it; a keyword only an Internet grant covers.
        let mut destinations: Vec<String> = expanded.ips.iter().map(ToString::to_string).collect();
        destinations.extend(
            expanded
                .suffixes
                .iter()
                .chain(&expanded.exact)
                .chain(&expanded.subdomains)
                .map(|s| format!("domain:{s}")),
        );
        let outside = destinations
            .iter()
            .find(|d| !permits.covers(d))
            .cloned()
            .or_else(|| {
                (!permits.internet)
                    .then(|| expanded.keywords.first().map(|k| format!("keyword:{k}")))
                    .flatten()
            });
        if let Some(outside) = outside {
            check.dependency(
                ConfigurationIssue::new(
                    format!("{rules_path}/{index}"),
                    IssueCode::ExitOutsideAccess,
                )
                .with("node", node)
                .with("destination", outside),
                retained_egress(rule, sets, previous),
            );
        }
    }
}
pub(super) fn validate_assignments_access(
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
    let assignments = &config.routing.egress.assignments;
    let judged = judged_sets(config, &context.rule_sets);
    for id in config.nodes.keys() {
        // Every (assignment, allowed profile) pair that applies to this Node,
        // with the pointer of the allowing entry.
        let allowed: Vec<(String, &String)> = assignments
            .iter()
            .enumerate()
            .filter(|(_, a)| a.target.iter().any(|s| subjects.matches(config, s, *id)))
            .flat_map(|(ai, a)| {
                a.allowed.iter().enumerate().map(move |(ji, name)| {
                    (
                        format!("/routing/egress/assignments/{ai}/allowed/{ji}"),
                        name,
                    )
                })
            })
            .collect();
        let previous = context.previous.filter(|old| old.nodes.contains_key(id));
        let permits = Permits::for_node(config, *id, &subjects);
        let sets = Sets {
            configuration: config,
            loaded: &context.rule_sets,
        };
        verify_rules(
            sets,
            &permits,
            *id,
            &config.routing.egress.rules,
            "/routing/egress/rules",
            check,
            previous.map(|old| PriorRules {
                configuration: old,
                rule_sets: &context.previous_rule_sets,
                rules: &old.routing.egress.rules,
            }),
        );
        if allowed.iter().any(|(_, name)| name.as_str() == "auto")
            && let Input::Value(auto) = &config.routing.egress.auto
            && let Some((path, outside)) = auto
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
                )))
                .find(|(_, p)| !allowed.iter().any(|(_, name)| name == p))
        {
            check.issue(
                ConfigurationIssue::new(path, IssueCode::AutoTargetNotAllowed)
                    .with("profile", outside)
                    .with("node", id),
            );
        }
        dns_plans(config, &judged, &permits, *id, &allowed, check);
        let mut seen = BTreeSet::new();
        for (allowed_path, name) in &allowed {
            let name: &String = name;
            let retained = previous.is_some_and(|old| {
                old.routing.egress.assignments.iter().any(|a| {
                    a.allowed.contains(name)
                        && a.target.iter().any(|s| prior_subjects.matches(old, s, *id))
                })
            });
            if name.starts_with("global:") && !permits.covers("autogroup:internet") {
                check.dependency(
                    ConfigurationIssue::new(allowed_path, IssueCode::GlobalExitRequiresInternet)
                        .with("node", id)
                        .with("profile", name),
                    retained,
                );
            }
            if !seen.insert(name) {
                continue;
            }
            if let Some(profile) = config.routing.egress.profiles.get(name) {
                let profile_path = format!("/routing/egress/profiles/{}", pointer(name));
                let old_profile = previous
                    .filter(|_| retained)
                    .and_then(|old| old.routing.egress.profiles.get(name));
                verify_rules(
                    sets,
                    &permits,
                    *id,
                    &profile.rules,
                    &format!("{profile_path}/rules"),
                    check,
                    previous.and_then(|old| {
                        old_profile.map(|p| PriorRules {
                            configuration: old,
                            rule_sets: &context.previous_rule_sets,
                            rules: &p.rules,
                        })
                    }),
                );
                if !retained {
                    // A profile newly allowed for this Node needs its
                    // dependencies live, not merely retained.
                    for (index, rule) in profile.rules.iter().enumerate() {
                        action(
                            &rule.action.clone().into(),
                            config,
                            &format!("{profile_path}/rules/{index}/action"),
                            check,
                            false,
                        );
                    }
                    action(
                        &profile.fallback,
                        config,
                        &format!("{profile_path}/fallback"),
                        check,
                        false,
                    );
                }
                if matches!(profile.fallback, RouteAction::Exit { .. })
                    && !permits.covers("autogroup:internet")
                {
                    check.dependency(
                        ConfigurationIssue::new(
                            format!("{profile_path}/fallback"),
                            IssueCode::ExitFallbackRequiresInternet,
                        )
                        .with("node", id)
                        .with("profile", name),
                        old_profile.is_some_and(|p| p.fallback == profile.fallback),
                    );
                }
            }
        }
    }
}

/// Every profile a Node may execute must have an executable DNS plan: its
/// name rules (and an exit fallback) leave through one exit and via
/// ([`crate::compile::dns_resolution`], the function its netmap uses). A
/// Node without an assignment executes `mesh`. Profiles whose rule sets
/// await their content are judged once the content is prepared.
fn dns_plans(
    config: &NetworkConfiguration,
    sets: &BTreeMap<String, Arc<meshway_rules::RuleSet>>,
    permits: &Permits,
    node: NodeId,
    allowed: &[(String, &String)],
    check: &mut Check,
) {
    let mesh = "mesh".to_owned();
    let implicit = [("/routing/egress/rules".to_owned(), &mesh)];
    let profiles: &[(String, &String)] = if allowed.is_empty() {
        &implicit
    } else {
        allowed
    };
    let mut seen = BTreeSet::new();
    for (path, name) in profiles {
        if !seen.insert(*name) || !profile_exists(name, config, true) {
            continue;
        }
        let profile_rules = config
            .routing
            .egress
            .profiles
            .get(*name)
            .map(|profile| profile.rules.as_slice())
            .unwrap_or_default();
        if config
            .routing
            .egress
            .rules
            .iter()
            .chain(profile_rules)
            .flat_map(|rule| &rule.matcher.rule_sets)
            .any(|key| !sets.contains_key(key))
        {
            continue;
        }
        if let Ok(crate::compile::DnsResolution::PlanRequired { pointer }) =
            crate::compile::dns_resolution(
                config,
                crate::compile::Exits::Full(config),
                sets,
                name,
                node,
            )
        {
            check.issue(
                ConfigurationIssue::new(
                    pointer.unwrap_or_else(|| path.clone()),
                    IssueCode::DnsExitPlanRequired,
                )
                .with("node", node)
                .with("profile", *name),
            );
        }
        // A kept resolver under an exit is reached from the Node itself;
        // that exception is no Access grant, so say when none covers it.
        for key in crate::compile::excepted_resolvers(config, sets, name) {
            let address = config.dns.servers[&key].server.to_canonical();
            if !permits.covers(&ipnet::IpNet::from(address).to_string()) {
                check.warn(
                    ConfigurationIssue::new(
                        format!("/dns/servers/{}", pointer(&key)),
                        IssueCode::ResolverOutsideAccess,
                    )
                    .with("node", node)
                    .with("profile", *name)
                    .with("server", &key),
                );
            }
        }
    }
}
