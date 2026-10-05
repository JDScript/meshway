//! Conflicts ahead of the private capture block. The ordinary local-table rule
//! stays first, so its only Mesh exception is an exact live owned TUN address.
use ipnet::IpNet;
use rtnetlink::packet_route::{
    AddressFamily,
    address::{AddressAttribute, AddressMessage, AddressScope},
    route::{
        RouteAddress, RouteAttribute, RouteMessage, RoutePreference, RouteProtocol, RouteScope,
        RouteType,
    },
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Clone, Copy)]
pub(super) struct Owned<'a> {
    pub index: u32,
    pub addresses: &'a [IpAddr],
}

pub(super) fn address_conflicts(
    pools: &[IpNet],
    owned: Option<Owned<'_>>,
    address: &AddressMessage,
) -> bool {
    let ips: Vec<_> = address
        .attributes
        .iter()
        .filter_map(|attribute| match attribute {
            AddressAttribute::Local(ip) | AddressAttribute::Address(ip) => Some(*ip),
            _ => None,
        })
        .collect();
    if !ips
        .iter()
        .any(|ip| pools.iter().any(|pool| pool.contains(ip)))
    {
        return false;
    }
    !owned.is_some_and(|owned| {
        address.header.index == owned.index
            && address.header.scope == AddressScope::Universe
            && ips.first().is_some_and(|ip| {
                owned.addresses.contains(ip)
                    && ips.iter().all(|other| other == ip)
                    && address.header.family == family(*ip)
                    && address.header.prefix_len == host_prefix(*ip)
            })
    })
}

pub(super) fn route_conflicts(
    pools: &[IpNet],
    owned: Option<Owned<'_>>,
    route: &RouteMessage,
) -> bool {
    if super::route_table(route) != 255 || route.header.kind == RouteType::Throw {
        return false;
    }
    let unspecified = match route.header.address_family {
        AddressFamily::Inet => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        AddressFamily::Inet6 => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        _ => return false,
    };
    if !pools
        .iter()
        .any(|pool| pool.addr().is_ipv4() == unspecified.is_ipv4())
    {
        return false;
    }
    let destinations: Vec<_> = route
        .attributes
        .iter()
        .filter_map(|attribute| match attribute {
            RouteAttribute::Destination(address) => Some(address),
            _ => None,
        })
        .collect();
    let destination = match destinations.as_slice() {
        [] if route.header.destination_prefix_length == 0 => unspecified,
        [RouteAddress::Inet(ip)] if unspecified.is_ipv4() => IpAddr::V4(*ip),
        [RouteAddress::Inet6(ip)] if unspecified.is_ipv6() => IpAddr::V6(*ip),
        _ => return true,
    };
    let Ok(network) = IpNet::new(destination, route.header.destination_prefix_length) else {
        return true;
    };
    if !pools
        .iter()
        .any(|pool| pool.contains(&network.network()) || network.contains(&pool.network()))
    {
        return false;
    }
    // No interface-wide exemption: a broader prefix, second address, gateway,
    // multipath or non-kernel replacement on that very TUN is still a conflict.
    !owned.is_some_and(|owned| {
        owned.index != 0
            && owned.addresses.contains(&destination)
            && route.header.destination_prefix_length == host_prefix(destination)
            && route.header.source_prefix_length == 0
            && route.header.tos == 0
            && route.header.flags.is_empty()
            && route.header.protocol == RouteProtocol::Kernel
            && route.header.kind == RouteType::Local
            && route.header.scope
                == if destination.is_ipv4() {
                    RouteScope::Host
                } else {
                    RouteScope::Universe
                }
            && route
                .attributes
                .iter()
                .filter(|a| matches!(a, RouteAttribute::Oif(_)))
                .count()
                == 1
            && route.attributes.iter().all(|attribute| match attribute {
                RouteAttribute::Destination(_) => true,
                RouteAttribute::Oif(index) => *index == owned.index,
                RouteAttribute::PrefSource(RouteAddress::Inet(ip)) => {
                    IpAddr::V4(*ip) == destination
                }
                RouteAttribute::PrefSource(RouteAddress::Inet6(ip)) => {
                    IpAddr::V6(*ip) == destination
                }
                RouteAttribute::Table(table) => *table == 255,
                RouteAttribute::Priority(priority) => *priority == 0,
                RouteAttribute::CacheInfo(_)
                | RouteAttribute::Preference(RoutePreference::Medium) => true,
                _ => false,
            })
    })
}

fn family(ip: IpAddr) -> AddressFamily {
    if ip.is_ipv4() {
        AddressFamily::Inet
    } else {
        AddressFamily::Inet6
    }
}
fn host_prefix(ip: IpAddr) -> u8 {
    if ip.is_ipv4() { 32 } else { 128 }
}

#[cfg(test)]
mod tests;
