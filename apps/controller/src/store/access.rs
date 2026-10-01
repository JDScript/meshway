use std::collections::{BTreeMap, BTreeSet};

use meshway_protocol::*;
use rusqlite::{Connection, TransactionBehavior};

use super::{Store, configuration, connections};
use crate::Error;

#[cfg(test)]
mod tests;

pub(super) fn upgrade(connection: &mut Connection) -> Result<(), Error> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == 5 {
        return Ok(());
    }
    if version != 4 {
        return Err(Error::Configuration(
            "access schema upgrade requires schema 4",
        ));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "CREATE TABLE mesh_access_source (
            id INTEGER PRIMARY KEY CHECK (id = 1), policy TEXT NOT NULL
         );
         INSERT INTO mesh_access_source VALUES (1, '{\"grants\":[]}');
         PRAGMA user_version = 5;",
    )?;
    transaction.commit()?;
    Ok(())
}

fn source(connection: &Connection) -> Result<MeshAccessPolicy, Error> {
    let json: String = connection.query_row(
        "SELECT policy FROM mesh_access_source WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    serde_json::from_str(&json).map_err(|_| Error::Configuration("invalid stored mesh access"))
}

fn bindings(connection: &Connection) -> Result<BTreeMap<String, String>, Error> {
    Ok(connection.prepare(
        "SELECT node_id, public_key FROM connection_identities JOIN nodes ON nodes.id = node_id WHERE nodes.revoked_at IS NULL ORDER BY node_id",
    )?.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_, _>>()?)
}

fn validate(
    policy: &MeshAccessPolicy,
    nodes: &[MeshNode],
    keys: &BTreeMap<String, String>,
) -> Vec<PolicyDiagnostic> {
    let mut diagnostics = meshway_policy::validate_mesh_access(policy, nodes);
    if !diagnostics.is_empty() {
        return diagnostics;
    }
    // Structural validation bounds every ID before binding diagnostics copy it.
    // A missing or noncanonical binding withdraws the entire effective policy.
    for grant in &policy.grants {
        for id in &grant.path {
            if diagnostics.len() == meshway_policy::MAX_DIAGNOSTICS {
                return diagnostics;
            }
            if keys
                .get(id)
                .is_none_or(|key| !valid_connection_public_key(key))
            {
                diagnostics.push(PolicyDiagnostic {
                    rule_id: Some(grant.id.clone()),
                    field: "path".into(),
                    message: "Every participant requires an active immutable canonical public-key binding".into(),
                });
            }
        }
    }
    diagnostics
}

fn admin_snapshot(connection: &Connection) -> Result<MeshAccessSnapshot, Error> {
    let policy = source(connection)?;
    let diagnostics = validate(
        &policy,
        &configuration::active_nodes(connection)?,
        &bindings(connection)?,
    );
    Ok(MeshAccessSnapshot {
        revision: configuration::revision(connection)?,
        policy,
        valid: diagnostics.is_empty(),
        diagnostics,
    })
}

fn local_access(access: &MeshAccessSnapshot, id: &str) -> MeshAccessPolicy {
    MeshAccessPolicy {
        grants: access
            .policy
            .grants
            .iter()
            .filter(|grant| access.valid && grant.path.iter().any(|node| node == id))
            .cloned()
            .collect(),
    }
}

fn identities(
    id: &str,
    connections: &NodeConnections,
    access: &MeshAccessPolicy,
    keys: &BTreeMap<String, String>,
) -> Result<Vec<ConnectionPeer>, Error> {
    let ids: BTreeSet<_> = connections
        .peers
        .iter()
        .map(|peer| peer.node_id.as_str())
        .chain(
            access
                .grants
                .iter()
                .flat_map(|grant| grant.path.iter().map(String::as_str)),
        )
        .filter(|peer| *peer != id)
        .collect();
    if ids.len() > MAX_DATAPLANE_IDENTITIES {
        return Err(Error::Capacity(
            "dataplane exceeds 64 union peer identities",
        ));
    }
    ids.into_iter()
        .map(|id| {
            let key = keys
                .get(id)
                .filter(|key| valid_connection_public_key(key))
                .ok_or(Error::Configuration(
                    "dataplane peer has no canonical immutable binding",
                ))?;
            Ok(ConnectionPeer {
                node_id: id.to_owned(),
                public_key: key.clone(),
            })
        })
        .collect()
}

impl Store {
    pub fn mesh_access(&self) -> Result<MeshAccessSnapshot, Error> {
        admin_snapshot(&self.connection)
    }

    pub fn save_mesh_access(
        &mut self,
        request: &SaveMeshAccessRequest,
    ) -> Result<MeshAccessSnapshot, Error> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if configuration::revision(&transaction)? != request.expected_revision {
            return Err(Error::Conflict);
        }
        let keys = bindings(&transaction)?;
        if !validate(
            &request.policy,
            &configuration::active_nodes(&transaction)?,
            &keys,
        )
        .is_empty()
        {
            return Err(Error::Unprocessable(
                "invalid mesh access: check grant IDs, unique full active paths and bound public keys",
            ));
        }
        let proposed = MeshAccessSnapshot {
            revision: request.expected_revision,
            policy: request.policy.clone(),
            valid: true,
            diagnostics: Vec::new(),
        };
        let connection_source = connections::admin_snapshot(&transaction)?;
        let participants: BTreeSet<_> = request
            .policy
            .grants
            .iter()
            .flat_map(|grant| &grant.path)
            .collect();
        for id in participants {
            let adjacency = connections::projection(&connection_source, id)?.payload;
            identities(id, &adjacency, &local_access(&proposed, id), &keys)?;
        }
        if source(&transaction)? != request.policy {
            let encoded = serde_json::to_string(&request.policy)
                .map_err(|_| Error::Configuration("cannot encode mesh access"))?;
            transaction.execute(
                "UPDATE mesh_access_source SET policy = ?1 WHERE id = 1",
                [encoded],
            )?;
            configuration::bump(&transaction)?;
            configuration::ensure_publishable(&transaction)?;
            connections::ensure_publishable(&transaction)?;
        }
        let response = admin_snapshot(&transaction)?;
        transaction.commit()?;
        Ok(response)
    }

    /// AppState holds one Store lock for this entire operation; the read
    /// transaction also pins a SQLite snapshot across every constituent query.
    /// A later connection-plan expansion can exceed the identity union bound;
    /// in that case GET fails closed (422), without truncating a path or peers.
    pub fn agent_dataplane(&mut self, token: &str) -> Result<AgentDataplaneSnapshot, Error> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let node_id = configuration::node_id(&transaction, token)?;
        let configuration = configuration::snapshot(&transaction, &node_id)?.payload;
        let connections =
            connections::projection(&connections::admin_snapshot(&transaction)?, &node_id)?.payload;
        let access = admin_snapshot(&transaction)?;
        let access_policy = local_access(&access, &node_id);
        let identities = identities(
            &node_id,
            &connections,
            &access_policy,
            &bindings(&transaction)?,
        )?;
        let revision = configuration.revision;
        if connections.revision != revision || access.revision != revision {
            return Err(Error::Configuration("incoherent dataplane revision"));
        }
        let payload = NodeDataplane {
            schema_version: DATAPLANE_SCHEMA_VERSION,
            revision,
            node_id,
            configuration,
            connections,
            access_policy,
            access_valid: access.valid,
            identities,
        };
        let digest = dataplane_digest(&payload)
            .map_err(|_| Error::Configuration("cannot encode dataplane"))?;
        let snapshot = AgentDataplaneSnapshot { digest, payload };
        if serde_json::to_vec(&snapshot)
            .map_err(|_| Error::Configuration("cannot encode dataplane"))?
            .len()
            > MAX_DATAPLANE_BYTES
        {
            return Err(Error::Capacity(
                "dataplane exceeds 2 MiB publication budget",
            ));
        }
        transaction.commit()?;
        Ok(snapshot)
    }
}
