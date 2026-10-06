//! Physical configuration compatibility, never packet or adjacency authority.
use meshway_configuration::execution::NodeConfiguration;
use meshway_model::state::ResourceRef;

/// Compare the exact configured purpose of an existing descriptor. Callers
/// must separately obtain Control's direct continuation and current admission.
pub fn resource_compatible(
    previous: &NodeConfiguration,
    next: &NodeConfiguration,
    resource: &ResourceRef,
) -> bool {
    match resource {
        ResourceRef::Listener { key } => previous
            .listeners
            .get(key)
            .is_some_and(|old| next.listeners.get(key) == Some(old)),
        ResourceRef::Dial { key } => previous.dialers.get(key).is_some_and(|old| {
            next.dialers.get(key).is_some_and(|new| {
                old.peer == new.peer
                    && old.listener_key == new.listener_key
                    && old.parameters == new.parameters
                    && old.endpoints == new.endpoints
                    && previous.peers.get(&old.peer) == next.peers.get(&new.peer)
            })
        }),
        ResourceRef::Coordination { .. } => false,
    }
}
