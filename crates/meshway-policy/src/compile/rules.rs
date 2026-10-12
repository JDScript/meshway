//! Rule-set membership for egress first-match. Every test of a destination
//! against an egress rule goes through `meshway-rules`: the rule's own
//! `match` lists as one in-memory set, each referenced set as its artifact.
use super::*;
use meshway_configuration::evaluation::RuleSetMatch;
use meshway_rules::{Cidr, DomainHit, RuleSet, inline::Inline};
use std::{net::IpAddr, sync::Arc};

/// Loaded rule sets keyed as routing rules reference them (network keys and
/// `<provider>:<name>`). A key a rule references but this map lacks is an
/// unavailable dependency, never an empty set.
pub type RuleSets = BTreeMap<String, Arc<RuleSet>>;

/// Loaded artifacts by content digest, as a Node holds them.
pub type Artifacts = BTreeMap<meshway_rules::Digest, Arc<RuleSet>>;

/// The netmap reference of a loaded set.
pub fn reference(set: &RuleSet) -> meshway_configuration::netmap::RuleSetRef {
    meshway_configuration::netmap::RuleSetRef {
        digest: set.digest(),
        format: meshway_rules::MAJOR,
        bytes: u32::try_from(set.as_bytes().len()).unwrap_or(u32::MAX),
        entries: u32::try_from(set.counts().entries()).unwrap_or(u32::MAX),
    }
}

/// The sets of a projection whose artifact is loaded, keyed as referenced.
pub fn resolve(
    references: &BTreeMap<String, meshway_configuration::netmap::RuleSetRef>,
    artifacts: &Artifacts,
) -> RuleSets {
    references
        .iter()
        .filter_map(|(key, reference)| {
            let set = artifacts.get(&reference.digest)?;
            (set.as_bytes().len() as u64 == u64::from(reference.bytes)
                && reference.format == meshway_rules::MAJOR)
                .then(|| (key.clone(), Arc::clone(set)))
        })
        .collect()
}

/// A destination as one egress rule judges it.
#[derive(Clone, Copy)]
pub(super) enum Destination<'a> {
    Address(IpAddr),
    /// Lowercase ASCII without a trailing dot. Names are compared as bytes,
    /// so a query name with `_` still meets suffix and keyword members.
    Name(&'a str),
}

pub(super) enum Membership {
    Hit(Option<RuleSetMatch>),
    /// Only the other destination kind could match (names for an address,
    /// addresses for a name).
    ResolutionDependent,
    Miss,
}

/// The rule's own `match` lists as one set; `None` when they are all empty.
pub(super) fn own(matcher: &RouteMatch) -> Result<Option<Arc<RuleSet>>, CompileError> {
    if matcher.domain.is_empty() && matcher.domain_suffix.is_empty() && matcher.ip_cidr.is_empty() {
        return Ok(None);
    }
    RuleSet::from_inline(&Inline {
        domain: &matcher.domain,
        domain_suffix: &matcher.domain_suffix,
        domain_subdomain: &[],
        ip_cidr: &matcher.ip_cidr,
    })
    .map(|set| Some(Arc::new(set)))
    .map_err(|_| CompileError::InvalidInput)
}

/// Own sets of every egress rule of a configuration, built once per
/// configuration (one netmap on a Node, one request on the Server).
pub(super) struct Matchers {
    public: Vec<Option<Arc<RuleSet>>>,
    profiles: BTreeMap<String, Vec<Option<Arc<RuleSet>>>>,
    /// Some list did not convert: every egress decision is unavailable.
    unusable: bool,
}

impl Matchers {
    pub(super) fn new(cfg: &NetworkConfiguration) -> Result<Self, CompileError> {
        let build = |rules: &[EgressRule]| -> Result<Vec<_>, CompileError> {
            rules.iter().map(|rule| own(&rule.matcher)).collect()
        };
        Ok(Self {
            public: build(&cfg.routing.egress.rules)?,
            profiles: cfg
                .routing
                .egress
                .profiles
                .iter()
                .map(|(name, profile)| Ok((name.clone(), build(&profile.rules)?)))
                .collect::<Result<_, CompileError>>()?,
            unusable: false,
        })
    }

    /// Matchers that judge nothing, for content whose lists do not convert.
    pub(super) fn unavailable() -> Self {
        Self {
            public: Vec::new(),
            profiles: BTreeMap::new(),
            unusable: true,
        }
    }

    /// Own sets of the public rules (`None`) or of a profile's rules.
    pub(super) fn of(
        &self,
        profile: Option<&str>,
    ) -> Result<&[Option<Arc<RuleSet>>], CompileError> {
        if self.unusable {
            return Err(CompileError::UnavailableDependency);
        }
        Ok(match profile {
            None => &self.public,
            Some(name) => self.profiles.get(name).map_or(&[], Vec::as_slice),
        })
    }
}

fn has_names(set: &RuleSet) -> bool {
    let c = set.counts();
    c.exact + c.suffix + c.subdomain + c.keyword > 0
}
fn has_addresses(set: &RuleSet) -> bool {
    let c = set.counts();
    c.ipv4 + c.ipv6 > 0
}

fn entry(set: &RuleSet, destination: Destination<'_>) -> Option<String> {
    match destination {
        Destination::Address(address) => {
            if !set.match_ip(address) {
                return None;
            }
            // Cold path: the canonical prefix of the merged range holding it.
            let range = set.ip_range(address)?;
            let covering = match (range.start(), range.end()) {
                (IpAddr::V4(a), IpAddr::V4(b)) => Cidr::cover_v4(*a..=*b),
                (IpAddr::V6(a), IpAddr::V6(b)) => Cidr::cover_v6(*a..=*b),
                _ => return None,
            };
            covering
                .into_iter()
                .find(|cidr| {
                    cidr.to_string()
                        .parse::<ipnet::IpNet>()
                        .is_ok_and(|net| net.contains(&address))
                })
                .map(|cidr| cidr.to_string())
        }
        Destination::Name(name) => Some(match set.match_domain(name)? {
            DomainHit::Exact => name.to_owned(),
            DomainHit::Suffix { len } | DomainHit::Subdomain { len } => {
                name.get(name.len().checked_sub(len)?..)?.to_owned()
            }
            DomainHit::Keyword { index } => set.keyword(index)?.to_owned(),
        }),
    }
}

/// One rule's verdict: its own lists first, then each referenced set in
/// order. A referenced set that is not loaded stops the evaluation.
pub(super) fn judge(
    own: Option<&RuleSet>,
    matcher: &RouteMatch,
    sets: &RuleSets,
    destination: Destination<'_>,
) -> Result<Membership, CompileError> {
    let mut other_kind = false;
    let mut test = |key: Option<&str>, set: &RuleSet| -> Option<Membership> {
        if let Some(entry) = entry(set, destination) {
            return Some(Membership::Hit(key.map(|key| RuleSetMatch {
                key: key.to_owned(),
                entry,
            })));
        }
        other_kind |= match destination {
            Destination::Address(_) => has_names(set),
            Destination::Name(_) => has_addresses(set),
        };
        None
    };
    if let Some(set) = own
        && let Some(hit) = test(None, set)
    {
        return Ok(hit);
    }
    for key in &matcher.rule_sets {
        let set = sets.get(key).ok_or(CompileError::UnavailableDependency)?;
        if let Some(hit) = test(Some(key), set) {
            return Ok(hit);
        }
    }
    Ok(if other_kind {
        Membership::ResolutionDependent
    } else {
        Membership::Miss
    })
}

/// The first rule of `rules` selecting the destination, with rules judged
/// only by the other destination kind appended to `skipped` under `prefix`.
pub(super) fn first_match(
    rules: &[EgressRule],
    own: &[Option<Arc<RuleSet>>],
    sets: &RuleSets,
    destination: Destination<'_>,
    prefix: &str,
    skipped: &mut Vec<String>,
) -> Result<Option<(usize, Option<RuleSetMatch>)>, CompileError> {
    for (index, rule) in rules.iter().enumerate() {
        let own = own.get(index).ok_or(CompileError::InvalidInput)?.as_deref();
        match judge(own, &rule.matcher, sets, destination)? {
            Membership::Hit(rule_set) => return Ok(Some((index, rule_set))),
            Membership::ResolutionDependent => skipped.push(format!("{prefix}/{index}")),
            Membership::Miss => {}
        }
    }
    Ok(None)
}

/// Address ranges a rule partitions: its own CIDRs and every referenced
/// set's merged ranges, and whether it also names domains.
pub(super) struct Ranges {
    pub v4: Vec<(u32, u32)>,
    pub v6: Vec<(u128, u128)>,
    pub names: bool,
}

pub(super) fn ranges(matcher: &RouteMatch, sets: &RuleSets) -> Result<Ranges, CompileError> {
    let (mut v4, mut v6) = (Vec::new(), Vec::new());
    let mut names = !matcher.domain.is_empty() || !matcher.domain_suffix.is_empty();
    for text in &matcher.ip_cidr {
        match text.parse().map_err(|_| CompileError::InvalidInput)? {
            ipnet::IpNet::V4(net) => {
                v4.push((u32::from(net.network()), u32::from(net.broadcast())))
            }
            ipnet::IpNet::V6(net) => {
                v6.push((u128::from(net.network()), u128::from(net.broadcast())))
            }
        }
    }
    for key in &matcher.rule_sets {
        let set = sets.get(key).ok_or(CompileError::UnavailableDependency)?;
        names |= has_names(set);
        v4.extend(
            set.ipv4_ranges()
                .map(|r| (u32::from(*r.start()), u32::from(*r.end()))),
        );
        v6.extend(
            set.ipv6_ranges()
                .map(|r| (u128::from(*r.start()), u128::from(*r.end()))),
        );
    }
    Ok(Ranges { v4, v6, names })
}

/// The canonical spelling a query name is judged in: ASCII lowercase without
/// one trailing root dot. No DNS-name validation: a query name with `_`
/// (`_dmarc.example.com`) still meets suffix and keyword members.
pub fn query_name(name: &str) -> String {
    name.strip_suffix('.').unwrap_or(name).to_ascii_lowercase()
}
