use std::collections::BTreeSet;

use chrono::Utc;
use meshway_protocol::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{NODE_SELECT, Store, configuration, node_row, timestamp};
use crate::Error;

#[cfg(test)]
mod tests;
mod validation;

pub(super) fn upgrade(connection: &mut Connection) -> Result<(), Error> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == 3 {
        return Ok(());
    }
    if version != 2 {
        return Err(Error::Configuration(
            "connection schema upgrade requires schema 2",
        ));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "CREATE TABLE connection_identities (
            node_id TEXT PRIMARY KEY REFERENCES nodes(id), public_key TEXT NOT NULL UNIQUE,
            capabilities TEXT NOT NULL
         );
         CREATE TABLE connection_source (
            id INTEGER PRIMARY KEY CHECK (id = 1), configuration TEXT NOT NULL
         );
         INSERT INTO connection_source VALUES (1, '{\"listeners\":[],\"candidates\":[]}');
         CREATE TABLE connection_receipts (
            node_id TEXT PRIMARY KEY REFERENCES nodes(id),
            revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 4294967295),
            digest TEXT NOT NULL, saved_at TEXT NOT NULL
         );
         PRAGMA user_version = 3;",
    )?;
    ensure_publishable(&transaction)?;
    transaction.commit()?;
    Ok(())
}

fn nodes(connection: &Connection) -> Result<Vec<ConnectionNode>, Error> {
    let inventory = connection
        .prepare(&format!("{NODE_SELECT} ORDER BY id"))?
        .query_map([], node_row)?
        .collect::<Result<Vec<_>, _>>()?;
    let mut identities = connection
        .prepare("SELECT public_key, capabilities FROM connection_identities WHERE node_id = ?1")?;
    let mut receipts = connection
        .prepare("SELECT revision, digest, saved_at FROM connection_receipts WHERE node_id = ?1")?;
    inventory
        .into_iter()
        .map(|node| {
            let identity: Option<(String, String)> = identities
                .query_row([&node.id], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?;
            let (public_key, capabilities) = match identity {
                Some((key, capabilities)) => (
                    Some(key),
                    serde_json::from_str(&capabilities).map_err(|_| {
                        Error::Configuration("invalid stored connection capabilities")
                    })?,
                ),
                None => (None, Vec::new()),
            };
            let receipt = receipts
                .query_row([&node.id], |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, String>(1)?,
                        timestamp(row, 2)?,
                    ))
                })
                .optional()?;
            let (acknowledged_revision, acknowledged_digest, acknowledged_at) = match receipt {
                Some((revision, digest, time)) => (revision, Some(digest), Some(time)),
                None => (0, None, None),
            };
            Ok(ConnectionNode {
                node,
                public_key,
                capabilities,
                acknowledged_revision,
                acknowledged_digest,
                acknowledged_at,
            })
        })
        .collect()
}

fn source(connection: &Connection) -> Result<ConnectionPlan, Error> {
    let json: String = connection.query_row(
        "SELECT configuration FROM connection_source WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    serde_json::from_str(&json).map_err(|_| Error::Configuration("invalid stored connection plan"))
}

fn admin_snapshot(connection: &Connection) -> Result<ConnectionsSnapshot, Error> {
    let configuration = source(connection)?;
    let nodes = nodes(connection)?;
    let (_, diagnostics) = validation::compile(&configuration, &nodes);
    Ok(ConnectionsSnapshot {
        revision: configuration::revision(connection)?,
        configuration,
        valid: diagnostics.is_empty(),
        diagnostics,
        nodes,
    })
}

fn projection(snapshot: &ConnectionsSnapshot, id: &str) -> Result<AgentConnectionsSnapshot, Error> {
    let own = snapshot
        .nodes
        .iter()
        .find(|node| node.node.id == id && node.node.revoked_at.is_none())
        .ok_or(Error::Forbidden)?;
    let mut payload = NodeConnections {
        schema_version: CONNECTIONS_SCHEMA_VERSION,
        revision: snapshot.revision,
        node_id: id.into(),
        public_key: own.public_key.clone(),
        capabilities: own.capabilities.clone(),
        valid: snapshot.valid,
        diagnostics: Vec::new(),
        listeners: Vec::new(),
        outgoing: Vec::new(),
        incoming: Vec::new(),
        peers: Vec::new(),
    };
    if snapshot.valid {
        let mut peers = BTreeSet::new();
        payload.listeners = snapshot
            .configuration
            .listeners
            .iter()
            .filter(|listener| listener.node_id == id)
            .cloned()
            .collect();
        for candidate in &snapshot.configuration.candidates {
            let listener = snapshot
                .configuration
                .listeners
                .iter()
                .find(|listener| listener.id == candidate.listener_id)
                .ok_or(Error::Configuration("valid plan has no listener"))?;
            if candidate.dialer_node_id == id {
                peers.insert(listener.node_id.clone());
                payload.outgoing.push(OutgoingConnectionCandidate {
                    id: candidate.id.clone(),
                    listener_id: listener.id.clone(),
                    peer_node_id: listener.node_id.clone(),
                    transport: listener.transport,
                    endpoint: candidate.endpoint.clone(),
                    priority: candidate.priority,
                    credential_ref: candidate.credential_ref.clone(),
                });
            }
            if listener.node_id == id {
                peers.insert(candidate.dialer_node_id.clone());
                payload.incoming.push(IncomingConnectionApproval {
                    candidate_id: candidate.id.clone(),
                    listener_id: listener.id.clone(),
                    peer_node_id: candidate.dialer_node_id.clone(),
                    transport: listener.transport,
                });
            }
        }
        payload
            .outgoing
            .sort_by(|a, b| (a.priority, &a.id).cmp(&(b.priority, &b.id)));
        payload
            .incoming
            .sort_by(|a, b| a.candidate_id.cmp(&b.candidate_id));
        for peer in peers {
            let node = snapshot
                .nodes
                .iter()
                .find(|node| node.node.id == peer)
                .ok_or(Error::Configuration("valid plan has no peer"))?;
            payload.peers.push(ConnectionPeer {
                node_id: peer,
                public_key: node
                    .public_key
                    .clone()
                    .ok_or(Error::Configuration("valid peer has no key"))?,
            });
        }
    } else {
        payload.diagnostics.push(ConnectionDiagnostic { item_id: None, field: "configuration".into(), message: "Connection source is invalid; all effective adjacency intent withdrawn. Administrator repair required.".into() });
    }
    let digest = connection_digest(&payload)
        .map_err(|_| Error::Configuration("cannot encode connection payload"))?;
    Ok(AgentConnectionsSnapshot { digest, payload })
}

/// Bound every node-scoped projection, not just a representative inventory.
/// Reserve enough for future fail-closed diagnostics and global revision growth.
pub(super) fn ensure_publishable(connection: &Connection) -> Result<(), Error> {
    let snapshot = admin_snapshot(connection)?;
    for node in &snapshot.nodes {
        if node.node.revoked_at.is_none() {
            let encoded = serde_json::to_vec(&projection(&snapshot, &node.node.id)?)
                .map_err(|_| Error::Configuration("cannot encode connections"))?;
            if encoded.len() > MAX_CONNECTIONS_BYTES - 4096 {
                return Err(Error::Capacity(
                    "connections exceed 1 MiB publication budget",
                ));
            }
        }
    }
    Ok(())
}

impl Store {
    pub fn publish_connection_identity(
        &mut self,
        token: &str,
        request: &PublishConnectionIdentityRequest,
    ) -> Result<PublishConnectionIdentityResponse, Error> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = configuration::node_id(&transaction, token)?;
        request.validate().map_err(Error::Invalid)?;
        let mut capabilities = request.capabilities.clone();
        capabilities.sort_by_key(|capability| capability.transport);
        let encoded = serde_json::to_string(&capabilities)
            .map_err(|_| Error::Configuration("cannot encode capabilities"))?;
        let current: Option<(String, String)> = transaction
            .query_row(
                "SELECT public_key, capabilities FROM connection_identities WHERE node_id = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if current
            .as_ref()
            .is_some_and(|(key, _)| key != &request.public_key)
        {
            return Err(Error::Conflict);
        }
        let duplicate: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM connection_identities WHERE public_key = ?1 AND node_id != ?2)", params![request.public_key, id], |row| row.get(0))?;
        if duplicate {
            return Err(Error::Conflict);
        }
        if current
            .as_ref()
            .is_none_or(|(_, stored)| stored != &encoded)
        {
            transaction.execute("INSERT INTO connection_identities (node_id,public_key,capabilities) VALUES (?1,?2,?3) ON CONFLICT(node_id) DO UPDATE SET capabilities = excluded.capabilities", params![id,request.public_key,encoded])?;
            configuration::bump(&transaction)?;
            configuration::ensure_publishable(&transaction)?;
            ensure_publishable(&transaction)?;
        }
        let response = PublishConnectionIdentityResponse {
            revision: configuration::revision(&transaction)?,
            node_id: id,
            public_key: request.public_key.clone(),
            capabilities,
        };
        transaction.commit()?;
        Ok(response)
    }

    pub fn connections(&self) -> Result<ConnectionsSnapshot, Error> {
        admin_snapshot(&self.connection)
    }

    pub fn save_connections(
        &mut self,
        request: &SaveConnectionsRequest,
    ) -> Result<ConnectionsSnapshot, Error> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if configuration::revision(&transaction)? != request.expected_revision {
            return Err(Error::Conflict);
        }
        let (normalized, diagnostics) =
            validation::compile(&request.configuration, &nodes(&transaction)?);
        if !diagnostics.is_empty() {
            return Err(Error::Unprocessable(
                "invalid connection plan: check node identities, roles, listeners, endpoints and local profile references",
            ));
        }
        if source(&transaction)? != normalized {
            let encoded = serde_json::to_string(&normalized)
                .map_err(|_| Error::Configuration("cannot encode connection plan"))?;
            if encoded.len() > 128 * 1024 {
                return Err(Error::Capacity("connection source exceeds 128 KiB"));
            }
            transaction.execute(
                "UPDATE connection_source SET configuration = ?1 WHERE id = 1",
                [encoded],
            )?;
            configuration::bump(&transaction)?;
            configuration::ensure_publishable(&transaction)?;
            ensure_publishable(&transaction)?;
        }
        let response = admin_snapshot(&transaction)?;
        transaction.commit()?;
        Ok(response)
    }

    pub fn agent_connections(&self, token: &str) -> Result<AgentConnectionsSnapshot, Error> {
        let id = configuration::node_id(&self.connection, token)?;
        projection(&admin_snapshot(&self.connection)?, &id)
    }

    pub fn acknowledge_connections(
        &mut self,
        token: &str,
        request: &AcknowledgeAgentConnectionsRequest,
    ) -> Result<AcknowledgeAgentConnectionsResponse, Error> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = configuration::node_id(&transaction, token)?;
        if request.revision == 0 || !local::valid_credential(&request.digest) {
            return Err(Error::Invalid(
                "invalid connection receipt revision or digest",
            ));
        }
        let current = projection(&admin_snapshot(&transaction)?, &id)?;
        if request.revision != current.payload.revision || request.digest != current.digest {
            return Err(Error::Conflict);
        }
        transaction.execute("INSERT INTO connection_receipts (node_id,revision,digest,saved_at) VALUES (?1,?2,?3,?4) ON CONFLICT(node_id) DO UPDATE SET revision=excluded.revision,digest=excluded.digest,saved_at=excluded.saved_at WHERE connection_receipts.revision != excluded.revision OR connection_receipts.digest != excluded.digest", params![id,request.revision,request.digest,Utc::now().to_rfc3339()])?;
        transaction.commit()?;
        Ok(AcknowledgeAgentConnectionsResponse {
            revision: request.revision,
            digest: request.digest.clone(),
            status: ConnectionReceiptStatus::Saved,
        })
    }
}
