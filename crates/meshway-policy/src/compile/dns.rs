//! Which resolver a source's ordinary names use is decided before resolution.
//! Answers never decide which resolver was authorized to obtain them.
use super::*;

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
/// selection its partition uses (public rules first, an automatic profile as
/// its fallback). A rule takes part when it has names: its own domain lists,
/// or a referenced set with names; a set not in `sets` counts as one with
/// names. Each such rule leaving through an exit, and an exit fallback,
/// must name the same single exit and via; a rule naming the source itself
/// is its direct egress ([`matching::executed`]).
pub fn resolution(
    cfg: &NetworkConfiguration,
    sets: &rules::RuleSets,
    profile: &str,
    source: NodeId,
) -> Result<DnsResolution, CompileError> {
    let capture::Selection {
        rules: selected,
        fallback,
    } = match capture::selection(cfg, profile) {
        Ok(selection) => selection,
        Err(CompileError::Serialization) => return Err(CompileError::Serialization),
        Err(_) => return Ok(DnsResolution::PlanRequired { pointer: None }),
    };
    let fallback_pointer = matching::resolve_profile(cfg, profile)
        .ok()
        .and_then(|resolved| match resolved {
            matching::EgressProfile::Custom(_) => Some(profile),
            matching::EgressProfile::Auto(auto) => Some(auto.fallback.as_str()),
            _ => None,
        })
        .filter(|name| cfg.routing.egress.profiles.contains_key(*name))
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
        let RouteAction::Exit { nodes, via } = matching::executed(cfg, action, source)? else {
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
