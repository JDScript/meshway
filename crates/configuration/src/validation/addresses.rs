use super::*;
use ipnet::{Ipv4Net, Ipv6Net};
use std::net::IpAddr;

pub(super) fn pools(
    input: AddressPoolsInput,
    previous: Option<&AddressPools>,
    check: &mut Check,
) -> AddressPools {
    fn field(
        value: Input<Vec<String>>,
        old: Option<&Vec<String>>,
        path: &str,
        kind: PreparationKind,
        check: &mut Check,
    ) -> Vec<String> {
        match value {
            Input::Value(value) => value,
            Input::Omitted => old.cloned().unwrap_or_else(|| {
                check.require(path, kind);
                vec![]
            }),
        }
    }
    let mut result = AddressPools {
        ipv4: field(
            input.ipv4,
            previous.map(|p| &p.ipv4),
            "/addressPools/ipv4",
            PreparationKind::AddressPoolV4,
            check,
        ),
        ipv6: field(
            input.ipv6,
            previous.map(|p| &p.ipv6),
            "/addressPools/ipv6",
            PreparationKind::AddressPoolV6,
            check,
        ),
    };
    let mut v4 = Vec::<Ipv4Net>::new();
    for (i, text) in result.ipv4.iter().enumerate() {
        let path = format!("/addressPools/ipv4/{i}");
        match text.parse::<Ipv4Net>() {
            Ok(net)
                if net.to_string() == *text
                    && net.addr() == net.network()
                    && !net.addr().is_unspecified()
                    && !net.addr().is_multicast()
                    && !net.addr().is_loopback()
                    && !net.addr().is_link_local()
                    && !net.broadcast().is_multicast() =>
            {
                if v4
                    .iter()
                    .any(|p| p.contains(&net.network()) || net.contains(&p.network()))
                {
                    check.issue(path, "overlapping address pools");
                }
                v4.push(net);
            }
            _ => check.issue(path, "expected a canonical unicast IPv4 pool"),
        }
    }
    let mut v6 = Vec::<Ipv6Net>::new();
    for (i, text) in result.ipv6.iter().enumerate() {
        let path = format!("/addressPools/ipv6/{i}");
        match text.parse::<Ipv6Net>() {
            Ok(net)
                if net.to_string() == *text
                    && net.addr() == net.network()
                    && !net.addr().is_unspecified()
                    && !net.addr().is_multicast()
                    && !net.addr().is_loopback()
                    && !net.addr().is_unicast_link_local() =>
            {
                if v6
                    .iter()
                    .any(|p| p.contains(&net.network()) || net.contains(&p.network()))
                {
                    check.issue(path, "overlapping address pools");
                }
                v6.push(net);
            }
            _ => check.issue(path, "expected a canonical unicast IPv6 pool"),
        }
    }
    result.ipv4.sort();
    result.ipv6.sort();
    result
}
pub(super) fn resolve<T: Copy>(
    input: Input<Option<T>>,
    old: Option<Option<T>>,
    empty: bool,
    path: &str,
    kind: PreparationKind,
    check: &mut Check,
) -> Option<T> {
    match input {
        Input::Value(v) => v,
        Input::Omitted => old.unwrap_or_else(|| {
            if !empty {
                check.require(path, kind);
            }
            None
        }),
    }
}
pub(super) fn nodes(config: &NetworkConfiguration, check: &mut Check) {
    let v4: Vec<_> = config
        .address_pools
        .ipv4
        .iter()
        .filter_map(|p| p.parse::<Ipv4Net>().ok())
        .collect();
    let v6: Vec<_> = config
        .address_pools
        .ipv6
        .iter()
        .filter_map(|p| p.parse::<Ipv6Net>().ok())
        .collect();
    let mut used = BTreeSet::<IpAddr>::new();
    let mut hosts = BTreeSet::new();
    for (id, node) in &config.nodes {
        if !hosts.insert(&node.hostname) {
            check.issue(format!("/nodes/{id}/hostname"), "duplicate hostname");
        }
        if let Some(ip) = node.addresses.ipv4 {
            let path = format!("/nodes/{id}/addresses/ipv4");
            if ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_loopback()
                || ip.is_link_local()
                || !v4.iter().any(|p| {
                    p.contains(&ip)
                        && (p.prefix_len() >= 31 || (ip != p.network() && ip != p.broadcast()))
                })
            {
                check.issue(&path, "address outside usable pool");
            }
            if !used.insert(ip.into()) {
                check.issue(path, "duplicate address");
            }
        }
        if let Some(ip) = node.addresses.ipv6 {
            let path = format!("/nodes/{id}/addresses/ipv6");
            if ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_loopback()
                || ip.is_unicast_link_local()
                || !v6.iter().any(|p| p.contains(&ip))
            {
                check.issue(&path, "address outside usable pool");
            }
            if !used.insert(ip.into()) {
                check.issue(path, "duplicate address");
            }
        }
    }
}
