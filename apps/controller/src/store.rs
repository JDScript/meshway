use std::{fs::File, path::Path, time::Duration};

use chrono::{DateTime, Utc};
use meshway_protocol::{
    AuthorizeDeviceRequest, CreateEnrollmentTokenResponse, EnrollAgentRequest,
    ExchangeDeviceRequest, NodeSummary,
    local::{self, digest},
};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};

use crate::Error;

mod access;
mod account_authorization;
mod account_operations;
mod configuration;
mod connection_runtime;
mod connections;
mod endpoint_discovery;
mod ipam;
mod jsonc;
mod latency_history;
mod managed_transports;
pub(crate) mod migrations;
mod network;
mod networks;
mod ownership;
mod policy_v3;
mod publication;
pub(crate) use networks::NetworkId;
mod tags;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

pub(crate) struct Store {
    connection: Connection,
    _lock: File,
    discoveries: std::collections::BTreeMap<String, endpoint_discovery::StoredDiscovery>,
    material_key: zeroize::Zeroizing<Vec<u8>>,
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
        let material_key = migrations::initialize(&mut connection, path, new, admin, options)?;
        // Initialization flags still refer to the original network; never
        // renumber any network on restart.
        let initial = NetworkId::initial(&connection)?;
        ipam::check_options(&connection, &initial, options)?;
        Ok(Self {
            connection,
            _lock: lock,
            discoveries: Default::default(),
            material_key,
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

    pub fn nodes(&self, network: &NetworkId) -> Result<Vec<NodeSummary>, Error> {
        let mut query = self
            .connection
            .prepare(&format!("{NODE_SELECT} WHERE network_id=?1 ORDER BY id"))?;
        Ok(query
            .query_map([network.as_str()], node_row)?
            .collect::<Result<_, _>>()?)
    }

    pub fn count(&self, network: &NetworkId) -> Result<u32, Error> {
        Ok(self.connection.query_row(
            "SELECT COUNT(*) FROM nodes WHERE network_id=?1",
            [network.as_str()],
            |row| row.get(0),
        )?)
    }

    pub fn create_token(
        &self,
        network: &NetworkId,
        ttl: u32,
    ) -> Result<CreateEnrollmentTokenResponse, Error> {
        if !(60..=86400).contains(&ttl) {
            return Err(Error::Invalid(
                "expires_in_seconds must be between 60 and 86400",
            ));
        }
        let token = local::credential()?;
        let token_id = local::credential()?;
        let expires_at = Utc::now() + chrono::Duration::seconds(i64::from(ttl));
        self.connection.execute(
            "INSERT INTO enrollment_tokens (id, token_hash, expires_at, network_id) VALUES (?1, ?2, ?3, ?4)",
            params![token_id, digest(&token), expires_at.timestamp_millis(), network.as_str()],
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
        let network = self.enrollment_network(token)?;
        self.enroll_bound(&network, token, request, None)
    }

    pub fn exchange_device(
        &mut self,
        code: &str,
        request: &ExchangeDeviceRequest,
    ) -> Result<NodeSummary, Error> {
        let network = self.enrollment_network(code)?;
        let enrollment = request.enrollment().map_err(Error::Invalid)?;
        let authorization = AuthorizeDeviceRequest {
            redirect_uri: request.redirect_uri.clone(),
            code_challenge: meshway_protocol::s256(&request.code_verifier),
            name: request.name.clone(),
            agent_version: request.agent_version.clone(),
        };
        let binding = serde_json::to_string(&authorization)
            .map_err(|_| Error::Invalid("invalid device exchange"))?;
        self.enroll_bound(&network, code, &enrollment, Some(&binding))
    }

    fn enroll_bound(
        &mut self,
        network: &NetworkId,
        token: &str,
        request: &EnrollAgentRequest,
        device_binding: Option<&str>,
    ) -> Result<NodeSummary, Error> {
        request.validate().map_err(Error::Invalid)?;
        let binding = digest(
            &serde_json::to_string(request).map_err(|_| Error::Invalid("invalid enrollment"))?,
        );
        let transaction = account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Immediate,
        )?;
        let record = transaction.query_row(
            "SELECT expires_at, used_node_id, binding_hash, device_binding FROM enrollment_tokens WHERE token_hash = ?1 AND network_id=?2",
            params![digest(token),network.as_str()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<Vec<u8>>>(2)?, row.get::<_, Option<String>>(3)?)),
        ).optional()?.ok_or(Error::Unauthorized)?;
        match (record.3.as_deref(), device_binding) {
            (None, None) => {}
            (Some(stored), Some(proposed)) if constant_eq(&digest(stored), &digest(proposed)) => {}
            _ => return Err(Error::Unauthorized),
        }
        let account_binding = ownership::binding(&transaction, network, token)?;
        if let Some(node_id) = record.1 {
            if !record
                .2
                .as_ref()
                .is_some_and(|stored| constant_eq(stored, &binding))
            {
                return Err(Error::Unauthorized);
            }
            let node = find_node(&transaction, network, &node_id)?;
            if node.revoked_at.is_some() {
                return Err(Error::Forbidden);
            }
            if let Some(binding) = &account_binding {
                ownership::retry(&transaction, binding, &node_id)?;
            }
            // Expiry limits the first consumption. A bound retry never creates
            // a node or changes authorization, even after that TTL has elapsed.
            return Ok(node);
        }
        let now = Utc::now();
        if record.0 <= now.timestamp_millis() {
            return Err(Error::Unauthorized);
        }
        let owner = account_binding
            .as_ref()
            .map(|binding| {
                crate::accounts::Accounts::new(&transaction).revalidate_device(
                    binding,
                    network,
                    now.timestamp(),
                )
            })
            .transpose()?;
        let credential_hash = digest(&request.node_credential);
        let duplicate = networks::credential_bound(&transaction, &credential_hash)?;
        if duplicate {
            return Err(Error::Conflict);
        }
        let id = local::credential()?;
        let (ipv4, ipv6) = ipam::allocate(&transaction, network)?;
        transaction.execute(
            "INSERT INTO nodes (id, credential_hash, name, agent_version, registered_at, last_seen, mesh_ipv4, mesh_ipv6, network_id) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8)",
            params![id, credential_hash, request.name, request.agent_version, now.to_rfc3339(), ipv4, ipv6, network.as_str()],
        )?;
        transaction.execute(
            "UPDATE enrollment_tokens SET used_node_id = ?1, binding_hash = ?2 WHERE token_hash = ?3 AND used_node_id IS NULL AND network_id=?4",
            params![id, binding, digest(token), network.as_str()],
        )?;
        managed_transports::enroll_initial(&transaction, network, token, &id, &self.material_key)?;
        if let Some(owner) = owner {
            ownership::assign(&transaction, network, &id, &owner.owner().id)?;
        }
        configuration::bump(&transaction, network)?;
        publication::finish_change(
            &transaction,
            network,
            publication::ChangeKind::NetworkOrEnrollment,
        )?;
        let node = find_node(&transaction, network, &id)?;
        transaction.commit()?;
        Ok(node)
    }

    pub fn heartbeat(&mut self, token: &str, version: &str) -> Result<NodeSummary, Error> {
        let network = self.node_network(token)?;
        let transaction = account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Immediate,
        )?;
        let id: String = transaction
            .query_row(
                "SELECT id FROM nodes WHERE credential_hash = ?1 AND network_id=?2",
                params![digest(token), network.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(Error::Unauthorized)?;
        if find_node(&transaction, &network, &id)?.revoked_at.is_some() {
            return Err(Error::Forbidden);
        }
        transaction.execute(
            "UPDATE nodes SET agent_version = ?1, last_seen = ?2 WHERE id = ?3 AND revoked_at IS NULL AND network_id=?4",
            params![version, Utc::now().to_rfc3339(), id,network.as_str()],
        )?;
        let node = find_node(&transaction, &network, &id)?;
        transaction.commit()?;
        Ok(node)
    }

    pub fn revoke(&mut self, network: &NetworkId, id: &str) -> Result<NodeSummary, Error> {
        let transaction = account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Immediate,
        )?;
        let current = find_node(&transaction, network, id)?;
        transaction.execute(
            "UPDATE nodes SET revoked_at = ?1 WHERE id = ?2 AND revoked_at IS NULL AND network_id=?3",
            params![Utc::now().to_rfc3339(), id, network.as_str()],
        )?;
        if current.revoked_at.is_none() {
            transaction.execute(
                "DELETE FROM connection_runtime_reports WHERE node_id = ?1 AND network_id=?2",
                params![id, network.as_str()],
            )?;
            transaction.execute(
                "DELETE FROM node_tags WHERE node_id = ?1 AND network_id=?2",
                params![id, network.as_str()],
            )?;
            configuration::bump(&transaction, network)?;
            publication::finish_change(
                &transaction,
                network,
                publication::ChangeKind::NodeWithdrawal,
            )?;
        }
        let node = find_node(&transaction, network, id)?;
        transaction.commit()?;
        Ok(node)
    }
}

const NODE_SELECT: &str = "SELECT id, name, agent_version, registered_at, last_seen, revoked_at, mesh_ipv4, mesh_ipv6, country, (SELECT revision FROM configuration WHERE id = 1 AND configuration.network_id=nodes.network_id), acknowledged_revision FROM nodes";

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0_u8, |diff, (a, b)| diff | (a ^ b)) == 0
}

fn find_node(connection: &Connection, network: &NetworkId, id: &str) -> Result<NodeSummary, Error> {
    connection
        .query_row(
            &format!("{NODE_SELECT} WHERE id = ?1 AND network_id=?2"),
            params![id, network.as_str()],
            node_row,
        )
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
