//! Physical configuration compatibility, never packet or adjacency authority.
use crate::target::View;
use meshway_configuration::netmap::NodeNetmap;
use meshway_model::state::ResourceRef;

/// The listener declaration, or the dial candidate together with the peer's
/// identity, addresses and keys, is unchanged between two contents. Callers
/// still require `netmap_continuation_from` and current admission.
pub fn netmap_resource_compatible(
    previous: &NodeNetmap,
    next: &NodeNetmap,
    resource: &ResourceRef,
) -> bool {
    compatible(View::new(previous), View::new(next), resource)
}

pub(crate) fn compatible(previous: View<'_>, next: View<'_>, resource: &ResourceRef) -> bool {
    match resource {
        ResourceRef::Listener { key } => previous
            .listeners()
            .get(key)
            .is_some_and(|old| next.listeners().get(key) == Some(old)),
        ResourceRef::Dial { key } => previous.dialers().get(key).is_some_and(|old| {
            next.dialers().get(key).is_some_and(|new| {
                old.peer == new.peer
                    && old.listener_key == new.listener_key
                    && old.parameters == new.parameters
                    && old.endpoints == new.endpoints
                    && previous.peer(old.peer) == next.peer(new.peer)
            })
        }),
        ResourceRef::Coordination { .. } => false,
    }
}
