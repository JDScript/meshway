use meshway_protocol::{
    AcknowledgeConfigurationRequest, AcknowledgeConfigurationResponse,
    CONFIGURATION_SCHEMA_VERSION, ConfigurationSnapshot, MeshNode, NodeConfiguration, NodeSummary,
    SetNodeCountryRequest, configuration_digest, local::digest,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{NetworkId, Store, find_node, ipam, publication};
use crate::Error;

pub(super) fn revision(connection: &Connection, network: &NetworkId) -> Result<u32, Error> {
    Ok(connection.query_row(
        "SELECT revision FROM configuration WHERE id = 1 AND network_id=?1",
        [network.as_str()],
        |row| row.get(0),
    )?)
}

pub(super) fn bump(connection: &Connection, network: &NetworkId) -> Result<(), Error> {
    if revision(connection, network)? == u32::MAX {
        return Err(Error::Capacity("configuration revision exhausted"));
    }
    connection.execute(
        "UPDATE configuration SET revision = revision + 1 WHERE id = 1 AND network_id=?1",
        [network.as_str()],
    )?;
    Ok(())
}

fn compare_revision(
    connection: &Connection,
    network: &NetworkId,
    expected: u32,
) -> Result<(), Error> {
    if revision(connection, network)? != expected {
        return Err(Error::Conflict);
    }
    Ok(())
}

pub(super) fn active_nodes(
    connection: &Connection,
    network: &NetworkId,
) -> Result<Vec<MeshNode>, Error> {
    Ok(connection.prepare("SELECT id, name, country, mesh_ipv4, mesh_ipv6 FROM nodes WHERE revoked_at IS NULL AND network_id=?1 ORDER BY id")?
        .query_map([network.as_str()], |row| Ok(MeshNode { id: row.get(0)?, name: row.get(1)?, country: row.get(2)?, mesh_ipv4: row.get(3)?, mesh_ipv6: row.get(4)? }))?
        .collect::<Result<_, _>>()?)
}

pub(super) fn node_id(
    connection: &Connection,
    network: &NetworkId,
    token: &str,
) -> Result<String, Error> {
    let (id, revoked): (String, bool) = connection
        .query_row(
            "SELECT id, revoked_at IS NOT NULL FROM nodes WHERE credential_hash = ?1 AND network_id=?2",
            params![digest(token), network.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or(Error::Unauthorized)?;
    if revoked {
        return Err(Error::Forbidden);
    }
    Ok(id)
}

pub(super) fn snapshot(
    connection: &Connection,
    scope: &NetworkId,
    id: &str,
) -> Result<ConfigurationSnapshot, Error> {
    let mut peers = active_nodes(connection, scope)?;
    let index = peers
        .iter()
        .position(|n| n.id == id)
        .ok_or(Error::Forbidden)?;
    let node = peers.remove(index);
    let payload = NodeConfiguration {
        schema_version: CONFIGURATION_SCHEMA_VERSION,
        revision: revision(connection, scope)?,
        network: ipam::network(connection, scope)?,
        node,
        peers,
    };
    let digest = configuration_digest(&payload)
        .map_err(|_| Error::Configuration("cannot hash inventory"))?;
    let snapshot = ConfigurationSnapshot { digest, payload };
    publication::budget::configuration(snapshot.clone())?;
    Ok(snapshot)
}

impl Store {
    pub fn revision(&self, network: &NetworkId) -> Result<u32, Error> {
        revision(&self.connection, network)
    }

    pub fn set_country(
        &mut self,
        network: &NetworkId,
        request: &SetNodeCountryRequest,
    ) -> Result<NodeSummary, Error> {
        let country = request
            .country
            .as_ref()
            .map(|country| {
                if country.len() != 2 || !country.bytes().all(|byte| byte.is_ascii_alphabetic()) {
                    return Err(Error::Invalid("country must be two ASCII letters or null"));
                }
                Ok(country.to_ascii_uppercase())
            })
            .transpose()?;
        let transaction = super::account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Immediate,
        )?;
        compare_revision(&transaction, network, request.expected_revision)?;
        let node = find_node(&transaction, network, &request.node_id)?;
        if node.country != country {
            transaction.execute(
                "UPDATE nodes SET country = ?1 WHERE id = ?2 AND network_id=?3",
                params![country, request.node_id, network.as_str()],
            )?;
            bump(&transaction, network)?;
            publication::finish_change(
                &transaction,
                network,
                publication::ChangeKind::RoutingOrSelectors,
            )?;
        }
        let node = find_node(&transaction, network, &request.node_id)?;
        transaction.commit()?;
        Ok(node)
    }

    pub fn configuration(&self, token: &str) -> Result<ConfigurationSnapshot, Error> {
        let network = self.node_network(token)?;
        snapshot(
            &self.connection,
            &network,
            &node_id(&self.connection, &network, token)?,
        )
    }

    pub fn acknowledge(
        &mut self,
        token: &str,
        request: &AcknowledgeConfigurationRequest,
    ) -> Result<AcknowledgeConfigurationResponse, Error> {
        let network = self.node_network(token)?;
        let transaction = super::account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Immediate,
        )?;
        let id = node_id(&transaction, &network, token)?;
        let current = snapshot(&transaction, &network, &id)?;
        if current.payload.revision != request.revision || current.digest != request.digest {
            return Err(Error::Conflict);
        }
        transaction.execute(
            "UPDATE nodes SET acknowledged_revision = ?1, acknowledged_digest = ?2 WHERE id = ?3 AND network_id=?4",
            params![request.revision, request.digest, id, network.as_str()],
        )?;
        transaction.commit()?;
        Ok(AcknowledgeConfigurationResponse {
            revision: request.revision,
            digest: request.digest.clone(),
        })
    }
}
