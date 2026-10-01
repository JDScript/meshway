use meshway_protocol::{
    AgentDnsSnapshot, DNS_SCHEMA_VERSION, MAX_DNS_BYTES, MeshDnsConfiguration, NetworkSnapshot,
    NodeDnsConfiguration, SaveNetworkRequest, compile_dns_records, dns_digest, local,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{NetworkId, Store, access, configuration, connections, ipam};
use crate::Error;

#[cfg(test)]
mod tests;

/// Additive v6 -> v7. Existing payloads and address pools do not change, so
/// retain the global revision. The new DNS body starts at that same revision.
pub(super) fn upgrade(connection: &mut Connection) -> Result<(), Error> {
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

fn snapshot(connection: &Connection, network: &NetworkId) -> Result<NetworkSnapshot, Error> {
    let (network_id, name, json): (String, String, String) = connection.query_row(
        "SELECT network_id,name,dns FROM network_identity WHERE id=1 AND network_id=?1",
        [network.as_str()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let dns: MeshDnsConfiguration = serde_json::from_str(&json)
        .map_err(|_| Error::Configuration("invalid persisted network DNS"))?;
    if dns.normalized().ok().as_ref() != Some(&dns) {
        return Err(Error::Configuration("invalid persisted network DNS"));
    }
    Ok(NetworkSnapshot {
        revision: configuration::revision(connection, network)?,
        network_id,
        name,
        network: ipam::network(connection, network)?,
        dns,
    })
}

fn dns_snapshot(
    connection: &Connection,
    scope: &NetworkId,
    node_id: String,
) -> Result<AgentDnsSnapshot, Error> {
    let network = snapshot(connection, scope)?;
    let (records, diagnostics) = compile_dns_records(
        &network.dns,
        &configuration::active_nodes(connection, scope)?,
    )
    .map_err(Error::Unprocessable)?;
    let payload = NodeDnsConfiguration {
        schema_version: DNS_SCHEMA_VERSION,
        revision: network.revision,
        node_id,
        network_id: network.network_id,
        dns: network.dns,
        records,
        diagnostics,
    };
    payload.validate().map_err(Error::Unprocessable)?;
    let digest =
        dns_digest(&payload).map_err(|_| Error::Configuration("cannot encode DNS snapshot"))?;
    let snapshot = AgentDnsSnapshot { digest, payload };
    if serde_json::to_vec(&snapshot)
        .map_err(|_| Error::Configuration("cannot encode DNS snapshot"))?
        .len()
        > MAX_DNS_BYTES
    {
        return Err(Error::Capacity(
            "DNS snapshot exceeds 1 MiB publication budget",
        ));
    }
    Ok(snapshot)
}

pub(super) fn ensure_publishable(
    connection: &Connection,
    network: &NetworkId,
) -> Result<(), Error> {
    let id: Option<String> = connection
        .query_row(
            "SELECT id FROM nodes WHERE revoked_at IS NULL AND network_id=?1 ORDER BY id LIMIT 1",
            [network.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = id {
        dns_snapshot(connection, network, id)?;
    }
    Ok(())
}

impl Store {
    pub fn network(&mut self, network: &NetworkId) -> Result<NetworkSnapshot, Error> {
        let transaction = self.connection.transaction()?;
        let result = snapshot(&transaction, network)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn save_network(
        &mut self,
        network: &NetworkId,
        request: &SaveNetworkRequest,
    ) -> Result<NetworkSnapshot, Error> {
        request.validate().map_err(Error::Unprocessable)?;
        let dns = request.dns.normalized().map_err(Error::Unprocessable)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = snapshot(&transaction, network)?;
        if current.revision != request.expected_revision {
            return Err(Error::Conflict);
        }
        if current.name != request.name || current.dns != dns {
            let json = serde_json::to_string(&dns)
                .map_err(|_| Error::Configuration("cannot encode network DNS"))?;
            transaction.execute(
                "UPDATE network_identity SET name=?1,dns=?2 WHERE id=1 AND network_id=?3",
                params![request.name, json, network.as_str()],
            )?;
            configuration::bump(&transaction, network)?;
            configuration::ensure_publishable(&transaction, network)?;
            connections::ensure_publishable(&transaction, network)?;
            access::ensure_publishable(&transaction, network)?;
            ensure_publishable(&transaction, network)?;
        }
        let result = snapshot(&transaction, network)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn agent_dns(&mut self, token: &str) -> Result<AgentDnsSnapshot, Error> {
        let network = self.node_network(token)?;
        // One Store lock and one SQLite read transaction cover credential scope,
        // global revision, persisted zone and the complete active inventory.
        let transaction = self.connection.transaction()?;
        let id = configuration::node_id(&transaction, &network, token)?;
        let result = dns_snapshot(&transaction, &network, id)?;
        transaction.commit()?;
        Ok(result)
    }
}
