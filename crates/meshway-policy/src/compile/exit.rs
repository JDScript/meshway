use super::*;
use meshway_model::InstanceId;

/// Authenticated live prerequisites for management evaluation only. These
/// observations never enter netmap projection or local path authorization.
#[derive(Clone, Debug, Default)]
pub struct ExitInputs {
    pub instances: BTreeMap<NodeId, InstanceId>,
    pub ready_until_ms: BTreeMap<NodeId, u64>,
    pub unavailable_reasons: BTreeMap<NodeId, meshway_model::exit::ExitUnavailableReason>,
}

pub(super) fn capable(topology: Topology<'_>, node: NodeId) -> bool {
    topology.inventory.get(&node).is_some_and(|trusted| {
        trusted
            .capabilities
            .get(meshway_model::exit::EXIT_IPV4_FEATURE)
            == Some(&meshway_model::state::Capability::Feature {
                version: meshway_model::exit::EXIT_IPV4_VERSION,
            })
    })
}
pub(super) fn available(
    topology: Topology<'_>,
    runtime: &ExitInputs,
    node: NodeId,
    now_ms: u64,
) -> bool {
    runtime.instances.contains_key(&node)
        && runtime
            .ready_until_ms
            .get(&node)
            .is_some_and(|expiry| *expiry > now_ms)
        && capable(topology, node)
}
