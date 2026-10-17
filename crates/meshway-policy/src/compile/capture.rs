//! First-match packet ownership as ordered, disjoint address ranges: the one
//! partition the Server validates and previews and every Node installs.
//!
//! Each rule takes from the not yet matched addresses the ranges its own
//! CIDRs and referenced sets cover; the fallback takes the rest. The result
//! covers IPv4 and IPv6 completely; adjacent ranges never share an action.
use super::{matching::EgressProfile, *};
use meshway_configuration::{evaluation::CaptureSummary, netmap::EgressAction};
use meshway_model::{Digest, exit::ExitFamily};
use meshway_rules::ranges::{self as intervals, Address};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    ops::RangeInclusive,
};

/// Interval elements of the capture sets (managed IPv4, managed IPv6 and
/// sunk IPv6 together) a Node installs. One nft batch and its readback stay
/// below 1.6 MB and 0.07 s (measured, see Routing · node-side limits).
pub const MAX_CAPTURE_RANGES: usize = 16_384;
/// Routes of the private capture tables: the CIDR covers of the managed IPv4
/// and IPv6 ranges together.
pub const MAX_CAPTURE_ROUTES: usize = 32_768;
/// Range operations of one partition computation, bounding its CPU time.
pub const MAX_WORK: usize = 1 << 24;

type Spans<T> = Vec<(T, T)>;

/// Split normalized `remaining` by normalized `cut` into (inside, outside).
fn split<T: Address>(
    remaining: &[(T, T)],
    cut: &[(T, T)],
    work: &mut usize,
) -> Result<(Spans<T>, Spans<T>), UnavailableReason> {
    *work += remaining.len() + cut.len();
    if *work > MAX_WORK {
        return Err(UnavailableReason::RoutingLimit);
    }
    let (mut hit, mut rest) = (Vec::new(), Vec::new());
    let mut cuts = cut.iter().peekable();
    for &(start, end) in remaining {
        let mut at = Some(start);
        while let Some(position) = at.filter(|p| *p <= end) {
            while cuts.next_if(|c| c.1 < position).is_some() {}
            match cuts.peek() {
                Some(&&(cs, ce)) if cs <= end => {
                    if cs > position {
                        rest.push((position, cs.previous().expect("cs > position")));
                    }
                    let to = ce.min(end);
                    hit.push((position.max(cs), to));
                    at = to.next();
                }
                _ => {
                    rest.push((position, end));
                    at = None;
                }
            }
        }
    }
    Ok((hit, rest))
}

/// A source's partition, or why there is none.
pub type Partition = Result<std::sync::Arc<EgressPartition>, UnavailableReason>;

/// One source's complete egress partition. It decides ownership only; Access
/// still authorizes every flow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EgressPartition {
    ipv4: Vec<(u32, u32, usize)>,
    ipv6: Vec<(u128, u128, usize)>,
    actions: Vec<EgressAction>,
    /// Per action: an exit some of whose exits carries IPv6 by configuration
    /// ([`matching::carries`]). An IPv6 exit range of another is sunk.
    carries_ipv6: Vec<bool>,
    /// The fallback is an exit and keeps some IPv4 range.
    default_capture: bool,
}

fn index(actions: &mut Vec<EgressAction>, action: EgressAction) -> usize {
    actions
        .iter()
        .position(|a| *a == action)
        .unwrap_or_else(|| {
            actions.push(action);
            actions.len() - 1
        })
}

fn assemble<T: Address>(mut pieces: Vec<(T, T, usize)>) -> Vec<(T, T, usize)> {
    pieces.sort_unstable();
    let mut out: Vec<(T, T, usize)> = Vec::with_capacity(pieces.len());
    for piece in pieces {
        match out.last_mut() {
            Some(last) if last.2 == piece.2 && last.1.next() == Some(piece.0) => last.1 = piece.1,
            _ => out.push(piece),
        }
    }
    out
}

fn find<T: Address>(ranges: &[(T, T, usize)], address: T) -> Option<usize> {
    let i = ranges.partition_point(|r| r.0 <= address);
    ranges
        .get(i.checked_sub(1)?)
        .filter(|r| r.1 >= address)
        .map(|r| r.2)
}

impl EgressPartition {
    /// The action of the unique range holding `address`.
    pub fn action(&self, address: IpAddr) -> &EgressAction {
        let index = match address.to_canonical() {
            IpAddr::V4(a) => find(&self.ipv4, u32::from(a)),
            IpAddr::V6(a) => find(&self.ipv6, u128::from(a)),
        };
        // Both families are covered completely.
        &self.actions[index.expect("complete partition")]
    }

    pub fn ipv4(&self) -> impl Iterator<Item = (RangeInclusive<Ipv4Addr>, &EgressAction)> {
        self.ipv4
            .iter()
            .map(|(s, e, a)| (Ipv4Addr::from(*s)..=Ipv4Addr::from(*e), &self.actions[*a]))
    }

    pub fn ipv6(&self) -> impl Iterator<Item = (RangeInclusive<Ipv6Addr>, &EgressAction)> {
        self.ipv6
            .iter()
            .map(|(s, e, a)| (Ipv6Addr::from(*s)..=Ipv6Addr::from(*e), &self.actions[*a]))
    }

    /// Ranges of one family whose action `keep` selects, adjacent ones merged.
    fn selected<T: Address>(
        &self,
        ranges: &[(T, T, usize)],
        keep: impl Fn(usize) -> bool,
    ) -> Vec<(T, T)> {
        intervals::merge(
            ranges
                .iter()
                .filter(|r| matches!(self.actions[r.2], EgressAction::Exit { .. }) && keep(r.2))
                .map(|r| (r.0, r.1))
                .collect(),
        )
    }

    /// Managed IPv4, managed IPv6 and sunk IPv6 ranges for a source that
    /// has (`ipv6`) or lacks a Mesh IPv6 address: without one, every IPv6
    /// exit range is sunk.
    fn ranges(&self, ipv6: bool) -> (Spans<u32>, Spans<u128>, Spans<u128>) {
        let managed4 = self.selected(&self.ipv4, |_| true);
        let carried = |id: usize| ipv6 && self.carries_ipv6[id];
        (
            managed4,
            self.selected(&self.ipv6, carried),
            self.selected(&self.ipv6, |id| !carried(id)),
        )
    }

    /// What a source with or without a Mesh IPv6 address (`ipv6`) installs
    /// for this partition, as counted against the node-side limits.
    pub fn summary(&self, ipv6: bool) -> CaptureSummary {
        let (managed4, managed6, sink6) = self.ranges(ipv6);
        counts(&managed4, &managed6, sink6.len(), None)
    }

    fn within_limits(&self) -> bool {
        // Both forms count the same ranges; with an IPv6 source managed
        // IPv6 also has routes.
        within(&self.summary(true)) && within(&self.summary(false))
    }

    /// The capture boundary of this partition for a source with or without
    /// a Mesh IPv6 address (`ipv6`).
    pub fn capture(&self, generation: Digest, ipv6: bool) -> EgressCapturePlan {
        let (managed4, managed6, sink6) = self.ranges(ipv6);
        let managed_ipv4: Vec<_> = managed4
            .into_iter()
            .map(|(s, e)| (Ipv4Addr::from(s), Ipv4Addr::from(e)))
            .collect();
        let six = |ranges: Spans<u128>| {
            ranges
                .into_iter()
                .map(|(s, e)| (Ipv6Addr::from(s), Ipv6Addr::from(e)))
                .collect()
        };
        EgressCapturePlan {
            generation,
            default_capture: self.default_capture
                || managed_ipv4 == [(Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST)],
            managed_ipv4,
            managed_ipv6: six(managed6),
            sink_ipv6: six(sink6),
            unavailable: None,
        }
    }
}

fn counts(
    managed4: &[(u32, u32)],
    managed6: &[(u128, u128)],
    sink6: usize,
    unavailable: Option<UnavailableReason>,
) -> CaptureSummary {
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    CaptureSummary {
        managed_ipv4: count(managed4.len()),
        managed_ipv6: count(managed6.len()),
        sink_ipv6: count(sink6),
        routes: count(
            managed4
                .iter()
                .map(|(s, e)| intervals::cover(*s, *e).count())
                .sum::<usize>()
                + managed6
                    .iter()
                    .map(|(s, e)| intervals::cover(*s, *e).count())
                    .sum::<usize>(),
        ),
        unavailable,
    }
}

fn within(summary: &CaptureSummary) -> bool {
    (summary.managed_ipv4 as usize + summary.managed_ipv6 as usize + summary.sink_ipv6 as usize)
        <= MAX_CAPTURE_RANGES
        && summary.routes as usize <= MAX_CAPTURE_ROUTES
}

/// How one associated address is captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assignment {
    /// Handed to the packet owner (managed IPv4 or IPv6).
    Managed,
    /// IPv6 only: rejected locally, never handed to the owner.
    Sunk,
    /// Released to the host.
    Released,
}

fn assign_range<T: Address>(ranges: &mut Vec<(T, T)>, address: T, captured: bool) {
    let mut work = 0;
    let (_, rest) = split(ranges, &[(address, address)], &mut work).expect("one host");
    let mut next = rest;
    if captured {
        next.push((address, address));
    }
    *ranges = intervals::merge(next);
}

fn six<T: Copy>(ranges: &[(T, T)], into: impl Fn(T) -> u128) -> Vec<(u128, u128)> {
    ranges.iter().map(|(s, e)| (into(*s), into(*e))).collect()
}

fn back(ranges: Vec<(u128, u128)>) -> Vec<(Ipv6Addr, Ipv6Addr)> {
    ranges
        .into_iter()
        .map(|(s, e)| (Ipv6Addr::from(s), Ipv6Addr::from(e)))
        .collect()
}

/// `plan` with `address` assigned as `how`. Used for the addresses of
/// trusted DNS associations and of live exit flows.
pub fn assign(plan: &mut EgressCapturePlan, address: IpAddr, how: Assignment) {
    match address.to_canonical() {
        IpAddr::V4(a) => {
            let mut ranges: Vec<(u32, u32)> = plan
                .managed_ipv4
                .iter()
                .map(|(s, e)| (u32::from(*s), u32::from(*e)))
                .collect();
            assign_range(&mut ranges, u32::from(a), how != Assignment::Released);
            plan.managed_ipv4 = ranges
                .into_iter()
                .map(|(s, e)| (Ipv4Addr::from(s), Ipv4Addr::from(e)))
                .collect();
        }
        IpAddr::V6(a) => {
            let mut managed = six(&plan.managed_ipv6, u128::from);
            let mut sunk = six(&plan.sink_ipv6, u128::from);
            assign_range(&mut managed, u128::from(a), how == Assignment::Managed);
            assign_range(&mut sunk, u128::from(a), how == Assignment::Sunk);
            plan.managed_ipv6 = back(managed);
            plan.sink_ipv6 = back(sunk);
        }
    }
}

/// What a Node installs for `plan`, counted as [`EgressPartition::summary`]
/// counts a partition.
pub fn plan_summary(plan: &EgressCapturePlan) -> CaptureSummary {
    let managed4: Vec<(u32, u32)> = plan
        .managed_ipv4
        .iter()
        .map(|(s, e)| (u32::from(*s), u32::from(*e)))
        .collect();
    counts(
        &managed4,
        &six(&plan.managed_ipv6, u128::from),
        plan.sink_ipv6.len(),
        plan.unavailable,
    )
}

/// `next` with every address of `keep` still managed that `previous`
/// managed and `next` no longer manages: the destinations of flows
/// established through the previous boundary keep reaching the packet
/// owner, which keeps their original decision, until those flows end.
/// Beyond the node-side limits the result fails closed like a partition
/// does. Returned with the retained addresses and the summary of the result.
pub fn retained_capture(
    previous: &EgressCapturePlan,
    next: &EgressCapturePlan,
    keep: impl IntoIterator<Item = IpAddr>,
) -> (EgressCapturePlan, Vec<IpAddr>, CaptureSummary) {
    let managed = |plan: &EgressCapturePlan, address: IpAddr| match address {
        IpAddr::V4(address) => plan
            .managed_ipv4
            .iter()
            .any(|(start, end)| *start <= address && address <= *end),
        IpAddr::V6(address) => plan
            .managed_ipv6
            .iter()
            .any(|(start, end)| *start <= address && address <= *end),
    };
    let mut kept: Vec<IpAddr> = keep
        .into_iter()
        .map(|address| address.to_canonical())
        .filter(|address| managed(previous, *address) && !managed(next, *address))
        .collect();
    kept.sort_unstable();
    kept.dedup();
    if kept.is_empty() || next.unavailable.is_some() {
        return (next.clone(), Vec::new(), plan_summary(next));
    }
    let mut plan = next.clone();
    for address in &kept {
        assign(&mut plan, *address, Assignment::Managed);
    }
    let summary = plan_summary(&plan);
    if !installable(&plan) {
        return (
            unavailable_capture(next.generation, UnavailableReason::RoutingLimit),
            kept,
            CaptureSummary {
                unavailable: Some(UnavailableReason::RoutingLimit),
                ..summary
            },
        );
    }
    (plan, kept, summary)
}

/// Whether a capture plan fits what a Node installs.
pub fn installable(plan: &EgressCapturePlan) -> bool {
    within(&plan_summary(plan))
}

/// The fail-closed boundary of an unavailable partition: full IPv4 capture,
/// full IPv6 sink, no direct or unmanaged exception.
pub fn unavailable_capture(generation: Digest, reason: UnavailableReason) -> EgressCapturePlan {
    EgressCapturePlan {
        generation,
        managed_ipv4: vec![(Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST)],
        default_capture: true,
        managed_ipv6: Vec::new(),
        sink_ipv6: vec![(Ipv6Addr::UNSPECIFIED, Ipv6Addr::from(u128::MAX))],
        unavailable: Some(reason),
    }
}

/// The ordered rules in effect for a profile (public first) with their JSON
/// Pointers, and the fallback.
pub(super) struct Selection<'a> {
    pub rules: Vec<(String, &'a EgressRule)>,
    pub fallback: RouteAction,
}

/// `source` resolves `auto` ([`matching::concrete_profile`]).
pub(super) fn selection<'a>(
    cfg: &'a NetworkConfiguration,
    profile: &str,
    source: Option<NodeId>,
) -> Result<Selection<'a>, CompileError> {
    let (resolved, name, _) = matching::concrete_profile(cfg, profile, source)?;
    let resolved = (resolved, name.as_str());
    let mut rules: Vec<_> = cfg
        .routing
        .egress
        .rules
        .iter()
        .enumerate()
        .map(|(i, rule)| (format!("/routing/egress/rules/{i}"), rule))
        .collect();
    let fallback = match resolved {
        (EgressProfile::Mesh, _) => RouteAction::Unmanaged,
        (EgressProfile::Global(node), _) => RouteAction::Exit {
            nodes: vec![format!("node:{node}")],
            via: vec![],
        },
        (EgressProfile::Custom(custom), name) => {
            let prefix = format!(
                "/routing/egress/profiles/{}/rules",
                name.replace('~', "~0").replace('/', "~1")
            );
            rules.extend(
                custom
                    .rules
                    .iter()
                    .enumerate()
                    .map(|(i, rule)| (format!("{prefix}/{i}"), rule)),
            );
            custom.fallback.clone()
        }
        (EgressProfile::Auto(_), _) => return Err(CompileError::InvalidInput),
    };
    Ok(Selection { rules, fallback })
}

/// Whether some action of `profile`'s selection names `source` as an exit,
/// i.e. whether `source` executes a partition other than the profile's.
pub(super) fn names_exit(
    cfg: &NetworkConfiguration,
    exits: matching::Exits<'_>,
    profile: &str,
    source: NodeId,
) -> bool {
    selection(cfg, profile, Some(source)).is_ok_and(|Selection { rules, fallback }| {
        rules
            .iter()
            .map(|(_, rule)| rule.action.clone().into())
            .chain(std::iter::once(fallback))
            .any(|route| {
                [ExitFamily::Ipv4, ExitFamily::Ipv6]
                    .into_iter()
                    .any(|family| {
                        matching::executed(exits, route.clone(), source, family)
                            .map_or(true, |executed| executed != route)
                    })
            })
    })
}

pub(super) fn action(route: &RouteAction) -> Result<EgressAction, CompileError> {
    let nodes = |selectors: &[String]| {
        selectors
            .iter()
            .map(|s| matching::node_selector(s))
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(match route {
        RouteAction::Exit { nodes: exits, via } => EgressAction::Exit {
            nodes: nodes(exits)?,
            via: nodes(via)?,
        },
        RouteAction::Direct => EgressAction::Direct,
        RouteAction::Unmanaged => EgressAction::Unmanaged,
    })
}

fn unavailable(error: CompileError) -> UnavailableReason {
    match error {
        CompileError::Budget => UnavailableReason::RoutingLimit,
        CompileError::UnavailableDependency => UnavailableReason::MissingDependency,
        _ => UnavailableReason::EgressConflict,
    }
}

/// Rule actions as `source` executes them toward `family`
/// ([`matching::executed`]); without a source (a profile-level preview) as
/// written.
fn executed(
    exits: matching::Exits<'_>,
    route: RouteAction,
    source: Option<NodeId>,
    family: ExitFamily,
) -> Result<RouteAction, CompileError> {
    match source {
        Some(source) => matching::executed(exits, route, source, family),
        None => Ok(route),
    }
}

/// The partition of `profile` that `source` executes, without the node-side
/// limits; `Err` names why no partition exists. A rule referencing a set not
/// in `sets` is a missing dependency, never an empty set.
pub fn compute(
    cfg: &NetworkConfiguration,
    exits: matching::Exits<'_>,
    sets: &rules::RuleSets,
    profile: &str,
    source: Option<NodeId>,
) -> Result<EgressPartition, UnavailableReason> {
    let Selection { rules, fallback } = selection(cfg, profile, source).map_err(unavailable)?;
    let fallback4 =
        executed(exits, fallback.clone(), source, ExitFamily::Ipv4).map_err(unavailable)?;
    let fallback6 = executed(exits, fallback, source, ExitFamily::Ipv6).map_err(unavailable)?;
    let mut actions = Vec::new();
    let mut remaining4 = vec![(0u32, u32::MAX)];
    let mut remaining6 = vec![(0u128, u128::MAX)];
    let (mut out4, mut out6) = (Vec::new(), Vec::new());
    let mut work = 0;
    for (_, rule) in rules {
        if remaining4.is_empty() && remaining6.is_empty() {
            break;
        }
        // A rule's names own no address: they decide only the addresses of
        // a trusted association (`netmap::Associations`).
        let rules::Ranges { v4, v6 } = rules::ranges(&rule.matcher, sets).map_err(unavailable)?;
        let mut id = |family| -> Result<usize, UnavailableReason> {
            Ok(index(
                &mut actions,
                action(
                    &executed(exits, rule.action.clone().into(), source, family)
                        .map_err(unavailable)?,
                )
                .map_err(unavailable)?,
            ))
        };
        let (id4, id6) = (id(ExitFamily::Ipv4)?, id(ExitFamily::Ipv6)?);
        let (hit, rest) = split(&remaining4, &intervals::merge(v4), &mut work)?;
        out4.extend(hit.into_iter().map(|(s, e)| (s, e, id4)));
        remaining4 = rest;
        let (hit, rest) = split(&remaining6, &intervals::merge(v6), &mut work)?;
        out6.extend(hit.into_iter().map(|(s, e)| (s, e, id6)));
        remaining6 = rest;
    }
    let default_capture = matches!(fallback4, RouteAction::Exit { .. }) && !remaining4.is_empty();
    let id = index(&mut actions, action(&fallback4).map_err(unavailable)?);
    out4.extend(remaining4.into_iter().map(|(s, e)| (s, e, id)));
    let id = index(&mut actions, action(&fallback6).map_err(unavailable)?);
    out6.extend(remaining6.into_iter().map(|(s, e)| (s, e, id)));
    // A resolver kept under an exit (`useWithExit`) is reached from this
    // Node itself: its address is a direct exception of any exit range.
    let resolvers = kept_resolvers(cfg);
    if !resolvers.is_empty() {
        let exits: Vec<bool> = actions
            .iter()
            .map(|action| matches!(action, EgressAction::Exit { .. }))
            .collect();
        let direct = index(&mut actions, EgressAction::Direct);
        let exit = |id: usize| exits.get(id).copied().unwrap_or(false);
        for address in resolvers {
            match address {
                IpAddr::V4(a) => except(&mut out4, u32::from(a), direct, exit),
                IpAddr::V6(a) => except(&mut out6, u128::from(a), direct, exit),
            }
        }
    }
    let carries_ipv6 = actions
        .iter()
        .map(|action| match action {
            EgressAction::Exit { nodes, .. } => matching::carries(exits, nodes, ExitFamily::Ipv6),
            _ => false,
        })
        .collect();
    Ok(EgressPartition {
        ipv4: assemble(out4),
        ipv6: assemble(out6),
        actions,
        carries_ipv6,
        default_capture,
    })
}

/// Addresses of the resolvers kept under an exit (`useWithExit`), outside
/// the Mesh: a Mesh address is never an egress destination.
pub(super) fn kept_resolvers(cfg: &NetworkConfiguration) -> Vec<IpAddr> {
    cfg.dns
        .servers
        .values()
        .filter(|server| server.use_with_exit)
        .map(|server| server.server.to_canonical())
        .filter(|address| {
            !address.is_unspecified()
                && !address.is_multicast()
                && !cfg
                    .address_pools
                    .ipv4
                    .iter()
                    .chain(&cfg.address_pools.ipv6)
                    .any(|pool| {
                        pool.parse::<ipnet::IpNet>()
                            .is_ok_and(|pool| pool.contains(address))
                    })
        })
        .collect()
}

/// The keys of the kept resolvers (`useWithExit`) that `profile`'s rules
/// would send through an exit, i.e. that are its direct exceptions. A
/// selection or rule that cannot be judged excepts nothing here; its
/// partition fails closed anyway.
pub fn excepted_resolvers(
    cfg: &NetworkConfiguration,
    sets: &rules::RuleSets,
    profile: &str,
    source: NodeId,
) -> Vec<String> {
    let Ok(Selection { rules, fallback }) = selection(cfg, profile, Some(source)) else {
        return Vec::new();
    };
    let kept = kept_resolvers(cfg);
    cfg.dns
        .servers
        .iter()
        .filter(|(_, server)| kept.contains(&server.server.to_canonical()))
        .filter(|(_, server)| {
            let address = server.server.to_canonical();
            let mut chosen = None;
            for (_, rule) in &rules {
                let Ok(ranges) = rules::ranges(&rule.matcher, sets) else {
                    return false;
                };
                let hit = match address {
                    IpAddr::V4(a) => ranges
                        .v4
                        .iter()
                        .any(|(s, e)| (*s..=*e).contains(&u32::from(a))),
                    IpAddr::V6(a) => ranges
                        .v6
                        .iter()
                        .any(|(s, e)| (*s..=*e).contains(&u128::from(a))),
                };
                if hit {
                    chosen = Some(RouteAction::from(rule.action.clone()));
                    break;
                }
            }
            matches!(
                chosen.unwrap_or_else(|| fallback.clone()),
                RouteAction::Exit { .. }
            )
        })
        .map(|(key, _)| key.clone())
        .collect()
}

/// `address` taken out of the range holding it, as `direct`, when that
/// range's action is an exit.
fn except<T: Address>(
    pieces: &mut Vec<(T, T, usize)>,
    address: T,
    direct: usize,
    exit: impl Fn(usize) -> bool,
) {
    let Some(at) = pieces
        .iter()
        .position(|(start, end, id)| *start <= address && address <= *end && exit(*id))
    else {
        return;
    };
    let (start, end, id) = pieces.swap_remove(at);
    if let Some(before) = address.previous().filter(|before| *before >= start) {
        pieces.push((start, before, id));
    }
    if let Some(after) = address.next().filter(|after| *after <= end) {
        pieces.push((after, end, id));
    }
    pieces.push((address, address, direct));
}

/// The partition a Node installs: [`compute`] within the node-side limits.
pub fn partition(
    cfg: &NetworkConfiguration,
    exits: matching::Exits<'_>,
    sets: &rules::RuleSets,
    profile: &str,
    source: Option<NodeId>,
) -> Result<EgressPartition, UnavailableReason> {
    let partition = compute(cfg, exits, sets, profile, source)?;
    if !partition.within_limits() {
        return Err(UnavailableReason::RoutingLimit);
    }
    Ok(partition)
}

/// What `source` (any source not named as an exit when omitted) would
/// install for `profile`: the partition's counts, or the fail-closed
/// boundary's with its reason.
pub fn summary(
    cfg: &NetworkConfiguration,
    exits: matching::Exits<'_>,
    sets: &rules::RuleSets,
    profile: &str,
    source: Option<NodeId>,
) -> CaptureSummary {
    match compute(cfg, exits, sets, profile, source) {
        Ok(partition) => {
            // A previewed source has a Mesh IPv6 address when it is known
            // to; a profile-level preview assumes one.
            let ipv6 = source
                .and_then(|node| cfg.nodes.get(&node))
                .is_none_or(|node| node.addresses.ipv6.is_some());
            let summary = partition.summary(ipv6);
            if partition.within_limits() {
                summary
            } else {
                CaptureSummary {
                    unavailable: Some(UnavailableReason::RoutingLimit),
                    ..summary
                }
            }
        }
        Err(reason) => CaptureSummary {
            managed_ipv4: 1,
            managed_ipv6: 0,
            sink_ipv6: 1,
            routes: 1,
            unavailable: Some(reason),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_destinations_of_live_flows_stay_captured_until_they_end() {
        let host = |s: &str| s.parse::<Ipv4Addr>().unwrap();
        let generation = Digest::from_bytes([1; 32]);
        let plan = |ranges: &[(&str, &str)]| EgressCapturePlan {
            generation,
            managed_ipv4: ranges.iter().map(|(a, b)| (host(a), host(b))).collect(),
            default_capture: false,
            managed_ipv6: vec![],
            sink_ipv6: vec![],
            unavailable: None,
        };
        let previous = plan(&[
            ("198.51.100.0", "198.51.100.255"),
            ("203.0.113.9", "203.0.113.9"),
        ]);
        let next = plan(&[("198.51.100.0", "198.51.100.255")]);
        let keep = ["203.0.113.9", "192.0.2.1", "198.51.100.7"].map(|a| IpAddr::V4(host(a)));
        let (kept_plan, kept, summary) = retained_capture(&previous, &next, keep);
        // Only the released one: never one `previous` did not capture.
        assert_eq!(kept, [IpAddr::V4(host("203.0.113.9"))]);
        assert_eq!(kept_plan.managed_ipv4, previous.managed_ipv4);
        assert_eq!(summary.managed_ipv4, 2);
        // Without live flows the release takes effect.
        let (released, kept, _) = retained_capture(&previous, &next, []);
        assert!(kept.is_empty() && released == next);
    }

    #[test]
    fn split_partitions_by_cut() {
        let mut work = 0;
        let (hit, rest) =
            split(&[(0u32, 100)], &[(10, 20), (30, 30), (90, 200)], &mut work).unwrap();
        assert_eq!(hit, [(10, 20), (30, 30), (90, 100)]);
        assert_eq!(rest, [(0, 9), (21, 29), (31, 89)]);
        let (hit, rest) = split(&[(0u32, u32::MAX)], &[(0, u32::MAX)], &mut work).unwrap();
        assert_eq!((hit, rest), (vec![(0, u32::MAX)], vec![]));
        let (hit, rest) = split(&[(5u32, 9), (20, 30)], &[(0, 4), (31, 40)], &mut work).unwrap();
        assert_eq!((hit, rest), (vec![], vec![(5, 9), (20, 30)]));
    }
}
