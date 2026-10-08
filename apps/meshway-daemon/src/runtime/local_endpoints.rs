//! Sample interface facts for existing listening sockets. Enumeration never
//! creates a listener, changes capture, or grants an execution permission.
use meshway_configuration::{execution::NodeConfiguration, transport::Listener};
use meshway_model::state::{
    LocalEndpoint, LocalEndpointSource, MAX_LOCAL_ENDPOINTS, NodeState, ResourceRef,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, SocketAddr},
};

struct InterfaceAddress {
    name: String,
    address: IpAddr,
}

pub(super) fn sample(state: &mut NodeState, configuration: &NodeConfiguration) {
    let Ok(interfaces) = interfaces() else {
        return;
    };
    let pools: Result<Vec<ipnet::IpNet>, _> = configuration
        .mesh_address_pools
        .ipv4
        .iter()
        .chain(&configuration.mesh_address_pools.ipv6)
        .map(|pool| pool.parse())
        .collect();
    let Ok(pools) = pools else {
        return;
    };
    let mut counts = BTreeMap::new();
    for (id, socket) in &mut state.sockets {
        let ResourceRef::Listener { key } = &socket.resource else {
            continue;
        };
        // Only the actual listening socket is a candidate, not a stream accepted
        // by that listener whose resource also refers to the listener key.
        if !state
            .listeners
            .get(key)
            .is_some_and(|listener| listener.socket_ids.contains(id))
        {
            continue;
        }
        let Some(listener) = configuration.listeners.get(key) else {
            continue;
        };
        let interface = match listener.declaration() {
            Listener::NativeUdp(listener) => &listener.bind_interface,
            Listener::NativeTcp(listener) => &listener.bind_interface,
            Listener::Vless(listener) => &listener.bind_interface,
            Listener::Trojan(listener) => &listener.bind_interface,
        };
        let mut local = candidates(socket.bound_address, interface, &interfaces, &pools);
        let count = counts.entry(key).or_insert(0);
        local.truncate(MAX_LOCAL_ENDPOINTS - *count);
        *count += local.len();
        socket.local_endpoints = Some(local);
    }
}

fn interfaces() -> Result<Vec<InterfaceAddress>, nix::errno::Errno> {
    use nix::{ifaddrs::getifaddrs, net::if_::InterfaceFlags};
    Ok(getifaddrs()?
        .filter(|interface| {
            interface.flags.contains(InterfaceFlags::IFF_UP)
                && !interface.flags.contains(InterfaceFlags::IFF_LOOPBACK)
        })
        .filter_map(|interface| {
            let socket = interface.address?;
            let address = socket
                .as_sockaddr_in()
                .map(|address| IpAddr::V4(address.ip()))
                .or_else(|| {
                    socket
                        .as_sockaddr_in6()
                        .map(|address| IpAddr::V6(address.ip()))
                })?;
            Some(InterfaceAddress {
                name: interface.interface_name,
                address,
            })
        })
        .collect())
}

fn candidates(
    bound: SocketAddr,
    interface: &str,
    interfaces: &[InterfaceAddress],
    pools: &[ipnet::IpNet],
) -> Vec<LocalEndpoint> {
    if bound.port() == 0 {
        return Vec::new();
    }
    interfaces
        .iter()
        .filter(|candidate| {
            (interface.is_empty() || candidate.name == interface)
                && bound.is_ipv4() == candidate.address.is_ipv4()
                && (bound.ip().is_unspecified() || bound.ip() == candidate.address)
                && usable(candidate.address, pools)
        })
        .map(|candidate| SocketAddr::new(candidate.address, bound.port()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(MAX_LOCAL_ENDPOINTS)
        .map(|address| LocalEndpoint {
            address,
            source: LocalEndpointSource::Local,
        })
        .collect()
}

fn usable(address: IpAddr, pools: &[ipnet::IpNet]) -> bool {
    !address.is_unspecified()
        && !address.is_loopback()
        && !address.is_multicast()
        && !pools.iter().any(|pool| pool.contains(&address))
        && match address {
            IpAddr::V4(address) => !address.is_link_local() && !address.is_broadcast(),
            IpAddr::V6(address) => {
                !address.is_unicast_link_local() && address.to_ipv4_mapped().is_none()
            }
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn interface(name: &str, address: &str) -> InterfaceAddress {
        InterfaceAddress {
            name: name.into(),
            address: address.parse().unwrap(),
        }
    }
    #[test]
    fn wildcard_expands_same_family_and_excludes_mesh_loopback_link_local() {
        let interfaces = [
            interface("lo", "127.0.0.1"),
            interface("tun", "100.64.1.1"),
            interface("eth0", "169.254.1.1"),
            interface("eth0", "192.168.1.2"),
            interface("eth1", "10.0.0.2"),
            interface("eth0", "fe80::1"),
            interface("tun", "fd00::1"),
            interface("eth0", "::ffff:192.168.1.2"),
            interface("eth0", "2001:db8::2"),
        ];
        let pools = [
            "100.64.0.0/10".parse().unwrap(),
            "fd00::/64".parse().unwrap(),
        ];
        let v4 = candidates("0.0.0.0:35000".parse().unwrap(), "", &interfaces, &pools);
        assert_eq!(
            v4.iter()
                .map(|endpoint| endpoint.address.to_string())
                .collect::<Vec<_>>(),
            ["10.0.0.2:35000", "192.168.1.2:35000"]
        );
        let v6 = candidates("[::]:35000".parse().unwrap(), "", &interfaces, &pools);
        assert_eq!(v6[0].address, "[2001:db8::2]:35000".parse().unwrap());
        assert_eq!(v6.len(), 1);
    }
    #[test]
    fn explicit_bind_interface_changes_dedup_and_budget() {
        let mut interfaces: Vec<_> = (1..20)
            .map(|i| interface("eth0", &format!("10.0.0.{i}")))
            .collect();
        interfaces.push(interface("eth0", "10.0.0.1"));
        assert_eq!(
            candidates("0.0.0.0:35000".parse().unwrap(), "", &interfaces, &[]).len(),
            8
        );
        assert!(candidates("0.0.0.0:35000".parse().unwrap(), "eth1", &interfaces, &[]).is_empty());
        let bound = "10.0.0.19:35000".parse().unwrap();
        assert_eq!(
            candidates(bound, "eth0", &interfaces, &[])[0].address,
            bound
        );
        interfaces.retain(|interface| interface.address != bound.ip());
        assert!(candidates(bound, "eth0", &interfaces, &[]).is_empty());
    }
}
