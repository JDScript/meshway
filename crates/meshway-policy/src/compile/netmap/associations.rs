//! Trusted DNS context in local decisions. An association is the address set
//! of one name a source's exit resolver answered and the Server signed; it
//! lets a name rule decide those addresses for that source. It is never an
//! Access grant by itself: services still come from Access rules.
use super::*;
use crate::compile::rules::{self, Destination, Membership};
use ipnet::IpNet;
use meshway_configuration::netmap::{AssociationRecordType, DnsAssociation, DnsAssociationOverlay};
use std::net::IpAddr;

/// Associations usable now, indexed by source and address. Built from a
/// verified overlay; entries that ended are left out by the builder, so the
/// value itself carries no clock.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Associations {
    entries: Vec<DnsAssociation>,
    by_address: BTreeMap<(NodeId, IpAddr), Vec<usize>>,
}

impl Associations {
    /// The entries of `overlay` still live at `now_ms`.
    pub fn new(overlay: Option<&DnsAssociationOverlay>, now_ms: u64) -> Self {
        let entries: Vec<DnsAssociation> = overlay
            .map(|overlay| {
                overlay
                    .associations
                    .iter()
                    .filter(|entry| entry.expires_at_ms > now_ms)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let mut by_address: BTreeMap<(NodeId, IpAddr), Vec<usize>> = BTreeMap::new();
        for (index, entry) in entries.iter().enumerate() {
            for address in &entry.addresses {
                by_address
                    .entry((entry.source, address.0.to_canonical()))
                    .or_default()
                    .push(index);
            }
        }
        Self {
            entries,
            by_address,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[DnsAssociation] {
        &self.entries
    }

    /// The earliest end of an entry, when the usable set next shrinks.
    pub fn next_expiry(&self) -> Option<u64> {
        self.entries.iter().map(|entry| entry.expires_at_ms).min()
    }

    /// Names associated with `address` for `source` under its selection `generation`.
    pub fn names(&self, source: NodeId, generation: Digest, address: IpAddr) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .by_address
            .get(&(source, address.to_canonical()))
            .into_iter()
            .flatten()
            .map(|index| &self.entries[*index])
            .filter(|entry| entry.generation == generation)
            .map(|entry| entry.name.as_str())
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// The association of (`source`, `name`, `record_type`) naming every one
    /// of `addresses`, if any.
    pub fn covering(
        &self,
        source: NodeId,
        name: &str,
        record_type: AssociationRecordType,
        addresses: &[IpAddr],
    ) -> Option<&DnsAssociation> {
        self.entries.iter().find(|entry| {
            entry.source == source
                && entry.name == name
                && entry.record_type == record_type
                && addresses.iter().all(|address| entry.contains(*address))
        })
    }

    /// Distinct associated addresses of `source` under `generation`.
    fn addresses(&self, source: NodeId, generation: Digest) -> Vec<IpAddr> {
        self.by_address
            .iter()
            .filter(|((node, _), indexes)| {
                *node == source
                    && indexes
                        .iter()
                        .any(|index| self.entries[*index].generation == generation)
            })
            .map(|((_, address), _)| *address)
            .collect()
    }
}

/// Whether an answered address may enter an association: a public unicast
/// address. Private, Mesh, link-local and special answers never do, so a
/// name cannot steer a source into a LAN through its rules.
pub fn associable(address: IpAddr) -> bool {
    crate::is_internet_address(address.to_canonical())
}

/// Capture group of one associated address on the source.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    Managed,
    Direct,
    Unmanaged,
    Sink,
    Uncaptured,
}

/// `net` without the single host `host` (which it contains), as disjoint prefixes.
fn exclude(net: IpNet, host: IpAddr) -> Vec<IpNet> {
    let mut out = Vec::new();
    let mut current = net;
    let max = match host {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    };
    while current.prefix_len() < max {
        let Ok(children) = current.subnets(current.prefix_len() + 1) else {
            break;
        };
        let mut next = None;
        for child in children {
            if child.contains(&host) {
                next = Some(child);
            } else {
                out.push(child);
            }
        }
        match next {
            Some(child) => current = child,
            None => break,
        }
    }
    out
}

/// Effective rules of one source in first-match order, with their own sets.
type EffectiveRules<'a> = Vec<(&'a EgressRule, Option<&'a meshway_rules::RuleSet>)>;

fn host(address: IpAddr) -> IpNet {
    IpNet::from(address)
}

impl LocalCompiler {
    /// The rules and fallback that decide external destinations of `source`,
    /// each rule with its own lists as one set.
    fn effective_rules(
        &self,
        source: NodeId,
    ) -> Result<(EffectiveRules<'_>, RouteAction), CompileError> {
        let cfg = &self.configuration;
        let projection = &self.netmap.projection;
        let mut profile = effective_profile(
            cfg,
            &projection.subjects,
            &projection.selected_profiles,
            source,
        )?;
        let mut rules: EffectiveRules<'_> = Vec::new();
        let own = self.matchers.of(None)?;
        for (index, rule) in cfg.routing.egress.rules.iter().enumerate() {
            rules.push((
                rule,
                own.get(index).ok_or(CompileError::InvalidInput)?.as_deref(),
            ));
        }
        if let matching::EgressProfile::Auto(auto) = matching::resolve_profile(cfg, &profile)? {
            if auto.fallback == "auto" {
                return Err(CompileError::InvalidInput);
            }
            profile = auto.fallback.clone();
        }
        let fallback = match matching::resolve_profile(cfg, &profile)? {
            matching::EgressProfile::Mesh => RouteAction::Unmanaged,
            matching::EgressProfile::Global(node) => RouteAction::Exit {
                nodes: vec![format!("node:{node}")],
                via: vec![],
            },
            matching::EgressProfile::Custom(custom) => {
                let own = self.matchers.of(Some(&profile))?;
                for (index, rule) in custom.rules.iter().enumerate() {
                    rules.push((
                        rule,
                        own.get(index).ok_or(CompileError::InvalidInput)?.as_deref(),
                    ));
                }
                custom.fallback.clone()
            }
            matching::EgressProfile::Auto(_) => return Err(CompileError::InvalidInput),
        };
        Ok((rules, fallback))
    }

    /// First match of `address` known as each of `names`: a rule decides when
    /// it matches the address or the name. Every name must reach the same
    /// action; `None` when they do not (the address is not executable).
    pub(super) fn associated_action(
        &self,
        source: NodeId,
        address: IpAddr,
        names: &[&str],
    ) -> Result<Option<RouteAction>, CompileError> {
        let (rules, fallback) = self.effective_rules(source)?;
        let mut decided: Option<RouteAction> = None;
        for name in names {
            let mut action = fallback.clone();
            for (rule, own) in &rules {
                let hit = |destination| -> Result<bool, CompileError> {
                    Ok(matches!(
                        rules::judge(*own, &rule.matcher, &self.rule_sets, destination)?,
                        Membership::Hit(_)
                    ))
                };
                if hit(Destination::Address(address))? || hit(Destination::Name(name))? {
                    action = rule.action.clone().into();
                    break;
                }
            }
            match &decided {
                Some(previous) if *previous != action => return Ok(None),
                _ => decided = Some(action),
            }
        }
        Ok(decided)
    }

    /// Services of `source` toward an associated `address`: external grants
    /// covering the address and `domain:` grants covering one of its names.
    pub(super) fn associated_services(
        &self,
        source: NodeId,
        address: IpAddr,
        names: &[&str],
    ) -> ServiceSet {
        let cfg = &self.configuration;
        let subjects = &self.netmap.projection.subjects;
        ServiceSet::from_grants(
            cfg.access
                .iter()
                .filter(|rule| matching::selectors(cfg, subjects, &rule.src, source))
                .filter(|rule| {
                    rule.dst.iter().any(|selector| {
                        if let Some(suffix) = selector.strip_prefix("domain:") {
                            names
                                .iter()
                                .any(|name| crate::domain_suffix_matches(name, suffix))
                        } else if selector == "autogroup:internet" {
                            crate::is_internet_address(address)
                        } else {
                            selector
                                .parse::<IpNet>()
                                .is_ok_and(|net| net.addr().is_ipv4() && net.contains(&address))
                        }
                    })
                })
                .flat_map(|rule| crate::access::token_services(&rule.ip)),
        )
    }

    /// Whether some effective rule of `origin` exits through exactly
    /// (`nodes`, `via`) by name, with an Access grant that a name can meet.
    pub(super) fn name_exit(
        &self,
        origin: NodeId,
        nodes: &[NodeId],
        via: &[NodeId],
    ) -> Result<bool, CompileError> {
        let cfg = &self.configuration;
        let subjects = &self.netmap.projection.subjects;
        let granted = cfg.access.iter().any(|rule| {
            matching::selectors(cfg, subjects, &rule.src, origin)
                && rule
                    .dst
                    .iter()
                    .any(|d| d == "autogroup:internet" || d.starts_with("domain:"))
                && crate::access::token_services(&rule.ip).next().is_some()
        });
        if !granted {
            return Ok(false);
        }
        let (rules, _) = self.effective_rules(origin)?;
        for (rule, own) in rules {
            let RuleAction::Exit {
                nodes: exits,
                via: waypoints,
            } = &rule.action
            else {
                continue;
            };
            let parsed = |selectors: &[String]| {
                selectors
                    .iter()
                    .map(|s| matching::node_selector(s))
                    .collect::<Result<Vec<_>, _>>()
            };
            if parsed(exits)? != nodes || parsed(waypoints)? != via {
                continue;
            }
            let mut names = own.is_some_and(|set| {
                let counts = set.counts();
                counts.exact + counts.suffix + counts.subdomain + counts.keyword > 0
            });
            for key in &rule.matcher.rule_sets {
                let set = self
                    .rule_sets
                    .get(key)
                    .ok_or(CompileError::UnavailableDependency)?;
                let counts = set.counts();
                names |= counts.exact + counts.suffix + counts.subdomain + counts.keyword > 0;
            }
            if names {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `plan` with every associated address of `source` captured as its
    /// association decides it: exit (and conflicting) addresses are managed
    /// (IPv6: sunk) so they reach the packet owner, direct and unmanaged ones
    /// are cut out of managed ranges. Routes and the selection identity stay
    /// the signed partition; only the capture lists change. A result above
    /// [`super::super::MAX_CAPTURE_PREFIXES`] fails closed like the partition
    /// itself: full IPv4 capture, IPv6 sink, no exception, `routingLimit`.
    pub fn associated_egress(
        &self,
        source: NodeId,
        plan: &EgressPlan,
        associations: &Associations,
    ) -> Result<EgressPlan, CompileError> {
        if plan.capture.unavailable.is_some() || associations.is_empty() {
            return Ok(plan.clone());
        }
        let generation = plan.capture.generation;
        let parse = |list: &[String]| -> Result<Vec<IpNet>, CompileError> {
            list.iter()
                .map(|text| text.parse().map_err(|_| CompileError::InvalidInput))
                .collect()
        };
        let capture = &plan.capture;
        let mut lists = [
            parse(&capture.managed_ipv4)?,
            parse(&capture.direct_ipv4)?,
            parse(&capture.unmanaged_ipv4)?,
            parse(&capture.sink_ipv6)?,
        ];
        let index = |group: Group| match group {
            Group::Managed => Some(0),
            Group::Direct => Some(1),
            Group::Unmanaged => Some(2),
            Group::Sink => Some(3),
            Group::Uncaptured => None,
        };
        for address in associations.addresses(source, generation) {
            let names = associations.names(source, generation, address);
            let action = self.associated_action(source, address, &names)?;
            let want = match (address, action) {
                (IpAddr::V4(_), Some(RouteAction::Direct)) => Group::Direct,
                (IpAddr::V4(_), Some(RouteAction::Unmanaged)) => Group::Unmanaged,
                (IpAddr::V4(_), _) => Group::Managed,
                (IpAddr::V6(_), Some(RouteAction::Direct | RouteAction::Unmanaged)) => {
                    Group::Uncaptured
                }
                (IpAddr::V6(_), _) => Group::Sink,
            };
            let current = lists.iter().enumerate().find_map(|(group, nets)| {
                nets.iter()
                    .position(|net| net.contains(&address))
                    .map(|position| (group, position))
            });
            if current.map(|(group, _)| group) == index(want) {
                continue;
            }
            if let Some((group, position)) = current {
                let net = lists[group].remove(position);
                lists[group].extend(exclude(net, address));
            }
            if let Some(group) = index(want) {
                lists[group].push(host(address));
            }
            if lists.iter().map(Vec::len).sum::<usize>() > super::super::MAX_CAPTURE_PREFIXES {
                let mut closed = plan.clone();
                closed.capture.managed_ipv4 = vec!["0.0.0.0/0".into()];
                closed.capture.default_capture = true;
                closed.capture.direct_ipv4.clear();
                closed.capture.unmanaged_ipv4.clear();
                closed.capture.sink_ipv6 = vec!["::/0".into()];
                closed.capture.unavailable = Some(UnavailableReason::RoutingLimit);
                return Ok(closed);
            }
        }
        let render = |nets: &mut Vec<IpNet>| {
            nets.sort();
            nets.iter().map(ToString::to_string).collect::<Vec<_>>()
        };
        let mut next = plan.clone();
        next.capture.managed_ipv4 = render(&mut lists[0]);
        next.capture.direct_ipv4 = render(&mut lists[1]);
        next.capture.unmanaged_ipv4 = render(&mut lists[2]);
        next.capture.sink_ipv6 = render(&mut lists[3]);
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excluding_a_host_leaves_a_disjoint_cover() {
        let net: IpNet = "10.0.0.0/8".parse().unwrap();
        let host: IpAddr = "10.1.2.3".parse().unwrap();
        let rest = exclude(net, host);
        assert_eq!(rest.len(), 24);
        assert!(rest.iter().all(|part| !part.contains(&host)));
        let total: u64 = rest
            .iter()
            .map(|part| 1u64 << (32 - part.prefix_len()))
            .sum();
        assert_eq!(total, (1u64 << 24) - 1);
        assert!(exclude("10.1.2.3/32".parse().unwrap(), host).is_empty());
    }
}
