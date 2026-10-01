//! Explicit, immutable network IDs and the auditable v8 -> v9 migration.
use super::Store;
use crate::Error;
use meshway_protocol::{
    CreateNetworkRequest, MeshDnsConfiguration, NetworkList, NetworkSnapshot, local,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

#[cfg(test)]
mod tests;

/// Only Store's validated resource/credential lookup can construct an ID.
/// Every administrative business method requires this type explicitly; there
/// is no selected network, no implicit default and no mutable connection scope.
/// The tuple field is private to this module; there is no From<String>, Default
/// or Deserialize implementation. API modules cannot access Store.connection.
/// Omitting this argument from Store::nodes/count/revision/policy/etc. is a
/// compiler error, not an ambient-scope fallback. Agent entry points construct
/// the same type through the globally unique credential association.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NetworkId(String);

impl NetworkId {
    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
    fn stored(value: String) -> Result<Self, Error> {
        if value.len() != 32
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Error::Configuration("invalid persisted network identity"));
        }
        Ok(Self(value))
    }
    pub(super) fn initial(connection: &Connection) -> Result<Self, Error> {
        Self::stored(connection.query_row(
            "SELECT network_id FROM networks WHERE initial=1",
            [],
            |row| row.get(0),
        )?)
    }
}

pub(super) const TABLES: &[(&str, &str)] = &[
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

pub(super) fn upgrade(connection: &mut Connection) -> Result<(), Error> {
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
    transaction.execute_batch(include_str!("schema_v9.sql"))?;
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

/// Deliberately global identity uniqueness checks, separate from scoped reads.
pub(super) fn credential_bound(connection: &Connection, hash: &[u8]) -> Result<bool, Error> {
    Ok(connection.query_row("SELECT EXISTS(SELECT 1 FROM nodes WHERE credential_hash=?1 UNION ALL SELECT 1 FROM settings WHERE admin_hash=?1 UNION ALL SELECT 1 FROM enrollment_tokens WHERE token_hash=?1)", [hash], |row|row.get(0))?)
}
pub(super) fn public_key_bound(
    connection: &Connection,
    key: &str,
    id: &str,
) -> Result<bool, Error> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM connection_identities WHERE public_key=?1 AND node_id!=?2)",
        params![key, id],
        |row| row.get(0),
    )?)
}

impl Store {
    pub fn network_id(&self, id: &str) -> Result<NetworkId, Error> {
        let found: Option<String> = self
            .connection
            .query_row(
                "SELECT network_id FROM networks WHERE network_id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        NetworkId::stored(found.ok_or(Error::NotFound)?)
    }
    #[cfg(test)]
    pub(super) fn initial_network_id(&self) -> Result<NetworkId, Error> {
        NetworkId::initial(&self.connection)
    }
    pub(super) fn node_network(&self, token: &str) -> Result<NetworkId, Error> {
        self.credential_network(token, false)
    }
    pub(super) fn enrollment_network(&self, token: &str) -> Result<NetworkId, Error> {
        self.credential_network(token, true)
    }
    fn credential_network(&self, token: &str, enrollment: bool) -> Result<NetworkId, Error> {
        let sql = if enrollment {
            "SELECT network_id FROM enrollment_tokens WHERE token_hash=?1"
        } else {
            "SELECT network_id FROM nodes WHERE credential_hash=?1"
        };
        NetworkId::stored(
            self.connection
                .query_row(sql, [local::digest(token)], |row| row.get::<_, String>(0))
                .optional()?
                .ok_or(Error::Unauthorized)?,
        )
    }
    pub fn networks(&mut self) -> Result<NetworkList, Error> {
        let ids = self
            .connection
            .prepare("SELECT network_id FROM networks ORDER BY initial DESC,created_at,network_id")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let networks = ids
            .into_iter()
            .map(|id| self.network(&NetworkId::stored(id)?))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(NetworkList { networks })
    }
    pub fn create_network(
        &mut self,
        request: &CreateNetworkRequest,
    ) -> Result<NetworkSnapshot, Error> {
        request.validate().map_err(Error::Unprocessable)?;
        let id = NetworkId(local::credential()?[..32].to_owned());
        let random = local::credential()?;
        let ipv6 = format!(
            "fd{}:{}:{}::/64",
            &random[..2],
            &random[2..6],
            &random[6..10]
        );
        let dns = serde_json::to_string(&MeshDnsConfiguration {
            enabled: false,
            zone: format!("net-{}.meshway.test", id.as_str()),
            ttl_seconds: 30,
        })
        .map_err(|_| Error::Configuration("cannot encode network DNS"))?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO networks(network_id,created_at,initial) VALUES (?1,?2,0)",
            params![id.as_str(), chrono::Utc::now().to_rfc3339()],
        )?;
        transaction.execute(
            "INSERT INTO network_identity(id,network_id,name,dns) VALUES(1,?1,?2,?3)",
            params![id.as_str(), request.name, dns],
        )?;
        transaction.execute("INSERT INTO configuration(id,revision,ipv4_prefix,ipv6_prefix,policy,source_jsonc,network_id) VALUES(1,1,'100.96.0.0/16',?1,'{\"default_action\":\"reject\",\"rules\":[]}',NULL,?2)",params![ipv6,id.as_str()])?;
        transaction.execute("INSERT INTO connection_source(id,configuration,network_id) VALUES(1,'{\"listeners\":[],\"candidates\":[]}',?1)",[id.as_str()])?;
        transaction.execute("INSERT INTO mesh_access_source(id,policy,source_jsonc,network_id) VALUES(1,'{\"default_action\":\"allow\",\"rules\":[]}',NULL,?1)",[id.as_str()])?;
        transaction.execute("INSERT INTO mesh_access_legacy_v5(id,policy,network_id) VALUES(1,'{\"grants\":[]}',?1)",[id.as_str()])?;
        transaction.commit()?;
        self.network(&id)
    }
}
