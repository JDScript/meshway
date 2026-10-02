use meshway_protocol::{
    AgentDnsSnapshot, DNS_SCHEMA_VERSION, MAX_DNS_BYTES, MeshDnsConfiguration, NetworkSnapshot,
    NodeDnsConfiguration, SaveNetworkRequest, compile_dns_records, dns_digest,
};
use rusqlite::{Connection, TransactionBehavior, params};

use super::{NetworkId, Store, configuration, ipam, publication};
use crate::Error;

#[cfg(test)]
mod tests;

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
    dns_projection(
        network,
        &configuration::active_nodes(connection, scope)?,
        node_id,
    )
}

fn dns_projection(
    network: NetworkSnapshot,
    nodes: &[meshway_protocol::MeshNode],
    node_id: String,
) -> Result<AgentDnsSnapshot, Error> {
    let (records, diagnostics) =
        compile_dns_records(&network.dns, nodes).map_err(Error::Unprocessable)?;
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

pub(super) fn ensure_publishable_with_nodes(
    connection: &Connection,
    network: &NetworkId,
    nodes: &[meshway_protocol::MeshNode],
) -> Result<(), Error> {
    if let Some(node) = nodes.first() {
        dns_projection(snapshot(connection, network)?, nodes, node.id.clone())?;
    }
    Ok(())
}

impl Store {
    pub fn network(&mut self, network: &NetworkId) -> Result<NetworkSnapshot, Error> {
        let transaction = super::account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Deferred,
        )?;
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
        let transaction = super::account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Immediate,
        )?;
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
            publication::finish_change(
                &transaction,
                network,
                publication::ChangeKind::NetworkOrEnrollment,
            )?;
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
