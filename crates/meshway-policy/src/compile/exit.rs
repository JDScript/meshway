use super::*;
use meshway_model::{InstanceId, exit::ExitFamily};

/// Authenticated live prerequisites for management evaluation only. These
/// observations never enter netmap projection or local path authorization.
#[derive(Clone, Debug, Default)]
pub struct ExitInputs {
    pub instances: BTreeMap<NodeId, InstanceId>,
    /// Until when each (exit, family) was last observed ready.
    pub ready_until_ms: BTreeMap<(NodeId, ExitFamily), u64>,
    pub unavailable_reasons:
        BTreeMap<(NodeId, ExitFamily), meshway_model::exit::ExitUnavailableReason>,
}

/// Whether `node` declares the executor of `family` (`feature:exit-ipv4`
/// or `feature:exit-ipv6`, version 1) in its trusted capabilities.
pub(super) fn capable_of(
    inventory: &BTreeMap<NodeId, TrustedNode>,
    node: NodeId,
    family: ExitFamily,
) -> bool {
    let (feature, version) = family.feature();
    inventory.get(&node).is_some_and(|trusted| {
        trusted.capabilities.get(feature)
            == Some(&meshway_model::state::Capability::Feature { version })
    })
}

pub(super) fn capable(topology: Topology<'_>, node: NodeId, family: ExitFamily) -> bool {
    capable_of(topology.inventory, node, family)
        && capable_of(topology.inventory, node, ExitFamily::Ipv4)
}
pub(super) fn available(
    topology: Topology<'_>,
    runtime: &ExitInputs,
    node: NodeId,
    family: ExitFamily,
    now_ms: u64,
) -> bool {
    runtime.instances.contains_key(&node)
        && runtime
            .ready_until_ms
            .get(&(node, family))
            .is_some_and(|expiry| *expiry > now_ms)
        && capable(topology, node, family)
}
