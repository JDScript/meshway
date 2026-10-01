//! Resolve shared path requests and project the installable source catalogs.
use super::profile_plan::{PathPlan, PathRequest, Plan};
use super::*;

pub(super) fn project(
    p: &NetworkPolicy,
    base: Compiled,
    matchers: Vec<ProfileMatcher>,
    plan: Plan,
    path_ids: BTreeMap<PathRequest, Vec<u32>>,
) -> NetworkCompilation {
    let resolve = |groups: PathPlan| {
        groups
            .into_iter()
            .flat_map(|mut group| {
                group.sort_by(|a, b| {
                    let rank = |r: &PathRequest| {
                        path_ids[r]
                            .first()
                            .map_or(usize::MAX, |i| base.paths[*i as usize].len())
                    };
                    rank(a).cmp(&rank(b)).then(a.dst.cmp(&b.dst))
                });
                group.into_iter().flat_map(|r| path_ids[&r].clone())
            })
            .collect::<Vec<_>>()
    };
    let sources = plan
        .sources
        .into_iter()
        .map(|s| SourceProfiles {
            allowed: s.allowed,
            default_profile: s.default_profile,
            locked: s.locked,
            automatic: p.automatic.clone(),
            profiles: s
                .profiles
                .into_iter()
                .map(|p| CompiledProfile {
                    id: p.id,
                    matcher: p.matcher,
                    routes: p
                        .routes
                        .into_iter()
                        .map(|(rule, target, paths)| ProfileRoute {
                            rule,
                            target,
                            paths: resolve(paths),
                        })
                        .collect(),
                    ip_table: p.table,
                    fallback: match p.fallback {
                        None => CompiledFinal::None,
                        Some((true, _)) => CompiledFinal::Direct,
                        Some((false, paths)) => CompiledFinal::Nodes(resolve(paths)),
                    },
                    lints: p.lints,
                })
                .collect(),
        })
        .collect();
    NetworkCompilation {
        base,
        matchers,
        ip_tables: plan.tables,
        sources,
    }
}
