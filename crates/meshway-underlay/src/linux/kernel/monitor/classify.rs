//! Classifying a notification: this owner's own interfaces, external
//! addresses inside a Mesh pool, and external network changes.
use super::*;

/// An interface that is neither loopback nor one of this owner's own (Mesh,
/// probe and exit TUNs, the DNS anchor, including while created or retired).
pub(in crate::linux::kernel) fn external(state: &Expected, index: u32) -> bool {
    index != 1
        && state.probe_tun != Some(index)
        && state.exit_tun != Some(index)
        && state.retiring_tun != Some(index)
        && state
            .tun
            .as_ref()
            .is_none_or(|(owned, _, _, _)| *owned != index)
        && state
            .anchor
            .as_ref()
            .is_none_or(|anchor| anchor.identity.index != index)
        && state
            .anchor_creating
            .as_ref()
            .is_none_or(|(_, creating)| *creating != Some(index))
}

/// The addresses of an external interface's address message inside a Mesh
/// pool, keyed as [`Expected::overlaps`] with the interface label.
pub(in crate::linux::kernel) fn pool_addresses(
    state: &Expected,
    address: &rtnetlink::packet_route::address::AddressMessage,
) -> Vec<((u32, std::net::IpAddr), String)> {
    let label = address
        .attributes
        .iter()
        .find_map(|a| match a {
            AddressAttribute::Label(label) => Some(label.clone()),
            _ => None,
        })
        .unwrap_or_else(|| format!("if{}", address.header.index));
    address
        .attributes
        .iter()
        .filter_map(|a| match a {
            AddressAttribute::Local(ip) | AddressAttribute::Address(ip) => Some(*ip),
            _ => None,
        })
        .filter(|ip| state.pools.iter().any(|pool| pool.contains(ip)))
        .map(|ip| ((address.header.index, ip), label.clone()))
        .collect()
}

/// A local-table route of an external interface that holds an address
/// inside a Mesh pool: the kernel's own consequence of that address.
pub(in crate::linux::kernel) fn overlap_route(
    state: &Expected,
    route: &rtnetlink::packet_route::route::RouteMessage,
) -> bool {
    route_table(route) == 255
        && route.attributes.iter().any(|attribute| {
            matches!(attribute, RouteAttribute::Oif(index)
                if state.overlaps.keys().any(|(owner, _)| owner == index))
        })
}

/// Marks an external notification for classification: links that are up,
/// addresses and routes of external interfaces, in any table but this
/// owner's own (capture, source capture, exit) and local.
pub(super) fn network_changed(
    state: &Expected,
    payload: &NetlinkPayload<RouteNetlinkMessage>,
    table: u32,
) -> bool {
    let external = |index| external(state, index);
    match payload {
        // A link that is not up carries no path: creating or deleting one
        // (a VPN daemon's idle ppp0, say) changes no route to anything. An up
        // link going down also deletes its addresses and routes, reported below.
        NetlinkPayload::InnerMessage(
            RouteNetlinkMessage::NewLink(link) | RouteNetlinkMessage::DelLink(link),
        ) => {
            link.header.flags.contains(LinkFlags::Up)
                && external(link.header.index)
                && !state.anchor_creating.as_ref().is_some_and(|(name, _)| {
                    link.attributes
                        .iter()
                        .any(|a| matches!(a, LinkAttribute::IfName(v) if v == name))
                })
        }
        NetlinkPayload::InnerMessage(
            RouteNetlinkMessage::NewAddress(address) | RouteNetlinkMessage::DelAddress(address),
        ) => external(address.header.index),
        NetlinkPayload::InnerMessage(
            RouteNetlinkMessage::NewRoute(route) | RouteNetlinkMessage::DelRoute(route),
        ) => {
            let owner = route_table(route);
            owner != table
                && owner != 255
                && state.egress_table != Some(owner)
                && !state.exit_tables.contains(&owner)
                && route.attributes.iter().all(|attribute| match attribute {
                    RouteAttribute::Oif(index) => external(*index),
                    _ => true,
                })
        }
        _ => false,
    }
}
