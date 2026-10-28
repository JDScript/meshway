//! Which resolver a source's ordinary names use is decided before resolution.
//! Answers never decide which resolver was authorized to obtain them.
use super::*;
use meshway_model::exit::ExitFamily;

pub(super) fn external_access(
    configuration: &NetworkConfiguration,
    subjects: &SubjectContext,
    source: NodeId,
) -> bool {
    configuration.access.iter().any(|rule| {
        !rule.ip.is_empty()
            && matching::selectors(configuration, subjects, &rule.src, source)
            && rule.dst.iter().any(|selector| {
                selector == "autogroup:internet"
                    || selector.starts_with("domain:")
                    || selector.parse::<ipnet::IpNet>().is_ok()
            })
    })
}

/// Where `source`'s ordinary names resolve under a profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DnsResolution {
    /// This Node's own resolvers: no name decides an exit and the fallback
    /// does not leave through one. IP rules, however many exits they use,
    /// never need an answer from an exit.
    Local,
    /// The one exit (and its ordered via) every name rule that leaves
    /// through an exit and an exit fallback share.
    Exit { exit: NodeId, via: Vec<NodeId> },
    /// Names would decide between exits (different exits or via lists, an
    /// exit set with backups) or the selection is unusable: no executable
    /// plan. `pointer` names the rule or fallback that makes it so, when one
    /// does.
    PlanRequired { pointer: Option<String> },
}

/// The DNS plan `source` executes for `profile`, on the same ordered
/// selection its partition uses (public rules first, `auto` as the profile
/// it resolves to for `source`). A rule takes part when it has names: its own domain lists,
/// or a referenced set with names; a set not in `sets` counts as one with
/// names. Each such rule leaving through an exit, and an exit fallback,
/// must name the same single exit and via; a rule naming the source itself
/// is its direct egress, and ineligible exits before it are skipped
/// ([`matching::executed`]).
pub fn resolution(
    cfg: &NetworkConfiguration,
    exits: matching::Exits<'_>,
    sets: &rules::RuleSets,
    profile: &str,
    source: NodeId,
) -> Result<DnsResolution, CompileError> {
    let capture::Selection {
        rules: selected,
        fallback,
    } = match capture::selection(cfg, profile, Some(source)) {
        Ok(selection) => selection,
        Err(CompileError::Serialization) => return Err(CompileError::Serialization),
        Err(_) => return Ok(DnsResolution::PlanRequired { pointer: None }),
    };
    let fallback_pointer = matching::concrete_profile(cfg, profile, Some(source))
        .ok()
        .and_then(|(resolved, name, _)| match resolved {
            matching::EgressProfile::Custom(_) => Some(name),
            _ => None,
        })
        .filter(|name| cfg.routing.egress.profiles.contains_key(name))
        .map(|name| {
            format!(
                "/routing/egress/profiles/{}/fallback",
                name.replace('~', "~0").replace('/', "~1")
            )
        });
    let deciding = selected
        .into_iter()
        .filter(|(_, rule)| rules::has_names(&rule.matcher, sets))
        .map(|(pointer, rule)| (Some(pointer), rule.action.clone().into()))
        .chain(std::iter::once((fallback_pointer, fallback)));
    let mut chosen: Option<(NodeId, Vec<NodeId>)> = None;
    for (pointer, action) in deciding {
        // The resolver is the IPv4 Mesh service of the exit: the IPv4
        // execution of the selection decides it.
        let RouteAction::Exit { nodes, via } =
            matching::executed(exits, action, source, ExitFamily::Ipv4)?
        else {
            continue;
        };
        let [exit] = nodes.as_slice() else {
            return Ok(DnsResolution::PlanRequired { pointer });
        };
        let candidate = (
            matching::node_selector(exit)?,
            via.iter()
                .map(|s| matching::node_selector(s))
                .collect::<Result<Vec<_>, _>>()?,
        );
        if chosen.as_ref().is_some_and(|old| *old != candidate) {
            return Ok(DnsResolution::PlanRequired { pointer });
        }
        chosen = Some(candidate);
    }
    Ok(match chosen {
        Some((exit, via)) => DnsResolution::Exit { exit, via },
        None => DnsResolution::Local,
    })
}

/// Domains (MagicDNS domain, split routes, search domains and the fallback
/// `.`) one Node's system DNS installation carries at most.
pub const MAX_SYSTEM_DNS_DOMAINS: usize = 256;

/// Why a Node cannot execute the DNS settings of its plan
/// ([`resolver_plan_gaps`]); the Node reports each as "DNS resolver plan is
/// incomplete" and the Server refuses to save a version with any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolverPlanGap {
    /// `searchDomains[index]` is neither the MagicDNS domain nor under it or
    /// a split route, and the plan has no fallback `.`: its names reach this
    /// Node's DNS link and only the system's other links could answer them,
    /// which a split-domain backend (systemd-resolved) does not flatten.
    SearchDomain { index: usize },
    /// The resolver is loopback or IPv6 link-local: a local stub can recurse
    /// through the split route being installed, and a scoped address has no
    /// published interface.
    LocalResolver { key: String },
    /// The resolver's decision is not a local socket (`Exit` or `Rejected`:
    /// a Mesh address, or an address the profile sends through an exit
    /// without `useWithExit`).
    RoutedResolver { key: String },
    /// The installation would carry more than [`MAX_SYSTEM_DNS_DOMAINS`].
    DomainLimit,
}
impl ResolverPlanGap {
    /// The `reason` parameter of `dns_resolver_plan_incomplete`.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::SearchDomain { .. } => "search_domain_without_resolver",
            Self::LocalResolver { .. } => "local_resolver",
            Self::RoutedResolver { .. } => "routed_resolver",
            Self::DomainLimit => "domain_limit",
        }
    }
    /// JSON pointer of the configuration value that makes the plan
    /// incomplete.
    pub fn pointer(&self) -> String {
        match self {
            Self::SearchDomain { index } => format!("/dns/searchDomains/{index}"),
            Self::LocalResolver { key } | Self::RoutedResolver { key } => {
                format!("/dns/servers/{}", key.replace('~', "~0").replace('/', "~1"))
            }
            Self::DomainLimit => "/dns".to_owned(),
        }
    }
}

/// Every gap of one Node's DNS plan: the one judgement the Node's DNS owner
/// installs by and the Server validates every Node and profile with.
/// `through_exit`: ordinary names resolve through an exit (`viaExit`, or the
/// unavailable plan that captures them negatively). `resolvers`: the
/// netmap's per-resolver decisions, `None` when the source's egress
/// partition is unavailable (its egress fails closed and is reported apart).
/// A resolver is judged where the Node queries it: always without an exit,
/// only with `useWithExit` through one.
pub fn resolver_plan_gaps(
    dns: &DnsConfiguration,
    through_exit: bool,
    resolvers: Option<&BTreeMap<String, meshway_configuration::netmap::ResolverAction>>,
) -> Vec<ResolverPlanGap> {
    use meshway_configuration::netmap::ResolverAction;
    let fallback = dns.override_local || through_exit;
    let mut gaps = Vec::new();
    for (key, server) in &dns.servers {
        if through_exit && !server.use_with_exit {
            continue;
        }
        let address = server.server.to_canonical();
        if address.is_loopback()
            || matches!(address, std::net::IpAddr::V6(ip) if ip.is_unicast_link_local())
        {
            gaps.push(ResolverPlanGap::LocalResolver { key: key.clone() });
        } else if let Some(resolvers) = resolvers
            && !matches!(
                resolvers.get(key),
                Some(ResolverAction::Direct | ResolverAction::Unmanaged)
            )
        {
            gaps.push(ResolverPlanGap::RoutedResolver { key: key.clone() });
        }
    }
    let name = |value: &str| normalize_dns_name(value).unwrap_or_else(|_| value.to_owned());
    let under = |parent: &str, child: &str| {
        child == parent
            || child
                .strip_suffix(parent)
                .is_some_and(|rest| rest.ends_with('.'))
    };
    let domain = name(&dns.domain);
    let routes: Vec<String> = dns.routes.keys().map(|route| name(route)).collect();
    let search: Vec<String> = dns.search_domains.iter().map(|s| name(s)).collect();
    if !fallback {
        for (index, suffix) in search.iter().enumerate() {
            if !(dns.magic_dns && under(&domain, suffix))
                && !routes.iter().any(|route| under(route, suffix))
            {
                gaps.push(ResolverPlanGap::SearchDomain { index });
            }
        }
    }
    let mut domains: std::collections::BTreeSet<&str> =
        routes.iter().chain(&search).map(String::as_str).collect();
    if dns.magic_dns {
        domains.insert(&domain);
    }
    if fallback {
        domains.insert(".");
    }
    if domains.len() > MAX_SYSTEM_DNS_DOMAINS {
        gaps.push(ResolverPlanGap::DomainLimit);
    }
    gaps
}

/// [`resolver_plan_gaps`] of `source` executing `profile`, from the full
/// configuration: the resolution and resolver decisions its netmap would
/// carry. A Node that is not enabled executes nothing and has none. Without
/// a source (a network with no Node yet), those of a Node that would execute
/// `profile` and resolve locally, as every first Node without an exit does.
pub fn profile_resolver_plan_gaps(
    cfg: &NetworkConfiguration,
    sets: &rules::RuleSets,
    profile: &str,
    source: Option<NodeId>,
) -> Result<Vec<ResolverPlanGap>, CompileError> {
    let exits = matching::Exits::Full(cfg);
    let through_exit = match source {
        Some(node) if !cfg.nodes.get(&node).is_some_and(|node| node.enabled) => {
            return Ok(Vec::new());
        }
        Some(node) => resolution(cfg, exits, sets, profile, node)? != DnsResolution::Local,
        None => false,
    };
    // The partition decides resolvers only; skip it when none is judged.
    let judged = cfg
        .dns
        .servers
        .values()
        .any(|server| !through_exit || server.use_with_exit);
    let resolvers = if judged {
        let partition =
            capture::partition(cfg, exits, sets, profile, source).map(std::sync::Arc::new);
        partition
            .is_ok()
            .then(|| netmap::local_resolvers(cfg, true, &partition))
    } else {
        Some(BTreeMap::new())
    };
    Ok(resolver_plan_gaps(
        &cfg.dns,
        through_exit,
        resolvers.as_ref(),
    ))
}
