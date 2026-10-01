use std::{fs::File, path::Path, time::Duration};

use chrono::{DateTime, Utc};
use meshway_protocol::{
    CreateEnrollmentTokenResponse, EnrollAgentRequest, NodeSummary,
    local::{self, digest},
};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};

use crate::Error;

mod access;
mod configuration;
mod connection_runtime;
mod connections;
mod ipam;
mod jsonc;
mod tags;
#[cfg(test)]
mod tests;

pub(crate) struct Store {
    connection: Connection,
    _lock: File,
}

impl Store {
    pub fn open(path: &Path, options: &crate::NetworkOptions) -> Result<Self, Error> {
        local::private_dir(path)?;
        let lock = local::lock(&path.join("controller.lock"))?;
        let database = path.join("controller.sqlite3");
        let admin_file = path.join("admin.token");
        // symlink_metadata also notices broken symlinks. Partial initialization
        // fails closed rather than creating a different bootstrap identity.
        let exists = |p: &Path| -> Result<bool, Error> {
            match std::fs::symlink_metadata(p) {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error.into()),
            }
        };
        let new = match (exists(&database)?, exists(&admin_file)?) {
            (false, false) => {
                for entry in std::fs::read_dir(path)? {
                    if entry?.file_name() != "controller.lock" {
                        return Err(Error::Configuration(
                            "nonempty data directory has no valid controller identity",
                        ));
                    }
                }
                true
            }
            (true, true) => false,
            _ => {
                return Err(Error::Configuration(
                    "incomplete controller state: both controller.sqlite3 and admin.token are required; refusing to reinitialize",
                ));
            }
        };
        if new {
            let admin = local::credential()?;
            local::atomic_write(&admin_file, format!("{admin}\n").as_bytes(), false)?;
            local::atomic_write(&database, &[], false)?;
        }
        local::open_private(&database, false)?;
        let admin = local::read_private(&admin_file)?;
        let admin = admin.trim_end_matches('\n');
        if !local::valid_credential(admin) {
            return Err(Error::Configuration("invalid admin.token"));
        }
        // A prior startup may have published a bootstrap file and then failed
        // its directory sync. Startup errors never imply those files vanished.
        // Reconfirm visible files before opening a serving controller identity.
        local::ensure_durable(&admin_file)?;
        local::ensure_durable(&database)?;
        let mut connection = Connection::open_with_flags(
            database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        if new {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(
                "CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK (id = 1), admin_hash BLOB NOT NULL);
                 CREATE TABLE nodes (
                    id TEXT PRIMARY KEY,
                    credential_hash BLOB NOT NULL UNIQUE,
                    name TEXT NOT NULL,
                    agent_version TEXT NOT NULL,
                    registered_at TEXT NOT NULL,
                    last_seen TEXT NOT NULL,
                    revoked_at TEXT
                 );
                 CREATE TABLE enrollment_tokens (
                    id TEXT PRIMARY KEY,
                    token_hash BLOB NOT NULL UNIQUE,
                    expires_at INTEGER NOT NULL,
                    used_node_id TEXT REFERENCES nodes(id),
                    binding_hash BLOB,
                    CHECK ((used_node_id IS NULL) = (binding_hash IS NULL))
                 );
                 PRAGMA user_version = 1;",
            )?;
            transaction.execute("INSERT INTO settings VALUES (1, ?1)", [digest(admin)])?;
            transaction.commit()?;
            File::open(path)?.sync_all()?;
        }
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if !(1..=6).contains(&version) {
            return Err(Error::Configuration(
                "unsupported or incomplete controller database schema",
            ));
        }
        let stored: Vec<u8> =
            connection.query_row("SELECT admin_hash FROM settings WHERE id = 1", [], |row| {
                row.get(0)
            })?;
        if stored != digest(admin) {
            return Err(Error::Configuration(
                "admin.token does not match the database; refusing to reinitialize",
            ));
        }
        ipam::upgrade(&mut connection, options)?;
        connections::upgrade(&mut connection)?;
        connection_runtime::upgrade(&mut connection)?;
        access::upgrade(&mut connection)?;
        Ok(Self {
            connection,
            _lock: lock,
        })
    }

    pub fn authorize_admin(&self, token: &str) -> Result<(), Error> {
        let matches: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM settings WHERE id = 1 AND admin_hash = ?1)",
            [digest(token)],
            |row| row.get(0),
        )?;
        if matches {
            Ok(())
        } else {
            Err(Error::Unauthorized)
        }
    }

    pub fn nodes(&self) -> Result<Vec<NodeSummary>, Error> {
        let mut query = self
            .connection
            .prepare(&format!("{NODE_SELECT} ORDER BY id"))?;
        Ok(query.query_map([], node_row)?.collect::<Result<_, _>>()?)
    }

    pub fn count(&self) -> Result<u32, Error> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))?)
    }

    pub fn create_token(&self, ttl: u32) -> Result<CreateEnrollmentTokenResponse, Error> {
        if !(60..=86400).contains(&ttl) {
            return Err(Error::Invalid(
                "expires_in_seconds must be between 60 and 86400",
            ));
        }
        let token = local::credential()?;
        let token_id = local::credential()?;
        let expires_at = Utc::now() + chrono::Duration::seconds(i64::from(ttl));
        self.connection.execute(
            "INSERT INTO enrollment_tokens (id, token_hash, expires_at) VALUES (?1, ?2, ?3)",
            params![token_id, digest(&token), expires_at.timestamp_millis()],
        )?;
        Ok(CreateEnrollmentTokenResponse {
            token_id,
            token,
            expires_at,
        })
    }

    pub fn enroll(
        &mut self,
        token: &str,
        request: &EnrollAgentRequest,
    ) -> Result<NodeSummary, Error> {
        request.validate().map_err(Error::Invalid)?;
        let binding = digest(
            &serde_json::to_string(request).map_err(|_| Error::Invalid("invalid enrollment"))?,
        );
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = transaction.query_row(
            "SELECT expires_at, used_node_id, binding_hash FROM enrollment_tokens WHERE token_hash = ?1",
            [digest(token)],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<Vec<u8>>>(2)?)),
        ).optional()?.ok_or(Error::Unauthorized)?;
        if let Some(node_id) = record.1 {
            if record.2.as_ref() != Some(&binding) {
                return Err(Error::Unauthorized);
            }
            let node = find_node(&transaction, &node_id)?;
            if node.revoked_at.is_some() {
                return Err(Error::Forbidden);
            }
            // Expiry limits the first consumption. A bound retry never creates
            // a node or changes authorization, even after that TTL has elapsed.
            return Ok(node);
        }
        let now = Utc::now();
        if record.0 <= now.timestamp_millis() {
            return Err(Error::Unauthorized);
        }
        let credential_hash = digest(&request.node_credential);
        let duplicate: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM nodes WHERE credential_hash = ?1 UNION ALL SELECT 1 FROM settings WHERE admin_hash = ?1 UNION ALL SELECT 1 FROM enrollment_tokens WHERE token_hash = ?1)",
            [&credential_hash], |row| row.get(0),
        )?;
        if duplicate {
            return Err(Error::Conflict);
        }
        let id = local::credential()?;
        let (ipv4, ipv6) = ipam::allocate(&transaction)?;
        transaction.execute(
            "INSERT INTO nodes (id, credential_hash, name, agent_version, registered_at, last_seen, mesh_ipv4, mesh_ipv6) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7)",
            params![id, credential_hash, request.name, request.agent_version, now.to_rfc3339(), ipv4, ipv6],
        )?;
        transaction.execute(
            "UPDATE enrollment_tokens SET used_node_id = ?1, binding_hash = ?2 WHERE token_hash = ?3 AND used_node_id IS NULL",
            params![id, binding, digest(token)],
        )?;
        configuration::bump(&transaction)?;
        configuration::ensure_publishable(&transaction)?;
        connections::ensure_publishable(&transaction)?;
        access::ensure_publishable(&transaction)?;
        let node = find_node(&transaction, &id)?;
        transaction.commit()?;
        Ok(node)
    }

    pub fn heartbeat(&mut self, token: &str, version: &str) -> Result<NodeSummary, Error> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id: String = transaction
            .query_row(
                "SELECT id FROM nodes WHERE credential_hash = ?1",
                [digest(token)],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(Error::Unauthorized)?;
        if find_node(&transaction, &id)?.revoked_at.is_some() {
            return Err(Error::Forbidden);
        }
        transaction.execute(
            "UPDATE nodes SET agent_version = ?1, last_seen = ?2 WHERE id = ?3 AND revoked_at IS NULL",
            params![version, Utc::now().to_rfc3339(), id],
        )?;
        let node = find_node(&transaction, &id)?;
        transaction.commit()?;
        Ok(node)
    }

    pub fn revoke(&mut self, id: &str) -> Result<NodeSummary, Error> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE nodes SET revoked_at = ?1 WHERE id = ?2 AND revoked_at IS NULL",
            params![Utc::now().to_rfc3339(), id],
        )?;
        if changed != 0 {
            transaction.execute(
                "DELETE FROM connection_runtime_reports WHERE node_id = ?1",
                [id],
            )?;
            transaction.execute("DELETE FROM node_tags WHERE node_id = ?1", [id])?;
            configuration::bump(&transaction)?;
            configuration::ensure_publishable(&transaction)?;
            connections::ensure_publishable(&transaction)?;
            access::ensure_publishable(&transaction)?;
        }
        let node = find_node(&transaction, id)?;
        transaction.commit()?;
        Ok(node)
    }
}

const NODE_SELECT: &str = "SELECT id, name, agent_version, registered_at, last_seen, revoked_at, mesh_ipv4, mesh_ipv6, country, (SELECT revision FROM configuration WHERE id = 1), acknowledged_revision FROM nodes";

fn find_node(connection: &Connection, id: &str) -> Result<NodeSummary, Error> {
    connection
        .query_row(&format!("{NODE_SELECT} WHERE id = ?1"), [id], node_row)
        .optional()?
        .ok_or(Error::NotFound)
}

fn timestamp(row: &Row<'_>, index: usize) -> rusqlite::Result<DateTime<Utc>> {
    let value: String = row.get(index)?;
    value.parse().map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn node_row(row: &Row<'_>) -> rusqlite::Result<NodeSummary> {
    Ok(NodeSummary {
        id: row.get(0)?,
        name: row.get(1)?,
        mesh_ipv4: row.get(6)?,
        mesh_ipv6: row.get(7)?,
        country: row.get(8)?,
        configuration_revision: row.get(9)?,
        acknowledged_revision: row.get(10)?,
        agent_version: row.get(2)?,
        registered_at: timestamp(row, 3)?,
        last_seen: timestamp(row, 4)?,
        revoked_at: if row.get::<_, Option<String>>(5)?.is_some() {
            Some(timestamp(row, 5)?)
        } else {
            None
        },
    })
}
