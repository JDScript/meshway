//! Multi-network table conversion and global persistent identity integrity.
use crate::{Error, store::NetworkId};
use rusqlite::{Connection, TransactionBehavior, params};

pub(in crate::store) const TABLES: &[(&str, &str)] = &[
    (
        "nodes",
        "id,credential_hash,name,agent_version,registered_at,last_seen,revoked_at,mesh_ipv4,mesh_ipv6,country,acknowledged_revision,acknowledged_digest",
    ),
    (
        "enrollment_tokens",
        "id,token_hash,expires_at,used_node_id,binding_hash,device_binding",
    ),
    (
        "configuration",
        "id,revision,ipv4_prefix,ipv6_prefix,policy,source_jsonc",
    ),
    ("connection_identities", "node_id,public_key,capabilities"),
    ("connection_source", "id,configuration"),
    ("connection_receipts", "node_id,revision,digest,saved_at"),
    ("connection_runtime_leases", "node_id,instance_id,epoch"),
    (
        "connection_runtime_reports",
        "node_id,sequence,body,reported_at",
    ),
    ("mesh_access_source", "id,policy,source_jsonc"),
    ("mesh_access_legacy_v5", "id,policy"),
    ("tag_definitions", "tag,description"),
    ("node_tags", "node_id,tag"),
    ("network_identity", "id,network_id,name,dns"),
];

pub(in crate::store) fn upgrade(connection: &mut Connection) -> Result<(), Error> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == 9 {
        return Ok(());
    }
    if version != 8 {
        return Err(Error::Configuration(
            "multi-network upgrade requires schema 8",
        ));
    }
    connection.pragma_update(None, "foreign_keys", false)?;
    let result = migrate(connection);
    connection.pragma_update(None, "foreign_keys", true)?;
    result
}

fn migrate(connection: &mut Connection) -> Result<(), Error> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let id: String = transaction.query_row(
        "SELECT network_id FROM network_identity WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    for (table, _) in TABLES {
        transaction.execute_batch(&format!("ALTER TABLE {table} RENAME TO {table}_v8;"))?;
    }
    transaction.execute_batch(include_str!("sql/schema_v9.sql"))?;
    transaction.execute(
        "INSERT INTO networks(network_id,created_at,initial) VALUES (?1,?2,1)",
        params![id, chrono::Utc::now().to_rfc3339()],
    )?;
    for (table, columns) in TABLES {
        if *table == "network_identity" {
            transaction.execute(
                &format!("INSERT INTO {table} ({columns}) SELECT {columns} FROM {table}_v8"),
                [],
            )?;
        } else {
            transaction.execute(
                &format!(
                    "INSERT INTO {table} ({columns},network_id) SELECT {columns},?1 FROM {table}_v8"
                ),
                [&id],
            )?;
        }
    }
    for (table, _) in TABLES.iter().rev() {
        transaction.execute_batch(&format!("DROP TABLE {table}_v8;"))?;
    }
    validate_integrity(&transaction)?;
    transaction.execute_batch(include_str!("sql/identity_guards.sql"))?;
    if transaction
        .prepare("PRAGMA foreign_key_check")?
        .exists([])?
    {
        return Err(Error::Configuration(
            "network migration foreign key validation failed",
        ));
    }
    transaction.pragma_update(None, "user_version", 9)?;
    transaction.commit()?;
    Ok(())
}

/// Includes revoked identities, semantic address duplicates and FK integrity.
pub(super) fn validate_integrity(connection: &Connection) -> Result<(), Error> {
    use std::{
        collections::BTreeSet,
        net::{Ipv4Addr, Ipv6Addr},
    };
    let mut addresses4 = BTreeSet::new();
    let mut addresses6 = BTreeSet::new();
    let mut query = connection.prepare("SELECT n.network_id,n.mesh_ipv4,n.mesh_ipv6,c.ipv4_prefix,c.ipv6_prefix FROM nodes n LEFT JOIN configuration c ON c.network_id=n.network_id AND c.id=1 ORDER BY n.id")?;
    let mut rows = query.query([])?;
    while let Some(row) = rows.next()? {
        let network = NetworkId::read(row, 0)?;
        let invalid =
            || Error::Configuration("node has invalid or missing persistent mesh addresses");
        let ipv4: Ipv4Addr = row
            .get::<_, Option<String>>(1)?
            .ok_or_else(invalid)?
            .parse()
            .map_err(|_| invalid())?;
        let ipv6: Ipv6Addr = row
            .get::<_, Option<String>>(2)?
            .ok_or_else(invalid)?
            .parse()
            .map_err(|_| invalid())?;
        let pool = meshway_protocol::MeshNetwork {
            ipv4_prefix: row.get::<_, Option<String>>(3)?.ok_or_else(invalid)?,
            ipv6_prefix: row.get::<_, Option<String>>(4)?.ok_or_else(invalid)?,
        };
        let (v4, v6) = crate::store::ipam::parse(&pool)?;
        if !v4.contains(&ipv4)
            || ipv4 == v4.network()
            || ipv4 == v4.broadcast()
            || ipv4.is_unspecified()
            || ipv4.is_loopback()
            || ipv4.is_multicast()
            || ipv4.is_link_local()
            || !v6.contains(&ipv6)
            || ipv6 == v6.network()
            || !addresses4.insert((network.as_str().to_owned(), ipv4))
            || !addresses6.insert((network.as_str().to_owned(), ipv6))
        {
            return Err(invalid());
        }
    }
    if connection.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(Error::Configuration(
            "persisted network relationships fail foreign key validation",
        ));
    }
    Ok(())
}

pub(super) fn install_identity_guards(connection: &Connection) -> Result<(), Error> {
    connection.execute_batch(include_str!("sql/identity_guards.sql"))?;
    Ok(())
}
