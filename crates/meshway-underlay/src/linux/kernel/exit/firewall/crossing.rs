use rtnetlink::packet_route::{
    AddressFamily,
    address::{AddressAttribute, AddressMessage},
    link::{LinkAttribute, LinkMessage},
    route::{RouteAddress, RouteAttribute, RouteMessage, RouteProtocol, RouteType},
};
use std::collections::BTreeMap;

/// The interfaces and addresses exit traffic crosses besides its own TUN.
#[derive(Clone, Debug, Default)]
pub(in crate::linux::kernel) struct Crossing {
    /// Interfaces of main-table IPv4 routes toward Internet-classified
    /// addresses: forwarded traffic leaves, replies arrive. Routes only to
    /// private networks (Docker bridges, say) are not exit paths to judge.
    pub egress: Vec<String>,
    /// Every interface present.
    pub present: Vec<String>,
    /// The installed exit TUN's name, matched exactly; without one the exit
    /// TUN is a new kernel `tun<N>`, none of `present`.
    pub installed: Option<String>,
    /// IPv4 addresses of the egress interfaces: a reply's destination before
    /// the masquerade is undone.
    pub local: Vec<std::net::Ipv4Addr>,
    /// Interfaces of main-table IPv6 routes toward Internet-classified
    /// addresses.
    pub egress6: Vec<String>,
    /// Usable (not tentative, failed or deprecated) Internet-classified IPv6
    /// addresses of the IPv6 egress interfaces: what masquerade uses.
    pub local6: Vec<std::net::Ipv6Addr>,
    /// IPv6 egress interfaces whose route toward the Internet a router
    /// advertisement installed.
    pub advertised6: Vec<String>,
    /// Mesh address pools: a reply's destination after it is undone, and
    /// that of this host's own errors toward the exit TUN.
    pub mesh: Vec<ipnet::IpNet>,
}

impl Crossing {
    pub fn observe(
        links: &[LinkMessage],
        routes: &[RouteMessage],
        addresses: &[AddressMessage],
    ) -> Self {
        let names: BTreeMap<u32, String> = links
            .iter()
            .filter_map(|link| {
                link.attributes
                    .iter()
                    .find_map(|attribute| match attribute {
                        LinkAttribute::IfName(name) => Some((link.header.index, name.clone())),
                        _ => None,
                    })
            })
            .collect();
        let egress_of = |family: AddressFamily, advertised: &mut Vec<String>| {
            let mut egress: Vec<String> = Vec::new();
            for route in routes.iter().filter(|route| {
                route.header.address_family == family
                    && route.header.kind == RouteType::Unicast
                    && super::super::super::plan::route_table(route) == 254
                    && reaches_internet(route)
            }) {
                for attribute in &route.attributes {
                    let indexes: Vec<u32> = match attribute {
                        RouteAttribute::Oif(index) => vec![*index],
                        RouteAttribute::MultiPath(hops) => {
                            hops.iter().map(|hop| hop.interface_index).collect()
                        }
                        _ => continue,
                    };
                    for index in indexes {
                        if let Some(name) = names.get(&index) {
                            if !egress.contains(name) {
                                egress.push(name.clone());
                            }
                            if route.header.protocol == RouteProtocol::Ra
                                && !advertised.contains(name)
                            {
                                advertised.push(name.clone());
                            }
                        }
                    }
                }
            }
            egress
        };
        let mut advertised6 = Vec::new();
        let egress = egress_of(AddressFamily::Inet, &mut Vec::new());
        let egress6 = egress_of(AddressFamily::Inet6, &mut advertised6);
        let local6 = addresses
            .iter()
            .filter(|address| {
                names
                    .get(&address.header.index)
                    .is_some_and(|name| egress6.contains(name))
                    // Tentative, deprecated or failed duplicate detection:
                    // not a source masquerade selects.
                    && super::super::super::network::address_flags(address) & 0x68 == 0
            })
            .flat_map(|address| &address.attributes)
            .filter_map(|attribute| match attribute {
                AddressAttribute::Address(std::net::IpAddr::V6(ip))
                    if meshway_policy::is_internet_address((*ip).into()) =>
                {
                    Some(*ip)
                }
                _ => None,
            })
            .fold(Vec::new(), |mut local, ip| {
                if !local.contains(&ip) {
                    local.push(ip);
                }
                local
            });
        let local = addresses
            .iter()
            .filter(|address| {
                names
                    .get(&address.header.index)
                    .is_some_and(|name| egress.contains(name))
            })
            .flat_map(|address| &address.attributes)
            .filter_map(|attribute| match attribute {
                AddressAttribute::Local(std::net::IpAddr::V4(ip))
                | AddressAttribute::Address(std::net::IpAddr::V4(ip)) => Some(*ip),
                _ => None,
            })
            .fold(Vec::new(), |mut local, ip| {
                if !local.contains(&ip) {
                    local.push(ip);
                }
                local
            });
        Self {
            egress,
            present: names.into_values().collect(),
            installed: None,
            local,
            egress6,
            local6,
            advertised6,
            mesh: Vec::new(),
        }
    }
}

/// Whether a route's destination holds an Internet-classified address: the
/// default route, or a connected public network.
fn reaches_internet(route: &RouteMessage) -> bool {
    let length = route.header.destination_prefix_length;
    let destination = route
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            RouteAttribute::Destination(RouteAddress::Inet(address)) => {
                Some(std::net::IpAddr::V4(*address))
            }
            RouteAttribute::Destination(RouteAddress::Inet6(address)) => {
                Some(std::net::IpAddr::V6(*address))
            }
            _ => None,
        });
    let address = destination.unwrap_or(if route.header.address_family == AddressFamily::Inet6 {
        std::net::Ipv6Addr::UNSPECIFIED.into()
    } else {
        std::net::Ipv4Addr::UNSPECIFIED.into()
    });
    let Ok(prefix) = ipnet::IpNet::new(address, length).map(|net| net.trunc()) else {
        return false;
    };
    meshway_policy::internet_ranges()
        .iter()
        .any(|range| range.contains(&prefix) || prefix.contains(range))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossing_holds_the_interfaces_toward_internet_addresses() {
        let link = |index, name: &str| {
            let mut link = LinkMessage::default();
            link.header.index = index;
            link.attributes.push(LinkAttribute::IfName(name.into()));
            link
        };
        let route = |destination: &str, length, oif, table| {
            let mut route = RouteMessage::default();
            route.header.address_family = AddressFamily::Inet;
            route.header.kind = RouteType::Unicast;
            route.header.destination_prefix_length = length;
            route.attributes.push(RouteAttribute::Table(table));
            if length > 0 {
                route
                    .attributes
                    .push(RouteAttribute::Destination(RouteAddress::Inet(
                        destination.parse().unwrap(),
                    )));
            }
            route.attributes.push(RouteAttribute::Oif(oif));
            route
        };
        let links = [
            link(1, "lo"),
            link(2, "eth0"),
            link(3, "docker0"),
            link(4, "ext"),
            link(5, "lan"),
            link(6, "tun0"),
        ];
        let routes = [
            route("0.0.0.0", 0, 2, 254),
            route("104.194.68.0", 22, 2, 254),
            route("172.17.0.0", 16, 3, 254),
            route("11.77.3.0", 29, 4, 254),
            route("192.168.1.0", 24, 5, 254),
            route("0.0.0.0", 0, 6, 1000),
        ];
        let address = |index, ip: &str| {
            let mut address = AddressMessage::default();
            address.header.index = index;
            address
                .attributes
                .push(AddressAttribute::Local(ip.parse().unwrap()));
            address
        };
        let addresses = [
            address(2, "104.194.68.10"),
            address(3, "172.17.0.1"),
            address(4, "11.77.3.2"),
            address(2, "104.194.68.10"),
        ];
        let crossing = Crossing::observe(&links, &routes, &addresses);
        assert_eq!(crossing.egress, ["eth0", "ext"]);
        assert_eq!(crossing.present.len(), 6);
        // Egress interfaces' addresses only, once each.
        assert_eq!(
            crossing.local,
            ["104.194.68.10", "11.77.3.2"].map(|ip| ip.parse::<std::net::Ipv4Addr>().unwrap())
        );
        // Without a default route the connected public network remains.
        assert_eq!(
            Crossing::observe(&links, &routes[2..], &addresses).egress,
            ["ext"]
        );
    }
}
