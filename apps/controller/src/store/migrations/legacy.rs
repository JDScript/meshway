//! The original single-network schema path (1 -> 8), with its original
//! per-step transactions and revision semantics.
use crate::store::ipam;
use crate::{Error, NetworkOptions};
use meshway_protocol::*;
use rusqlite::{Connection, TransactionBehavior, params};
use std::collections::{BTreeMap, BTreeSet};

fn initial_network(options: &NetworkOptions) -> Result<MeshNetwork, Error> {
    let random = local::credential()?;
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
    let (v4, v6) = ipam::parse(&network)?;
    Ok(MeshNetwork {
        ipv4_prefix: v4.to_string(),
        ipv6_prefix: v6.to_string(),
    })
}

pub(in crate::store) fn address_pool(
    connection: &mut Connection,
    options: &NetworkOptions,
) -> Result<(), Error> {
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
        if ids.len() > MAX_INVENTORY_NODES {
            return Err(Error::Capacity(
                "existing inventory exceeds configuration capacity",
            ));
        }
        for id in ids {
            let used = transaction
                .prepare("SELECT mesh_ipv4 FROM nodes WHERE mesh_ipv4 IS NOT NULL")?
                .query_map([], |row| row.get(0))?
                .collect::<Result<BTreeSet<String>, _>>()?;
            let (v4, v6) = ipam::allocate_used(&network, &used)?;
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
    ipam::compare_options(&stored, options)
}

fn connections(connection: &mut Connection) -> Result<(), Error> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if (3..=8).contains(&version) {
        return Ok(());
    }
    if version != 2 {
        return Err(Error::Configuration(
            "connection schema upgrade requires schema 2",
        ));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "CREATE TABLE connection_identities (
            node_id TEXT PRIMARY KEY REFERENCES nodes(id), public_key TEXT NOT NULL UNIQUE,
            capabilities TEXT NOT NULL
         );
         CREATE TABLE connection_source (
            id INTEGER PRIMARY KEY CHECK (id = 1), configuration TEXT NOT NULL
         );
         INSERT INTO connection_source VALUES (1, '{\"listeners\":[],\"candidates\":[]}');
         CREATE TABLE connection_receipts (
            node_id TEXT PRIMARY KEY REFERENCES nodes(id),
            revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 4294967295),
            digest TEXT NOT NULL, saved_at TEXT NOT NULL
         );
         PRAGMA user_version = 3;",
    )?;
    transaction.commit()?;
    Ok(())
}

fn runtime(connection: &mut Connection) -> Result<(), Error> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if (4..=8).contains(&version) {
        return Ok(());
    }
    if version != 3 {
        return Err(Error::Configuration(
            "runtime schema upgrade requires schema 3",
        ));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "CREATE TABLE connection_runtime_leases (
            node_id TEXT PRIMARY KEY REFERENCES nodes(id),
            instance_id TEXT NOT NULL,
            epoch INTEGER NOT NULL CHECK (epoch BETWEEN 1 AND 4294967295)
         );
         CREATE TABLE connection_runtime_reports (
            node_id TEXT PRIMARY KEY REFERENCES connection_runtime_leases(node_id),
            sequence INTEGER NOT NULL CHECK (sequence BETWEEN 1 AND 4294967295),
            body BLOB NOT NULL, reported_at TEXT NOT NULL
         );
         PRAGMA user_version = 4;",
    )?;
    transaction.commit()?;
    Ok(())
}

fn valid_legacy_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}
fn migrated_id(ids: impl Iterator<Item = String>, source: &str, destination: &str) -> String {
    ids.filter(|id| valid_legacy_id(id))
        .min()
        .unwrap_or_else(|| {
            local::digest(&format!("legacy:{source}:{destination}"))
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        })
}
fn migrate_access(legacy: MeshPathAuthorization) -> MeshAccessPolicy {
    let mut directed: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for grant in legacy.grants {
        if let (Some(source), Some(destination)) = (grant.path.first(), grant.path.last())
            && source != destination
        {
            directed
                .entry((source.clone(), destination.clone()))
                .or_default()
                .push(grant.id);
        }
    }
    let mut rules = Vec::new();
    while let Some(((source, destination), mut ids)) = directed.pop_first() {
        let reverse = directed.remove(&(destination.clone(), source.clone()));
        let bidirectional = reverse.is_some();
        if let Some(reverse) = reverse {
            ids.extend(reverse);
        }
        rules.push(MeshAccessRule {
            id: migrated_id(ids.into_iter(), &source, &destination),
            source: vec![format!("node:{source}")],
            destination: vec![format!("node:{destination}")],
            action: AccessAction::Allow,
            bidirectional,
        });
    }
    MeshAccessPolicy {
        default_action: AccessAction::Reject,
        rules,
    }
}

fn access(connection: &mut Connection) -> Result<(), Error> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if (6..=8).contains(&version) {
        return Ok(());
    }
    if version == 4 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "CREATE TABLE mesh_access_source (
                id INTEGER PRIMARY KEY CHECK (id = 1), policy TEXT NOT NULL
             );
             INSERT INTO mesh_access_source VALUES (1, '{\"grants\":[]}');
             PRAGMA user_version = 5;",
        )?;
        transaction.commit()?;
    } else if version != 5 {
        return Err(Error::Configuration(
            "access schema upgrade requires schema 4 or 5",
        ));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let legacy_json: String = transaction.query_row(
        "SELECT policy FROM mesh_access_source WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    let legacy: MeshPathAuthorization = serde_json::from_str(&legacy_json)
        .map_err(|_| Error::Configuration("invalid legacy mesh access"))?;
    let revision_must_change = !legacy.grants.is_empty();
    let policy = serde_json::to_string(&migrate_access(legacy))
        .map_err(|_| Error::Configuration("cannot migrate mesh access"))?;
    if transaction.query_row("SELECT revision FROM configuration WHERE id=1", [], |row| {
        row.get::<_, u32>(0)
    })? == u32::MAX
    {
        return Err(Error::Capacity("configuration revision exhausted"));
    }
    transaction.execute_batch(
        "ALTER TABLE mesh_access_source RENAME TO mesh_access_legacy_v5;
         CREATE TABLE mesh_access_source (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            policy TEXT NOT NULL,
            source_jsonc TEXT
         );
         ALTER TABLE configuration ADD COLUMN source_jsonc TEXT;
         CREATE TABLE tag_definitions (
            tag TEXT PRIMARY KEY,
            description TEXT NOT NULL
         );
         CREATE TABLE node_tags (
            node_id TEXT NOT NULL REFERENCES nodes(id),
            tag TEXT NOT NULL REFERENCES tag_definitions(tag),
            PRIMARY KEY(node_id, tag)
         );",
    )?;
    transaction.execute(
        "INSERT INTO mesh_access_source(id,policy,source_jsonc) VALUES (1,?1,NULL)",
        [policy],
    )?;
    if revision_must_change {
        transaction.execute(
            "UPDATE configuration SET revision=revision+1 WHERE id=1",
            [],
        )?;
    }
    transaction.pragma_update(None, "user_version", 6)?;
    transaction.commit()?;
    Ok(())
}

fn network_identity(connection: &mut Connection) -> Result<(), Error> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if (7..=8).contains(&version) {
        return Ok(());
    }
    if version != 6 {
        return Err(Error::Configuration("network upgrade requires schema 6"));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "CREATE TABLE network_identity (
        id INTEGER PRIMARY KEY CHECK (id=1),
        network_id TEXT NOT NULL UNIQUE CHECK (length(network_id)=32),
        name TEXT NOT NULL, dns TEXT NOT NULL
    );",
    )?;
    let network_id = local::credential()?[..32].to_owned();
    let dns = serde_json::to_string(&MeshDnsConfiguration::default())
        .map_err(|_| Error::Configuration("cannot encode network DNS"))?;
    transaction.execute(
        "INSERT INTO network_identity VALUES (1, ?1, 'Meshway', ?2)",
        params![network_id, dns],
    )?;
    transaction.pragma_update(None, "user_version", 7)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn upgrade(
    connection: &mut Connection,
    options: &NetworkOptions,
    original_version: i64,
) -> Result<(), Error> {
    address_pool(connection, options)?;
    connections(connection)?;
    runtime(connection)?;
    access(connection)?;
    network_identity(connection)?;
    if original_version < 8 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch("ALTER TABLE enrollment_tokens ADD COLUMN device_binding TEXT; PRAGMA user_version = 8;")?;
        transaction.commit()?;
    }
    Ok(())
}
