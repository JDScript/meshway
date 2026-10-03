use super::*;
use ipnet::{Ipv4Net, Ipv6Net};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

// IPAM exclusions, not the autogroup:internet/public-routing classification.
// RFC1918, CGNAT, ULA and other unicast space remain eligible. A whole pool
// must avoid every excluded interval; checking only its endpoints is unsound.
const IPV4_EXCLUDED: [(Ipv4Addr, u32); 5] = [
    (Ipv4Addr::new(0, 0, 0, 0), 8), // This network, including unspecified.
    (Ipv4Addr::new(127, 0, 0, 0), 8), // Loopback.
    (Ipv4Addr::new(169, 254, 0, 0), 16), // Link-local.
    (Ipv4Addr::new(224, 0, 0, 0), 4), // Multicast.
    (Ipv4Addr::new(240, 0, 0, 0), 4), // Reserved, including limited broadcast.
];
const IPV6_EXCLUDED: [(Ipv6Addr, u32); 4] = [
    (Ipv6Addr::UNSPECIFIED, 128),
    (Ipv6Addr::LOCALHOST, 128),
    (Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0), 10), // Link-local.
    (Ipv6Addr::new(0xff00, 0, 0, 0, 0, 0, 0, 0), 8),  // Multicast.
];

fn ipv4_range_allowed(first: Ipv4Addr, last: Ipv4Addr) -> bool {
    let (first, last) = (u32::from(first), u32::from(last));
    !IPV4_EXCLUDED.iter().any(|(network, prefix)| {
        let start = u32::from(*network);
        let end = start | u32::MAX.checked_shr(*prefix).unwrap_or(0);
        first <= end && start <= last
    })
}

fn ipv6_range_allowed(first: Ipv6Addr, last: Ipv6Addr) -> bool {
    let (first, last) = (u128::from(first), u128::from(last));
    !IPV6_EXCLUDED.iter().any(|(network, prefix)| {
        let start = u128::from(*network);
        let end = start | u128::MAX.checked_shr(*prefix).unwrap_or(0);
        first <= end && start <= last
    })
}

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
                    && ipv4_range_allowed(net.network(), net.broadcast()) =>
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
                    && ipv6_range_allowed(net.network(), net.broadcast()) =>
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
            if !ipv4_range_allowed(ip, ip)
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
            if !ipv6_range_allowed(ip, ip) || !v6.iter().any(|p| p.contains(&ip)) {
                check.issue(&path, "address outside usable pool");
            }
            if !used.insert(ip.into()) {
                check.issue(path, "duplicate address");
            }
        }
    }
}
