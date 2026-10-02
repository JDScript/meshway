//! Explicit network lookup, creation and globally unique credential binding.
use super::{NetworkId, Store};
use crate::Error;
use meshway_protocol::{
    CreateNetworkRequest, MeshDnsConfiguration, NetworkList, NetworkSnapshot, local,
};
use rusqlite::{Connection, TransactionBehavior, params};

#[cfg(test)]
mod tests;

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
        NetworkId::lookup(&self.connection, id)
    }
    #[cfg(test)]
    pub(super) fn initial_network_id(&self) -> Result<NetworkId, Error> {
        NetworkId::initial(&self.connection)
    }
    pub(super) fn node_network(&self, token: &str) -> Result<NetworkId, Error> {
        NetworkId::node_credential(&self.connection, token)
    }
    pub(super) fn enrollment_network(&self, token: &str) -> Result<NetworkId, Error> {
        NetworkId::enrollment_credential(&self.connection, token)
    }
    pub fn networks(&mut self) -> Result<NetworkList, Error> {
        let ids = NetworkId::all(&self.connection)?;
        let networks = ids
            .into_iter()
            .map(|id| self.network(&id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(NetworkList { networks })
    }
    pub fn create_network(
        &mut self,
        request: &CreateNetworkRequest,
    ) -> Result<NetworkSnapshot, Error> {
        request.validate().map_err(Error::Unprocessable)?;
        let id = NetworkId::generate()?;
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
