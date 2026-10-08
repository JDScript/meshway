use super::*;
use meshway_model::{InstanceId, exit::ExitFamily};

/// Fresh instance-bound prerequisite facts supplied by the authenticated Server.
/// Missing facts mean unavailable. Role preparation does not depend on readiness.
#[derive(Clone, Debug, Default)]
pub struct ExitInputs {
    pub instances: BTreeMap<NodeId, InstanceId>,
    pub ready_until_ms: BTreeMap<NodeId, u64>,
    pub unavailable_reasons: BTreeMap<NodeId, meshway_model::exit::ExitUnavailableReason>,
    pub selection_generations: BTreeMap<NodeId, u64>,
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
        && topology.inventory.get(&node).is_some_and(|trusted| {
            trusted
                .capabilities
                .get(meshway_model::exit::EXIT_IPV4_FEATURE)
                == Some(&meshway_model::state::Capability::Feature {
                    version: meshway_model::exit::EXIT_IPV4_VERSION,
                })
        })
}

pub(super) fn role(
    input: &CompilationInput<'_>,
    runtime: &ExitInputs,
    node: NodeId,
) -> Result<Option<ExitRolePlan>, CompileError> {
    let Some(instance_id) = runtime.instances.get(&node).copied() else {
        return Ok(None);
    };
    if !input
        .configuration
        .nodes
        .get(&node)
        .is_some_and(|settings| settings.enabled && settings.exit_enabled)
    {
        return Ok(None);
    }
    Ok(Some(ExitRolePlan {
        instance_id,
        generation: digest(&(input.network, node, input.revision, instance_id))?,
        family: ExitFamily::Ipv4,
        nat: ExitNat::Masquerade,
    }))
}

pub(super) fn destination(
    input: &CompilationInput<'_>,
    demand: &Demand,
) -> Option<ExitDestinationScope> {
    let BusinessTarget::Ip { address } = demand.target else {
        return None;
    };
    let traffic = crate::access::TrafficQuery::Exact {
        protocol: demand.protocol,
        port: demand.port,
    };
    input
        .configuration
        .access
        .iter()
        .filter(|rule| {
            traffic.matches(&rule.ip)
                && matching::selectors(
                    input.configuration,
                    input.subjects,
                    &rule.src,
                    demand.source,
                )
        })
        .flat_map(|rule| &rule.dst)
        .find_map(|selector| {
            if selector == "autogroup:internet" && crate::is_internet_address(address.0) {
                Some(ExitDestinationScope::Internet)
            } else if matching::cidr(selector, address.0) {
                Some(ExitDestinationScope::Cidr {
                    prefix: selector.clone(),
                })
            } else {
                None
            }
        })
}
