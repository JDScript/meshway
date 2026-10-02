//! Address ownership validation independent of policy source compilation.
use ipnet::{Ipv4Net, Ipv6Net};
use meshway_protocol::{MeshNetwork, MeshNode, local};
use std::{
    collections::BTreeSet,
    net::{Ipv4Addr, Ipv6Addr},
};

pub fn validate_inventory(network: &MeshNetwork, nodes: &[MeshNode]) -> Result<(), &'static str> {
    let v4: Ipv4Net = network
        .ipv4_prefix
        .parse()
        .map_err(|_| "invalid IPv4 pool")?;
    let v6: Ipv6Net = network
        .ipv6_prefix
        .parse()
        .map_err(|_| "invalid IPv6 pool")?;
    if v4.prefix_len() > 30
        || v4.addr() != v4.network()
        || v6.prefix_len() != 64
        || v6.addr() != v6.network()
        || u128::from(v6.addr()) >> 120 != 0xfd
    {
        return Err("invalid canonical Mesh pools");
    }
    if nodes.len() > meshway_protocol::MAX_INVENTORY_NODES {
        return Err("inventory exceeds capacity");
    }
    let mut ids = BTreeSet::new();
    let mut addresses4 = BTreeSet::new();
    let mut addresses6 = BTreeSet::new();
    for node in nodes {
        let a: Ipv4Addr = node.mesh_ipv4.parse().map_err(|_| "invalid Mesh IPv4")?;
        let b: Ipv6Addr = node.mesh_ipv6.parse().map_err(|_| "invalid Mesh IPv6")?;
        if !local::valid_credential(&node.id)
            || !ids.insert(&node.id)
            || !v4.contains(&a)
            || a == v4.network()
            || a == v4.broadcast()
            || a.is_unspecified()
            || a.is_loopback()
            || a.is_multicast()
            || a.is_link_local()
            || !v6.contains(&b)
            || b == v6.network()
            || !addresses4.insert(a)
            || !addresses6.insert(b)
        {
            return Err("invalid or duplicate Mesh endpoint ownership");
        }
    }
    Ok(())
}
