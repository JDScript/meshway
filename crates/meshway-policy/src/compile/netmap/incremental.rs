//! Stable-input fingerprints select outputs before materializing netmaps.
//! Only the committing Server owns these hints; they are never wire authority.
use super::*;

pub struct IncrementalNetmaps {
    /// Only outputs whose stable inputs may have changed; unchanged content
    /// still retains its old serial. Missing members are withdrawals.
    pub compiled: CompiledNetmaps,
    /// Every current member's input fingerprint, persisted in the same
    /// transaction as output changes. No configuration payload is duplicated.
    pub inputs: BTreeMap<NodeId, Digest>,
}

pub fn compile_incremental(
    input: NetmapInput<'_>,
    previous_inputs: &BTreeMap<NodeId, Digest>,
) -> Result<IncrementalNetmaps, NetmapError> {
    let members = validate(&input)?;
    let graph = paths::Graph::new(input.topology())?;
    let index = project::Index::new(&input, &graph, &members)?;
    let inputs = fingerprints(&input, &index, &members)?;
    let selected = inputs
        .iter()
        .filter_map(|(id, fingerprint)| {
            (previous_inputs.get(id) != Some(fingerprint) || !input.previous.contains_key(id))
                .then_some(*id)
        })
        .collect();
    let compiled = compile_selected(&input, &graph, &index, &members, &selected)?;
    Ok(IncrementalNetmaps { compiled, inputs })
}

fn fingerprints(
    input: &NetmapInput<'_>,
    index: &project::Index,
    members: &[NodeId],
) -> Result<BTreeMap<NodeId, Digest>, NetmapError> {
    // Exhaustive destructuring makes additions to the canonical input require
    // an explicit dependency decision. Revisions/comments are not semantics.
    let NetworkConfiguration {
        address_pools,
        defaults,
        groups,
        tags_owners,
        access,
        nodes,
        dns,
        topology,
        routing,
        rule_sets,
    } = input.configuration;
    let addresses: BTreeMap<_, _> = nodes
        .iter()
        .map(|(id, node)| (id, &node.addresses))
        .collect();
    let global = digest(&(
        (
            NETMAP_COMPILER_VERSION,
            input.network,
            address_pools,
            defaults,
            groups,
            tags_owners,
        ),
        (access, dns, topology, routing, rule_sets, input.rule_sets),
        // Resolver classification consults all configured addresses, and
        // subject aliases can affect selectors outside a retained projection.
        (&addresses, input.subjects),
    ))?;
    let node_inputs: BTreeMap<_, _> = nodes
        .iter()
        .map(|(id, settings)| {
            Ok((
                *id,
                digest(&(
                    settings,
                    input.inventory.get(id),
                    input.selected_profiles.get(id),
                    input.selection_versions.get(id),
                ))?,
            ))
        })
        .collect::<Result<_, NetmapError>>()?;
    // Even an unreachable configured exit can change DNS unavailability from
    // missing capability to no path. Keep those dependencies explicitly.
    // Includes built-in global:<node> and auto choices without inventing a
    // selector expansion for those profiles. A role becoming enabled/disabled
    // changes the dependency set itself.
    let mut exits: BTreeSet<_> = nodes
        .iter()
        .filter(|(_, node)| node.exit_enabled)
        .map(|(id, _)| *id)
        .collect();
    for profile in routing.egress.profiles.values() {
        if let RouteAction::Exit { nodes, via } = &profile.fallback {
            exits.extend(
                nodes
                    .iter()
                    .chain(via)
                    .map(|s| matching::node_selector(s))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        for rule in &profile.rules {
            if let RuleAction::Exit { nodes, via } = &rule.action {
                exits.extend(
                    nodes
                        .iter()
                        .chain(via)
                        .map(|s| matching::node_selector(s))
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
        }
    }
    members
        .iter()
        .map(|id| {
            let dependencies: BTreeMap<_, _> = index
                .visible(*id)
                .into_iter()
                .chain(exits.iter().copied())
                .map(|node| (node, node_inputs.get(&node)))
                .collect();
            Ok((*id, digest(&(global, id, dependencies))?))
        })
        .collect()
}
