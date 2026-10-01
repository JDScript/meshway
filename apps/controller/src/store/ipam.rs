use std::{
    collections::BTreeSet,
    net::{Ipv4Addr, Ipv6Addr},
};

use ipnet::{Ipv4Net, Ipv6Net};
use meshway_protocol::{MeshNetwork, local};
use rusqlite::{Connection, TransactionBehavior, params};

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

fn initial_network(options: &NetworkOptions) -> Result<MeshNetwork, Error> {
    let random = local::credential()?;
    // RFC 4193: locally assigned /48 with a random 40-bit global ID, subnet 0.
    let ula = format!(
        "fd{}:{}:{}::/64",
        &random[..2],
        &random[2..6],
        &random[6..10]
    );
    let network = MeshNetwork {
        ipv4_prefix: options
            .ipv4_prefix
            .clone()
            .unwrap_or_else(|| "100.96.0.0/16".into()),
        ipv6_prefix: options.ipv6_prefix.clone().unwrap_or(ula),
    };
    let (v4, v6) = parse(&network)?;
    Ok(MeshNetwork {
        ipv4_prefix: v4.to_string(),
        ipv6_prefix: v6.to_string(),
    })
}

/// Additive, all-or-nothing upgrade; includes revoked identities in allocation.
pub(super) fn upgrade(connection: &mut Connection, options: &NetworkOptions) -> Result<(), Error> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == 1 {
        let network = initial_network(options)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "CREATE TABLE configuration (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 4294967295),
                ipv4_prefix TEXT NOT NULL, ipv6_prefix TEXT NOT NULL, policy TEXT NOT NULL
             );
             ALTER TABLE nodes ADD COLUMN mesh_ipv4 TEXT;
             ALTER TABLE nodes ADD COLUMN mesh_ipv6 TEXT;
             ALTER TABLE nodes ADD COLUMN country TEXT;
             ALTER TABLE nodes ADD COLUMN acknowledged_revision INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE nodes ADD COLUMN acknowledged_digest TEXT;
             CREATE UNIQUE INDEX node_ipv4 ON nodes(mesh_ipv4);
             CREATE UNIQUE INDEX node_ipv6 ON nodes(mesh_ipv6);",
        )?;
        transaction.execute(
            "INSERT INTO configuration VALUES (1, 1, ?1, ?2, ?3)",
            params![
                network.ipv4_prefix,
                network.ipv6_prefix,
                r#"{"default_action":"reject","rules":[]}"#
            ],
        )?;
        let ids: Vec<String> = transaction
            .prepare("SELECT id FROM nodes ORDER BY id")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        if ids.len() > meshway_protocol::MAX_INVENTORY_NODES {
            return Err(Error::Capacity(
                "existing inventory exceeds configuration capacity",
            ));
        }
        for id in ids {
            let used = transaction
                .prepare("SELECT mesh_ipv4 FROM nodes WHERE mesh_ipv4 IS NOT NULL")?
                .query_map([], |row| row.get(0))?
                .collect::<Result<BTreeSet<String>, _>>()?;
            let (v4, v6) = allocate_used(&network, &used)?;
            transaction.execute(
                "UPDATE nodes SET mesh_ipv4 = ?1, mesh_ipv6 = ?2 WHERE id = ?3",
                params![v4, v6, id],
            )?;
        }
        transaction.pragma_update(None, "user_version", 2)?;
        transaction.commit()?;
    } else if !(2..=8).contains(&version) {
        return Err(Error::Configuration(
            "unsupported or incomplete controller database schema",
        ));
    }
    let stored = connection.query_row(
        "SELECT ipv4_prefix,ipv6_prefix FROM configuration WHERE id=1",
        [],
        |row| {
            Ok(MeshNetwork {
                ipv4_prefix: row.get(0)?,
                ipv6_prefix: row.get(1)?,
            })
        },
    )?;
    compare_options(&stored, options)
}

pub(super) fn check_options(
    connection: &Connection,
    scope: &NetworkId,
    options: &NetworkOptions,
) -> Result<(), Error> {
    compare_options(&network(connection, scope)?, options)
}

fn compare_options(stored: &MeshNetwork, options: &NetworkOptions) -> Result<(), Error> {
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

fn allocate_used(
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
