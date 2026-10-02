//! Explicit network lookup, creation and globally unique credential binding.
use super::Store;
use crate::Error;
use meshway_protocol::{
    CreateNetworkRequest, MeshDnsConfiguration, NetworkList, NetworkSnapshot, local,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

#[cfg(test)]
mod tests;

/// Only Store's validated resource/credential lookup can construct an ID.
/// This proves existence/format only, never Owner or management permission.
/// Management routes authenticate the administrator before resolving this ID;
/// a future membership-authorized scope must be checked separately in P2b.
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
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
    pub(in crate::store) fn stored(value: String) -> Result<Self, Error> {
        if value.len() != 32
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Error::Configuration("invalid persisted network identity"));
        }
        Ok(Self(value))
    }
    pub(crate) fn lookup(connection: &Connection, id: &str) -> Result<Self, Error> {
        Self::stored(
            connection
                .query_row(
                    "SELECT network_id FROM networks WHERE network_id=?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or(Error::NotFound)?,
        )
    }
    pub(super) fn initial(connection: &Connection) -> Result<Self, Error> {
        Self::stored(connection.query_row(
            "SELECT network_id FROM networks WHERE initial=1",
            [],
            |row| row.get(0),
        )?)
    }
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
        // Intentional global lookup: credential uniqueness locates one immutable
        // node/token network. It grants no administrator or Owner permission.
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
