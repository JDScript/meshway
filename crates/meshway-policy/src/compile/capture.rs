//! First-match packet ownership as ordered, disjoint address ranges: the one
//! partition the Server validates and previews and every Node installs.
//!
//! Each rule takes from the not yet matched addresses the ranges its own
//! CIDRs and referenced sets cover; the fallback takes the rest. The result
//! covers IPv4 and IPv6 completely; adjacent ranges never share an action.
use super::{matching::EgressProfile, *};
use meshway_configuration::{evaluation::CaptureSummary, netmap::EgressAction};
use meshway_model::Digest;
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    ops::RangeInclusive,
};

/// Interval elements of the capture sets (managed IPv4 plus sunk IPv6) a Node
/// installs. One nft batch and its readback stay below 1.6 MB and 0.07 s
/// (measured, see Routing · node-side limits).
pub const MAX_CAPTURE_RANGES: usize = 16_384;
/// Routes of the private capture table: the CIDR cover of the managed IPv4
/// ranges.
pub const MAX_CAPTURE_ROUTES: usize = 32_768;
/// Range operations of one partition computation, bounding its CPU time.
pub const MAX_WORK: usize = 1 << 24;

trait Address: Copy + Ord {
    fn next(self) -> Option<Self>;
    fn previous(self) -> Option<Self>;
}
impl Address for u32 {
    fn next(self) -> Option<Self> {
        self.checked_add(1)
    }
    fn previous(self) -> Option<Self> {
        self.checked_sub(1)
    }
}
impl Address for u128 {
    fn next(self) -> Option<Self> {
        self.checked_add(1)
    }
    fn previous(self) -> Option<Self> {
        self.checked_sub(1)
    }
}

/// Sorted, merged ranges; touching ranges become one.
fn normalize<T: Address>(mut ranges: Vec<(T, T)>) -> Vec<(T, T)> {
    ranges.sort_unstable();
    let mut out: Vec<(T, T)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        match out.last_mut() {
            Some(last) if last.1.next().is_none_or(|after| start <= after) => {
                last.1 = last.1.max(end);
            }
            _ => out.push((start, end)),
        }
    }
    out
}

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

/// Number of CIDR prefixes covering `[start, end]`.
fn cover(start: u32, end: u32) -> usize {
    let (mut at, end) = (u64::from(start), u64::from(end));
    let mut count = 0;
    while at <= end {
        let mut size = if at == 0 {
            1 << 32
        } else {
            1u64 << at.trailing_zeros()
        };
        while at + size - 1 > end {
            size >>= 1;
        }
        at += size;
        count += 1;
    }
    count
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

    /// Exit-owned ranges of one family, adjacent ones merged.
    fn managed<T: Address>(&self, ranges: &[(T, T, usize)]) -> Vec<(T, T)> {
        normalize(
            ranges
                .iter()
                .filter(|r| matches!(self.actions[r.2], EgressAction::Exit { .. }))
                .map(|r| (r.0, r.1))
                .collect(),
        )
    }

    /// What a Node installs for this partition, as counted against the
    /// node-side limits.
    pub fn summary(&self) -> CaptureSummary {
        let managed = self.managed(&self.ipv4);
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        CaptureSummary {
            managed_ipv4: count(managed.len()),
            sink_ipv6: count(self.managed(&self.ipv6).len()),
            routes: count(managed.iter().map(|(s, e)| cover(*s, *e)).sum()),
            unavailable: None,
        }
    }

    fn within_limits(&self) -> bool {
        let summary = self.summary();
        (summary.managed_ipv4 as usize + summary.sink_ipv6 as usize) <= MAX_CAPTURE_RANGES
            && summary.routes as usize <= MAX_CAPTURE_ROUTES
    }

    /// The capture boundary of this partition.
    pub fn capture(&self, generation: Digest) -> EgressCapturePlan {
        let managed_ipv4: Vec<_> = self
            .managed(&self.ipv4)
            .into_iter()
            .map(|(s, e)| (Ipv4Addr::from(s), Ipv4Addr::from(e)))
            .collect();
        EgressCapturePlan {
            generation,
            default_capture: self.default_capture
                || managed_ipv4 == [(Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST)],
            managed_ipv4,
            sink_ipv6: self
                .managed(&self.ipv6)
                .into_iter()
                .map(|(s, e)| (Ipv6Addr::from(s), Ipv6Addr::from(e)))
                .collect(),
            unavailable: None,
        }
    }
}

fn assign_range<T: Address>(ranges: &mut Vec<(T, T)>, address: T, captured: bool) {
    let mut work = 0;
    let (_, rest) = split(ranges, &[(address, address)], &mut work).expect("one host");
    let mut next = rest;
    if captured {
        next.push((address, address));
    }
    *ranges = normalize(next);
}

/// `plan` with `address` captured (managed IPv4, sunk IPv6) or released to
/// the host. Used for the addresses of trusted DNS associations.
pub fn assign(plan: &mut EgressCapturePlan, address: IpAddr, captured: bool) {
    match address.to_canonical() {
        IpAddr::V4(a) => {
            let mut ranges: Vec<(u32, u32)> = plan
                .managed_ipv4
                .iter()
                .map(|(s, e)| (u32::from(*s), u32::from(*e)))
                .collect();
            assign_range(&mut ranges, u32::from(a), captured);
            plan.managed_ipv4 = ranges
                .into_iter()
                .map(|(s, e)| (Ipv4Addr::from(s), Ipv4Addr::from(e)))
                .collect();
        }
        IpAddr::V6(a) => {
            let mut ranges: Vec<(u128, u128)> = plan
                .sink_ipv6
                .iter()
                .map(|(s, e)| (u128::from(*s), u128::from(*e)))
                .collect();
            assign_range(&mut ranges, u128::from(a), captured);
            plan.sink_ipv6 = ranges
                .into_iter()
                .map(|(s, e)| (Ipv6Addr::from(s), Ipv6Addr::from(e)))
                .collect();
        }
    }
}

/// What a Node installs for `plan`, counted as [`EgressPartition::summary`]
/// counts a partition.
pub fn plan_summary(plan: &EgressCapturePlan) -> CaptureSummary {
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    CaptureSummary {
        managed_ipv4: count(plan.managed_ipv4.len()),
        sink_ipv6: count(plan.sink_ipv6.len()),
        routes: count(
            plan.managed_ipv4
                .iter()
                .map(|(s, e)| cover(u32::from(*s), u32::from(*e)))
                .sum(),
        ),
        unavailable: plan.unavailable,
    }
}

/// `next` with every IPv4 address of `keep` still captured that `previous`
/// captured and `next` releases: the destinations of flows established
/// through the previous boundary keep reaching the packet owner, which keeps
/// their original decision, until those flows end. Beyond the node-side
/// limits the result fails closed like a partition does. Returned with the
/// retained addresses and the summary of the result.
pub fn retained_capture(
    previous: &EgressCapturePlan,
    next: &EgressCapturePlan,
    keep: impl IntoIterator<Item = IpAddr>,
) -> (EgressCapturePlan, Vec<Ipv4Addr>, CaptureSummary) {
    let managed = |plan: &EgressCapturePlan, address: Ipv4Addr| {
        plan.managed_ipv4
            .iter()
            .any(|(start, end)| *start <= address && address <= *end)
    };
    let mut kept: Vec<Ipv4Addr> = keep
        .into_iter()
        .filter_map(|address| match address.to_canonical() {
            IpAddr::V4(address) => Some(address),
            IpAddr::V6(_) => None,
        })
        .filter(|address| managed(previous, *address) && !managed(next, *address))
        .collect();
    kept.sort_unstable();
    kept.dedup();
    if kept.is_empty() || next.unavailable.is_some() {
        return (next.clone(), Vec::new(), plan_summary(next));
    }
    let mut plan = next.clone();
    for address in &kept {
        assign(&mut plan, IpAddr::V4(*address), true);
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
    let summary = plan_summary(plan);
    (summary.managed_ipv4 as usize + summary.sink_ipv6 as usize) <= MAX_CAPTURE_RANGES
        && summary.routes as usize <= MAX_CAPTURE_ROUTES
}

/// The fail-closed boundary of an unavailable partition: full IPv4 capture,
/// full IPv6 sink, no direct or unmanaged exception.
pub fn unavailable_capture(generation: Digest, reason: UnavailableReason) -> EgressCapturePlan {
    EgressCapturePlan {
        generation,
        managed_ipv4: vec![(Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST)],
        default_capture: true,
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

pub(super) fn selection<'a>(
    cfg: &'a NetworkConfiguration,
    profile: &str,
) -> Result<Selection<'a>, CompileError> {
    let resolved = match matching::resolve_profile(cfg, profile)? {
        EgressProfile::Auto(auto) => {
            if auto.fallback == "auto" {
                return Err(CompileError::InvalidInput);
            }
            (
                matching::resolve_profile(cfg, &auto.fallback)?,
                auto.fallback.as_str(),
            )
        }
        other => (other, profile),
    };
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
pub(super) fn names_exit(cfg: &NetworkConfiguration, profile: &str, source: NodeId) -> bool {
    selection(cfg, profile).is_ok_and(|Selection { rules, fallback }| {
        rules
            .iter()
            .map(|(_, rule)| rule.action.clone().into())
            .chain(std::iter::once(fallback))
            .any(|route| {
                matching::executed(cfg, route.clone(), source)
                    .map_or(true, |executed| executed != route)
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

/// Rule actions as `source` executes them ([`matching::executed`]); without a
/// source (a profile-level preview) as written.
fn executed(
    cfg: &NetworkConfiguration,
    route: RouteAction,
    source: Option<NodeId>,
) -> Result<RouteAction, CompileError> {
    match source {
        Some(source) => matching::executed(cfg, route, source),
        None => Ok(route),
    }
}

/// The partition of `profile` that `source` executes, without the node-side
/// limits; `Err` names why no partition exists. A rule referencing a set not
/// in `sets` is a missing dependency, never an empty set.
pub fn compute(
    cfg: &NetworkConfiguration,
    sets: &rules::RuleSets,
    profile: &str,
    source: Option<NodeId>,
) -> Result<EgressPartition, UnavailableReason> {
    let Selection { rules, fallback } = selection(cfg, profile).map_err(unavailable)?;
    let fallback = executed(cfg, fallback, source).map_err(unavailable)?;
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
        let id = index(
            &mut actions,
            action(&executed(cfg, rule.action.clone().into(), source).map_err(unavailable)?)
                .map_err(unavailable)?,
        );
        let (hit, rest) = split(&remaining4, &normalize(v4), &mut work)?;
        out4.extend(hit.into_iter().map(|(s, e)| (s, e, id)));
        remaining4 = rest;
        let (hit, rest) = split(&remaining6, &normalize(v6), &mut work)?;
        out6.extend(hit.into_iter().map(|(s, e)| (s, e, id)));
        remaining6 = rest;
    }
    let default_capture = matches!(fallback, RouteAction::Exit { .. }) && !remaining4.is_empty();
    let id = index(&mut actions, action(&fallback).map_err(unavailable)?);
    out4.extend(remaining4.into_iter().map(|(s, e)| (s, e, id)));
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
    Ok(EgressPartition {
        ipv4: assemble(out4),
        ipv6: assemble(out6),
        actions,
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
) -> Vec<String> {
    let Ok(Selection { rules, fallback }) = selection(cfg, profile) else {
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
    sets: &rules::RuleSets,
    profile: &str,
    source: Option<NodeId>,
) -> Result<EgressPartition, UnavailableReason> {
    let partition = compute(cfg, sets, profile, source)?;
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
    sets: &rules::RuleSets,
    profile: &str,
    source: Option<NodeId>,
) -> CaptureSummary {
    match compute(cfg, sets, profile, source) {
        Ok(partition) => {
            let summary = partition.summary();
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
        assert_eq!(kept, [host("203.0.113.9")]);
        assert_eq!(kept_plan.managed_ipv4, previous.managed_ipv4);
        assert_eq!(summary.managed_ipv4, 2);
        // Without live flows the release takes effect.
        let (released, kept, _) = retained_capture(&previous, &next, []);
        assert!(kept.is_empty() && released == next);
    }

    #[test]
    fn split_and_cover() {
        let mut work = 0;
        let (hit, rest) =
            split(&[(0u32, 100)], &[(10, 20), (30, 30), (90, 200)], &mut work).unwrap();
        assert_eq!(hit, [(10, 20), (30, 30), (90, 100)]);
        assert_eq!(rest, [(0, 9), (21, 29), (31, 89)]);
        let (hit, rest) = split(&[(0u32, u32::MAX)], &[(0, u32::MAX)], &mut work).unwrap();
        assert_eq!((hit, rest), (vec![(0, u32::MAX)], vec![]));
        let (hit, rest) = split(&[(5u32, 9), (20, 30)], &[(0, 4), (31, 40)], &mut work).unwrap();
        assert_eq!((hit, rest), (vec![], vec![(5, 9), (20, 30)]));
        assert_eq!(
            normalize(vec![(5u32, 9), (0, 4), (11, 12)]),
            [(0, 9), (11, 12)]
        );
        assert_eq!(
            normalize(vec![(0u128, u128::MAX), (3, 4)]),
            [(0, u128::MAX)]
        );
        assert_eq!(cover(0, u32::MAX), 1);
        assert_eq!(cover(1, u32::MAX), 32);
        assert_eq!(cover(10, 10), 1);
        assert_eq!(cover(0, 2), 2);
    }
}
