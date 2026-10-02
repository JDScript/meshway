use std::{
    collections::BTreeSet,
    net::{Ipv4Addr, Ipv6Addr},
};

use ipnet::{Ipv4Net, Ipv6Net};
use meshway_protocol::MeshNetwork;
use rusqlite::Connection;

use super::NetworkId;
use crate::{Error, NetworkOptions};

pub(super) fn parse(network: &MeshNetwork) -> Result<(Ipv4Net, Ipv6Net), Error> {
    let v4: Ipv4Net = network
        .ipv4_prefix
        .parse()
        .map_err(|_| Error::Configuration("invalid IPv4 prefix"))?;
    let v6: Ipv6Net = network
        .ipv6_prefix
        .parse()
        .map_err(|_| Error::Configuration("invalid IPv6 prefix"))?;
    if v4.prefix_len() > 30 || v4.addr() != v4.network() {
        return Err(Error::Configuration(
            "IPv4 prefix must be canonical and have usable host addresses",
        ));
    }
    if v6.prefix_len() != 64 || v6.addr() != v6.network() || (u128::from(v6.addr()) >> 120) != 0xfd
    {
        return Err(Error::Configuration(
            "IPv6 prefix must be a canonical private fd00::/8 ULA /64",
        ));
    }
    Ok((v4, v6))
}

pub(super) fn network(connection: &Connection, network: &NetworkId) -> Result<MeshNetwork, Error> {
    Ok(connection.query_row(
        "SELECT ipv4_prefix, ipv6_prefix FROM configuration WHERE id = 1 AND network_id=?1",
        [network.as_str()],
        |row| {
            Ok(MeshNetwork {
                ipv4_prefix: row.get(0)?,
                ipv6_prefix: row.get(1)?,
            })
        },
    )?)
}

pub(super) fn check_options(
    connection: &Connection,
    scope: &NetworkId,
    options: &NetworkOptions,
) -> Result<(), Error> {
    compare_options(&network(connection, scope)?, options)
}

pub(in crate::store) fn compare_options(
    stored: &MeshNetwork,
    options: &NetworkOptions,
) -> Result<(), Error> {
    let (v4, v6) = parse(stored)?;
    if options
        .ipv4_prefix
        .as_ref()
        .is_some_and(|value| value.parse::<Ipv4Net>().ok() != Some(v4))
        || options
            .ipv6_prefix
            .as_ref()
            .is_some_and(|value| value.parse::<Ipv6Net>().ok() != Some(v6))
    {
        return Err(Error::Configuration(
            "explicit network prefix differs from persisted network; refusing to renumber",
        ));
    }
    Ok(())
}

pub(super) fn allocate(
    connection: &Connection,
    scope: &NetworkId,
) -> Result<(String, String), Error> {
    let used: BTreeSet<String> = connection
        .prepare("SELECT mesh_ipv4 FROM nodes WHERE mesh_ipv4 IS NOT NULL AND network_id=?1")?
        .query_map([scope.as_str()], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    allocate_used(&network(connection, scope)?, &used)
}

pub(in crate::store) fn allocate_used(
    network: &MeshNetwork,
    used: &BTreeSet<String>,
) -> Result<(String, String), Error> {
    let (v4, v6) = parse(network)?;
    if used.len() >= meshway_protocol::MAX_INVENTORY_NODES {
        return Err(Error::Capacity("node inventory limit reached"));
    }
    let base = u32::from(v4.network());
    // At most inventory+1 candidates are needed; revoked addresses remain used.
    for offset in 1..=u32::try_from(used.len() + 1).expect("bounded inventory") {
        let address = base
            .checked_add(offset)
            .map(Ipv4Addr::from)
            .ok_or(Error::Capacity("IPv4 address pool exhausted"))?;
        if address >= v4.broadcast() {
            return Err(Error::Capacity("IPv4 address pool exhausted"));
        }
        if !used.contains(&address.to_string()) {
            let ipv6 = Ipv6Addr::from(u128::from(v6.network()) + u128::from(offset));
            return Ok((address.to_string(), ipv6.to_string()));
        }
    }
    Err(Error::Capacity("IPv4 address pool exhausted"))
}
