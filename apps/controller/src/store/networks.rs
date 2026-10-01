//! Durable row-level network ownership. Connection-local views are the only
//! business-table surface used by Store: reads and writes share one scope under
//! its mutex, including every compiler and transaction. Physical tables retain
//! cross-network uniqueness for identities and composite ownership foreign keys.
use meshway_protocol::{
    CreateNetworkRequest, MeshDnsConfiguration, NetworkList, NetworkSnapshot, local,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::Store;
use crate::Error;

#[cfg(test)]
mod tests;

const TABLES: &[&str] = &[
    "nodes",
    "enrollment_tokens",
    "configuration",
    "connection_identities",
    "connection_source",
    "connection_receipts",
    "connection_runtime_leases",
    "connection_runtime_reports",
    "mesh_access_source",
    "mesh_access_legacy_v5",
    "tag_definitions",
    "node_tags",
    "network_identity",
];

fn version(connection: &Connection) -> Result<u32, Error> {
    Ok(connection.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

pub(super) fn upgrade(connection: &mut Connection) -> Result<(), Error> {
    if version(connection)? == 9 {
        return Ok(());
    }
    if version(connection)? != 8 {
        return Err(Error::Configuration(
            "multi-network upgrade requires schema 8",
        ));
    }
    // SQLite cannot rebuild referenced tables with FK checking enabled. Disable
    // it before BEGIN, validate the complete result before COMMIT, then restore
    // it on both success and error. DDL and all row copies roll back together.
    connection.pragma_update(None, "foreign_keys", false)?;
    let result = migrate(connection);
    connection.pragma_update(None, "foreign_keys", true)?;
    result
}

fn migrate(connection: &mut Connection) -> Result<(), Error> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let network_id: String = transaction.query_row(
        "SELECT network_id FROM network_identity WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    let schemas: Vec<(String, String)> = TABLES
        .iter()
        .map(|table| {
            Ok((
                (*table).to_owned(),
                transaction.query_row(
                    "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get::<_, String>(0),
                )?,
            ))
        })
        .collect::<Result<_, Error>>()?;
    transaction.execute_batch("CREATE TABLE networks (network_id TEXT PRIMARY KEY CHECK(length(network_id)=32), created_at TEXT NOT NULL, initial INTEGER NOT NULL CHECK(initial IN (0,1))); CREATE UNIQUE INDEX initial_network ON networks(initial) WHERE initial=1;")?;
    transaction.execute(
        "INSERT INTO networks VALUES (?1,?2,1)",
        params![network_id, chrono::Utc::now().to_rfc3339()],
    )?;
    for (table, _) in &schemas {
        transaction.execute_batch(&format!("ALTER TABLE {table} RENAME TO {table}_v8;"))?;
    }
    // Explicit table/index transformations preserve all legacy column values,
    // checks, global node IDs, credentials and public-key uniqueness.
    for (table, original) in &schemas {
        let open = original
            .find('(')
            .ok_or(Error::Configuration("invalid legacy table schema"))?;
        let mut sql = format!("CREATE TABLE {table}_scoped {}", &original[open..]);
        sql = sql.replace("id INTEGER PRIMARY KEY", "id INTEGER NOT NULL");
        sql = sql.replace("tag TEXT PRIMARY KEY", "tag TEXT NOT NULL");
        sql = sql.replace("REFERENCES nodes(id)", "REFERENCES nodes_scoped(id)");
        sql = sql.replace(
            "REFERENCES connection_runtime_leases(node_id)",
            "REFERENCES connection_runtime_leases_scoped(node_id)",
        );
        sql = sql.replace(
            "tag TEXT NOT NULL REFERENCES tag_definitions(tag)",
            "tag TEXT NOT NULL",
        );
        if table != "network_identity" {
            let open = sql
                .find('(')
                .ok_or(Error::Configuration("invalid legacy table schema"))?;
            sql.insert_str(
                open + 1,
                "network_id TEXT NOT NULL REFERENCES networks(network_id),",
            );
        }
        let close = sql
            .rfind(')')
            .ok_or(Error::Configuration("invalid legacy table schema"))?;
        let mut extra = String::new();
        if [
            "configuration",
            "connection_source",
            "mesh_access_source",
            "mesh_access_legacy_v5",
            "network_identity",
        ]
        .contains(&table.as_str())
        {
            extra.push_str(", PRIMARY KEY(network_id,id)");
        }
        if table == "network_identity" {
            extra.push_str(", FOREIGN KEY(network_id) REFERENCES networks(network_id)");
        }
        if table == "nodes" {
            extra.push_str(", UNIQUE(network_id,id), UNIQUE(network_id,mesh_ipv4), UNIQUE(network_id,mesh_ipv6)");
        }
        if table == "tag_definitions" {
            extra.push_str(", PRIMARY KEY(network_id,tag)");
        }
        if table == "connection_runtime_leases" {
            extra.push_str(", UNIQUE(network_id,node_id)");
        }
        if [
            "connection_identities",
            "connection_receipts",
            "connection_runtime_leases",
            "node_tags",
        ]
        .contains(&table.as_str())
        {
            extra.push_str(
                ", FOREIGN KEY(network_id,node_id) REFERENCES nodes_scoped(network_id,id)",
            );
        }
        if table == "connection_runtime_reports" {
            extra.push_str(", FOREIGN KEY(network_id,node_id) REFERENCES connection_runtime_leases_scoped(network_id,node_id)");
        }
        if table == "enrollment_tokens" {
            extra.push_str(
                ", FOREIGN KEY(network_id,used_node_id) REFERENCES nodes_scoped(network_id,id)",
            );
        }
        if table == "node_tags" {
            extra.push_str(
                ", FOREIGN KEY(network_id,tag) REFERENCES tag_definitions_scoped(network_id,tag)",
            );
        }
        sql.insert_str(close, &extra);
        transaction.execute_batch(&sql)?;
    }
    for (table, _) in &schemas {
        let columns = transaction
            .prepare(&format!("PRAGMA table_info({table}_v8)"))?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?
            .join(",");
        let sql = if table == "network_identity" {
            format!("INSERT INTO {table}_scoped SELECT * FROM {table}_v8")
        } else {
            format!(
                "INSERT INTO {table}_scoped ({columns},network_id) SELECT *, ?1 FROM {table}_v8"
            )
        };
        if table == "network_identity" {
            transaction.execute(&sql, [])?;
        } else {
            transaction.execute(&sql, [&network_id])?;
        }
    }
    for (table, _) in schemas.iter().rev() {
        transaction.execute_batch(&format!("DROP TABLE {table}_v8;"))?;
    }
    let invalid = transaction
        .prepare("PRAGMA foreign_key_check")?
        .exists([])?;
    if invalid {
        return Err(Error::Configuration(
            "network migration foreign key validation failed",
        ));
    }
    transaction.pragma_update(None, "user_version", 9)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn install_scope(connection: &Connection) -> Result<(), Error> {
    connection.execute_batch("CREATE TEMP TABLE network_scope (network_id TEXT NOT NULL); INSERT INTO network_scope SELECT network_id FROM main.networks WHERE initial=1;")?;
    for table in TABLES {
        let columns = connection
            .prepare(&format!("PRAGMA main.table_info({table}_scoped)"))?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, u32>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let visible: Vec<_> = columns
            .iter()
            .filter(|(name, _, _)| name != "network_id" || *table == "network_identity")
            .collect();
        let names = visible
            .iter()
            .map(|(name, _, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let writes: Vec<_> = columns
            .iter()
            .map(|(name, default, _)| {
                if name == "network_id" {
                    "(SELECT network_id FROM network_scope)".to_owned()
                } else if let Some(default) = default {
                    format!("COALESCE(NEW.{name},{default})")
                } else {
                    format!("NEW.{name}")
                }
            })
            .collect();
        let all_names = columns
            .iter()
            .map(|(name, _, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let keys = columns
            .iter()
            .filter(|(name, _, pk)| *pk > 0 && name != "network_id")
            .map(|(name, _, _)| format!("{name}=OLD.{name}"))
            .collect::<Vec<_>>()
            .join(" AND ");
        let changes = visible
            .iter()
            .filter(|(name, _, _)| name != "network_id")
            .map(|(name, _, _)| format!("{name}=NEW.{name}"))
            .collect::<Vec<_>>()
            .join(",");
        connection.execute_batch(&format!(
            "CREATE TEMP VIEW {table} AS SELECT {names} FROM main.{table}_scoped WHERE network_id=(SELECT network_id FROM network_scope);
             CREATE TEMP TRIGGER {table}_insert INSTEAD OF INSERT ON {table} BEGIN INSERT INTO {table}_scoped ({all_names}) VALUES ({}); END;
             CREATE TEMP TRIGGER {table}_update INSTEAD OF UPDATE ON {table} BEGIN UPDATE {table}_scoped SET {changes} WHERE {keys} AND network_id=(SELECT network_id FROM network_scope); END;
             CREATE TEMP TRIGGER {table}_delete INSTEAD OF DELETE ON {table} BEGIN DELETE FROM {table}_scoped WHERE {keys} AND network_id=(SELECT network_id FROM network_scope); END;", writes.join(",")))?;
    }
    Ok(())
}

pub(super) fn credential_bound(connection: &Connection, hash: &[u8]) -> Result<bool, Error> {
    let suffix = if version(connection)? == 9 {
        "_scoped"
    } else {
        ""
    };
    Ok(connection.query_row(&format!("SELECT EXISTS(SELECT 1 FROM main.nodes{suffix} WHERE credential_hash=?1 UNION ALL SELECT 1 FROM settings WHERE admin_hash=?1 UNION ALL SELECT 1 FROM main.enrollment_tokens{suffix} WHERE token_hash=?1)"), [hash], |row| row.get(0))?)
}

pub(super) fn select_node(connection: &Connection, token: &str) -> Result<(), Error> {
    if version(connection)? < 9 {
        return Ok(());
    }
    let id: String = connection
        .query_row(
            "SELECT network_id FROM main.nodes_scoped WHERE credential_hash=?1",
            [local::digest(token)],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(Error::Unauthorized)?;
    connection.execute("UPDATE temp.network_scope SET network_id=?1", [id])?;
    Ok(())
}

pub(super) fn public_key_bound(
    connection: &Connection,
    key: &str,
    id: &str,
) -> Result<bool, Error> {
    let suffix = if version(connection)? == 9 {
        "_scoped"
    } else {
        ""
    };
    Ok(connection.query_row(&format!("SELECT EXISTS(SELECT 1 FROM main.connection_identities{suffix} WHERE public_key=?1 AND node_id!=?2)"), params![key,id], |row| row.get(0))?)
}

impl Store {
    pub fn select_network(&self, id: &str) -> Result<(), Error> {
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM main.networks WHERE network_id=?1)",
            [id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(Error::NotFound);
        }
        self.connection
            .execute("UPDATE temp.network_scope SET network_id=?1", [id])?;
        Ok(())
    }

    fn select_credential(&self, token: &str, enrollment: bool) -> Result<(), Error> {
        if version(&self.connection)? < 9 {
            return Ok(());
        }
        let (table, column) = if enrollment {
            ("enrollment_tokens_scoped", "token_hash")
        } else {
            ("nodes_scoped", "credential_hash")
        };
        let id: String = self
            .connection
            .query_row(
                &format!("SELECT network_id FROM main.{table} WHERE {column}=?1"),
                [local::digest(token)],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(Error::Unauthorized)?;
        self.select_network(&id)
    }

    pub fn select_node(&self, token: &str) -> Result<(), Error> {
        self.select_credential(token, false)
    }
    pub(super) fn select_enrollment(&self, token: &str) -> Result<(), Error> {
        self.select_credential(token, true)
    }

    pub fn networks(&mut self) -> Result<NetworkList, Error> {
        let previous: String =
            self.connection
                .query_row("SELECT network_id FROM temp.network_scope", [], |row| {
                    row.get(0)
                })?;
        let ids = self
            .connection
            .prepare(
                "SELECT network_id FROM main.networks ORDER BY initial DESC,created_at,network_id",
            )?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let result = ids
            .into_iter()
            .map(|id| {
                self.select_network(&id)?;
                self.network()
            })
            .collect::<Result<Vec<_>, Error>>();
        self.select_network(&previous)?;
        Ok(NetworkList { networks: result? })
    }

    pub fn create_network(
        &mut self,
        request: &CreateNetworkRequest,
    ) -> Result<NetworkSnapshot, Error> {
        request.validate().map_err(Error::Unprocessable)?;
        let id = local::credential()?[..32].to_owned();
        let random = local::credential()?;
        let ipv6 = format!(
            "fd{}:{}:{}::/64",
            &random[..2],
            &random[2..6],
            &random[6..10]
        );
        let dns = MeshDnsConfiguration {
            enabled: false,
            zone: format!("net-{id}.meshway.test"),
            ttl_seconds: 30,
        };
        let dns = serde_json::to_string(&dns)
            .map_err(|_| Error::Configuration("cannot encode network DNS"))?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO main.networks VALUES (?1,?2,0)",
            params![id, chrono::Utc::now().to_rfc3339()],
        )?;
        transaction.execute(
            "INSERT INTO main.network_identity_scoped (id,network_id,name,dns) VALUES (1,?1,?2,?3)",
            params![id, request.name, dns],
        )?;
        transaction.execute("INSERT INTO main.configuration_scoped (id,revision,ipv4_prefix,ipv6_prefix,policy,source_jsonc,network_id) VALUES (1,1,'100.96.0.0/16',?1,'{\"default_action\":\"reject\",\"rules\":[]}',NULL,?2)", params![ipv6,id])?;
        transaction.execute("INSERT INTO main.connection_source_scoped (id,configuration,network_id) VALUES (1,'{\"listeners\":[],\"candidates\":[]}',?1)", [&id])?;
        transaction.execute("INSERT INTO main.mesh_access_source_scoped (id,policy,source_jsonc,network_id) VALUES (1,'{\"default_action\":\"allow\",\"rules\":[]}',NULL,?1)", [&id])?;
        transaction.execute("INSERT INTO main.mesh_access_legacy_v5_scoped (id,policy,network_id) VALUES (1,'{\"grants\":[]}',?1)", [&id])?;
        transaction.commit()?;
        let previous: String =
            self.connection
                .query_row("SELECT network_id FROM temp.network_scope", [], |row| {
                    row.get(0)
                })?;
        self.select_network(&id)?;
        let result = self.network();
        self.select_network(&previous)?;
        result
    }
}
