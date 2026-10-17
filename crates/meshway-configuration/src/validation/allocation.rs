use super::*;
use ipnet::{Ipv4Net, Ipv6Net};
use std::net::IpAddr;

/// Deterministic, bounded planning. No reservation is made here. The caller
/// supplies every not-yet-retired address and commits under the same Config CAS.
pub fn allocate_addresses(
    input: &mut NetworkConfigurationInput,
    current: &NetworkConfiguration,
    unavailable: &BTreeSet<IpAddr>,
    reassign: &BTreeSet<(NodeId, AddressFamily)>,
) -> Result<BTreeMap<NodeId, NodeAddresses>, ConfigurationError> {
    let mut check = Check::default();
    let pools = addresses::pools(
        input.address_pools.clone(),
        Some(&current.address_pools),
        &mut check,
    );
    check.finish()?;
    let v4 = pools
        .ipv4
        .iter()
        .map(|p| p.parse::<Ipv4Net>().expect("validated pool"))
        .collect::<Vec<_>>();
    let v6 = pools
        .ipv6
        .iter()
        .map(|p| p.parse::<Ipv6Net>().expect("validated pool"))
        .collect::<Vec<_>>();
    let mut used = unavailable.clone();
    for node in current.nodes.values() {
        used.extend(node.addresses.ipv4.map(IpAddr::V4));
        used.extend(node.addresses.ipv6.map(IpAddr::V6));
    }
    for node in input.nodes.values() {
        if let Input::Value(Some(ip)) = node.addresses.ipv4 {
            used.insert(ip.into());
        }
        if let Input::Value(Some(ip)) = node.addresses.ipv6 {
            used.insert(ip.into());
        }
    }
    for (id, _) in reassign {
        if !input.nodes.contains_key(id) {
            check.issue(
                ConfigurationIssue::new(
                    format!("/nodes/{id}"),
                    IssueCode::UnknownReassignmentTarget,
                )
                .with("node", id),
            );
        }
    }
    check.finish()?;
    let mut allocations = BTreeMap::new();
    for (id, node) in &mut input.nodes {
        let old = current.nodes.get(id);
        let allocate4 = reassign.contains(&(*id, AddressFamily::Ipv4))
            || (old.is_none() && node.addresses.ipv4.is_omitted());
        let allocate6 = reassign.contains(&(*id, AddressFamily::Ipv6))
            || (old.is_none() && node.addresses.ipv6.is_omitted());
        if allocate4 {
            let selected = v4
                .iter()
                .flat_map(|p| p.hosts())
                .take(used.len() + 1)
                .find(|ip| !used.contains(&IpAddr::V4(*ip)));
            if selected.is_none() && !v4.is_empty() {
                check.issue(
                    ConfigurationIssue::new(
                        format!("/nodes/{id}/addresses/ipv4"),
                        IssueCode::NoAddressAvailable,
                    )
                    .with("family", "ipv4"),
                );
            }
            used.extend(selected.map(IpAddr::V4));
            node.addresses.ipv4 = Input::Value(selected);
        }
        if allocate6 {
            // Never the pool's all-zero address: it is the Subnet-Router
            // anycast address (RFC 4291 §2.6.1), to which hosts holding the
            // pool route send no ICMPv6 error.
            let selected = v6
                .iter()
                .flat_map(|p| p.hosts().filter(move |ip| *ip != p.network()))
                .take(used.len() + 1)
                .find(|ip| !used.contains(&IpAddr::V6(*ip)));
            if selected.is_none() && !v6.is_empty() {
                check.issue(
                    ConfigurationIssue::new(
                        format!("/nodes/{id}/addresses/ipv6"),
                        IssueCode::NoAddressAvailable,
                    )
                    .with("family", "ipv6"),
                );
            }
            used.extend(selected.map(IpAddr::V6));
            node.addresses.ipv6 = Input::Value(selected);
        }
        if allocate4 || allocate6 {
            allocations.insert(
                *id,
                NodeAddresses {
                    ipv4: match node.addresses.ipv4 {
                        Input::Value(ip) => ip,
                        Input::Omitted => old.and_then(|n| n.addresses.ipv4),
                    },
                    ipv6: match node.addresses.ipv6 {
                        Input::Value(ip) => ip,
                        Input::Omitted => old.and_then(|n| n.addresses.ipv6),
                    },
                },
            );
        }
    }
    check.finish()?;
    Ok(allocations)
}
