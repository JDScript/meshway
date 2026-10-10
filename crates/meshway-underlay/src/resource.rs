//! Physical configuration compatibility, never packet or adjacency authority.
use crate::target::View;
use meshway_configuration::netmap::NodeNetmap;
use meshway_model::state::ResourceRef;

/// The listener declaration, or the dial candidate together with the peer's
/// registered keys, is unchanged between two contents. The peer's Mesh
/// addresses and identity are policy inputs, not part of an outer socket's
/// purpose. This is the single resource-retention rule: the Host's descriptor
/// registry and the connectivity Manager both apply it. Callers
/// apply only content Engine accepted, and still require current admission.
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
                    && previous.peer(old.peer).map(|peer| peer.keys)
                        == next.peer(new.peer).map(|peer| peer.keys)
            })
        }),
        ResourceRef::Coordination { .. } => false,
    }
}
