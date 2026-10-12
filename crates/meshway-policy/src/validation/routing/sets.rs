use super::*;

/// The loaded content a configuration's rules are judged against.
#[derive(Clone, Copy)]
pub(super) struct Sets<'a> {
    pub(super) configuration: &'a NetworkConfiguration,
    pub(super) loaded: &'a BTreeMap<String, Arc<meshway_rules::RuleSet>>,
}

/// A matcher's destinations as validation compares them. Subdomain members
/// (only names strictly below) are kept apart; keywords cannot be compared
/// with suffix and CIDR grants.
#[derive(Default)]
pub(super) struct Expanded {
    pub(super) ips: Vec<IpNet>,
    pub(super) suffixes: Vec<String>,
    pub(super) exact: Vec<String>,
    pub(super) subdomains: Vec<String>,
    pub(super) keywords: Vec<String>,
}

impl Expanded {
    pub(super) fn has_destinations(&self) -> bool {
        !self.ips.is_empty()
            || !self.suffixes.is_empty()
            || !self.exact.is_empty()
            || !self.subdomains.is_empty()
            || !self.keywords.is_empty()
    }
    /// Every destination is inside what `domains`/`ranges` match. A subdomain
    /// member is judged as its suffix (a stronger requirement); a keyword is
    /// never covered.
    pub(super) fn covered_by(&self, ranges: &AddressSet, domains: &DomainSet) -> bool {
        self.keywords.is_empty()
            && self.ips.iter().all(|net| ranges.covers(net))
            && domains.covers_all(&self.suffixes, &self.exact)
            && self.subdomains.iter().all(|n| domains.covers_suffix(n))
    }
    fn extend(&mut self, other: Expanded) {
        self.ips.extend(other.ips);
        self.suffixes.extend(other.suffixes);
        self.exact.extend(other.exact);
        self.subdomains.extend(other.subdomains);
        self.keywords.extend(other.keywords);
    }
}

/// The matcher's own lists and every referenced set's members; `None` when a
/// referenced set has no loaded content (or its own CIDRs are invalid).
pub(super) fn expanded_match(matcher: &RouteMatch, sets: Sets<'_>) -> Option<Expanded> {
    let mut result = Expanded {
        ips: matcher
            .ip_cidr
            .iter()
            .map(|s| s.parse::<IpNet>().ok())
            .collect::<Option<_>>()?,
        suffixes: matcher.domain_suffix.clone(),
        exact: matcher.domain.clone(),
        ..Expanded::default()
    };
    for key in &matcher.rule_sets {
        if let Some(RuleSet::Inline {
            ip_cidr,
            domain_suffix,
            domain,
            domain_subdomain,
        }) = sets.configuration.rule_sets.get(key)
        {
            result.extend(Expanded {
                ips: ip_cidr
                    .iter()
                    .map(|s| s.parse::<IpNet>().ok())
                    .collect::<Option<_>>()?,
                suffixes: domain_suffix.clone(),
                exact: domain.clone(),
                subdomains: domain_subdomain.clone(),
                keywords: Vec::new(),
            });
            continue;
        }
        let set = sets.loaded.get(key)?;
        let mut members = Expanded::default();
        for entry in set.domains() {
            match entry.kind {
                meshway_rules::DomainKind::Exact => members.exact.push(entry.name),
                meshway_rules::DomainKind::Suffix => members.suffixes.push(entry.name),
                meshway_rules::DomainKind::Subdomain => members.subdomains.push(entry.name),
            }
        }
        members.keywords = set.keywords().map(str::to_owned).collect();
        for cidr in set
            .ipv4_ranges()
            .flat_map(meshway_rules::Cidr::cover_v4)
            .chain(set.ipv6_ranges().flat_map(meshway_rules::Cidr::cover_v6))
        {
            members.ips.push(cidr.to_string().parse().ok()?);
        }
        result.extend(members);
    }
    Some(result)
}

/// The identity of one referenced set's content: its inline definition or the
/// digest of its loaded artifact.
pub(super) fn content_of(
    key: &str,
    sets: Sets<'_>,
) -> Option<Result<RuleSet, meshway_rules::Digest>> {
    match sets.configuration.rule_sets.get(key) {
        Some(inline @ RuleSet::Inline { .. }) => Some(Ok(inline.clone())),
        _ => sets.loaded.get(key).map(|set| Err(set.digest())),
    }
}
