use std::collections::BTreeSet;

use chrono::Utc;
use meshway_protocol::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{NODE_SELECT, NetworkId, Store, configuration, node_row, publication, timestamp};
use crate::Error;

#[cfg(test)]
mod tests;
pub(super) mod validation;

pub(super) fn upgrade(connection: &mut Connection) -> Result<(), Error> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if (3..=8).contains(&version) {
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
    transaction.commit()?;
    Ok(())
}

fn nodes(connection: &Connection, network: &NetworkId) -> Result<Vec<ConnectionNode>, Error> {
    let inventory = connection
        .prepare(&format!("{NODE_SELECT} WHERE network_id=?1 ORDER BY id"))?
        .query_map([network.as_str()], node_row)?
        .collect::<Result<Vec<_>, _>>()?;
    let mut identities = connection
        .prepare("SELECT public_key, capabilities FROM connection_identities WHERE node_id = ?1 AND network_id=?2")?;
    let mut receipts = connection
        .prepare("SELECT revision, digest, saved_at FROM connection_receipts WHERE node_id = ?1 AND network_id=?2")?;
    inventory
        .into_iter()
        .map(|node| {
            let identity: Option<(String, String)> = identities
                .query_row(params![node.id, network.as_str()], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
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
                .query_row(params![node.id, network.as_str()], |row| {
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

pub(super) fn source(
    connection: &Connection,
    network: &NetworkId,
) -> Result<ConnectionPlan, Error> {
    let json: String = connection.query_row(
        "SELECT configuration FROM connection_source WHERE id = 1 AND network_id=?1",
        [network.as_str()],
        |row| row.get(0),
    )?;
    serde_json::from_str(&json).map_err(|_| Error::Configuration("invalid stored connection plan"))
}

pub(super) fn admin_snapshot(
    connection: &Connection,
    network: &NetworkId,
) -> Result<ConnectionsSnapshot, Error> {
    let mut configuration = source(connection, network)?;
    super::managed_transports::append(connection, network, &mut configuration)?;
    let nodes = nodes(connection, network)?;
    let (_, mut diagnostics) = validation::compile(&configuration, &nodes);
    diagnostics.extend(super::managed_transports::stream_diagnostics(
        connection,
        network,
        &configuration,
    )?);
    diagnostics.truncate(64);
    if let Some((policy, _)) = super::policy_v3::source(connection, network)? {
        let assignments = super::tags::assignments(connection, network)?;
        let make = |id: &str| meshway_policy::v3::Node {
            id: id.to_owned(),
            tags: assignments.get(id).cloned().unwrap_or_default(),
            listener: true,
            exit_enabled: false,
        };
        configuration.candidates.retain(|candidate| {
            configuration
                .listeners
                .iter()
                .find(|l| l.id == candidate.listener_id)
                .is_some_and(|l| {
                    meshway_policy::v3::connection_allowed(
                        &policy.connections,
                        &make(&candidate.dialer_node_id),
                        &make(&l.node_id),
                    )
                })
        });
    }
    Ok(ConnectionsSnapshot {
        revision: configuration::revision(connection, network)?,
        configuration,
        valid: diagnostics.is_empty(),
        diagnostics,
        nodes,
    })
}

pub(super) fn projection(
    snapshot: &ConnectionsSnapshot,
    id: &str,
) -> Result<AgentConnectionsSnapshot, Error> {
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
#[cfg(test)]
pub(super) fn ensure_publishable(
    connection: &Connection,
    network: &NetworkId,
) -> Result<(), Error> {
    let snapshot = admin_snapshot(connection, network)?;
    for node in &snapshot.nodes {
        if node.node.revoked_at.is_none() {
            let projected = projection(&snapshot, &node.node.id)?;
            publication::budget::connections(connection, network, &projected)?;
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
        let network = self.node_network(token)?;
        let transaction = super::account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Immediate,
        )?;
        let id = configuration::node_id(&transaction, &network, token)?;
        request.validate().map_err(Error::Invalid)?;
        let mut capabilities = request.capabilities.clone();
        capabilities.sort_by_key(|capability| capability.transport);
        let encoded = serde_json::to_string(&capabilities)
            .map_err(|_| Error::Configuration("cannot encode capabilities"))?;
        let current: Option<(String, String)> = transaction
            .query_row(
                "SELECT public_key, capabilities FROM connection_identities WHERE node_id = ?1 AND network_id=?2",
                params![id,network.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if current
            .as_ref()
            .is_some_and(|(key, _)| key != &request.public_key)
        {
            return Err(Error::Conflict);
        }
        let duplicate = super::networks::public_key_bound(&transaction, &request.public_key, &id)?;
        if duplicate {
            return Err(Error::Conflict);
        }
        if current
            .as_ref()
            .is_none_or(|(_, stored)| stored != &encoded)
        {
            if current.is_some() {
                transaction.execute(
                    "UPDATE connection_identities SET capabilities=?2 WHERE node_id=?1 AND network_id=?3",
                    params![id, encoded,network.as_str()],
                )?;
            } else {
                transaction.execute("INSERT INTO connection_identities (node_id,public_key,capabilities,network_id) VALUES (?1,?2,?3,?4)", params![id,request.public_key,encoded,network.as_str()])?;
            }
            configuration::bump(&transaction, &network)?;
            publication::finish_change(&transaction, &network, publication::ChangeKind::Adjacency)?;
        }
        let response = PublishConnectionIdentityResponse {
            revision: configuration::revision(&transaction, &network)?,
            node_id: id,
            public_key: request.public_key.clone(),
            capabilities,
        };
        transaction.commit()?;
        Ok(response)
    }

    pub fn connections(&self, network: &NetworkId) -> Result<ConnectionsSnapshot, Error> {
        admin_snapshot(&self.connection, network)
    }

    pub fn save_connections(
        &mut self,
        network: &NetworkId,
        request: &SaveConnectionsRequest,
    ) -> Result<ConnectionsSnapshot, Error> {
        let transaction = super::account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Immediate,
        )?;
        if configuration::revision(&transaction, network)? != request.expected_revision {
            return Err(Error::Conflict);
        }
        let managed = super::managed_transports::listeners(&transaction, network)?;
        let mut source = request.configuration.clone();
        let current = admin_snapshot(&transaction, network)?;
        for summary in &managed {
            let id = &summary.settings.id;
            if source.listeners.iter().find(|l| &l.id == id)
                != current.configuration.listeners.iter().find(|l| &l.id == id)
            {
                return Err(Error::Unprocessable(
                    "managed listener edits require managed listener API",
                ));
            }
        }
        source
            .listeners
            .retain(|l| !managed.iter().any(|m| m.settings.id == l.id));
        let mut effective = source.clone();
        super::managed_transports::append(&transaction, network, &mut effective)?;
        let (mut normalized, diagnostics) =
            validation::compile(&effective, &nodes(&transaction, network)?);
        if !diagnostics.is_empty() {
            return Err(Error::Unprocessable(
                "invalid connection plan: check node identities, roles, listeners, endpoints and local profile references",
            ));
        }
        normalized
            .listeners
            .retain(|l| !managed.iter().any(|m| m.settings.id == l.id));
        if self::source(&transaction, network)? != normalized {
            let encoded = serde_json::to_string(&normalized)
                .map_err(|_| Error::Configuration("cannot encode connection plan"))?;
            if encoded.len() > 128 * 1024 {
                return Err(Error::Capacity("connection source exceeds 128 KiB"));
            }
            transaction.execute(
                "UPDATE connection_source SET configuration = ?1 WHERE id = 1 AND network_id=?2",
                params![encoded, network.as_str()],
            )?;
            configuration::bump(&transaction, network)?;
            publication::finish_change(&transaction, network, publication::ChangeKind::Adjacency)?;
        }
        let response = admin_snapshot(&transaction, network)?;
        transaction.commit()?;
        Ok(response)
    }

    pub fn agent_connections(&self, token: &str) -> Result<AgentConnectionsSnapshot, Error> {
        let network = self.node_network(token)?;
        let id = configuration::node_id(&self.connection, &network, token)?;
        projection(&admin_snapshot(&self.connection, &network)?, &id)
    }

    pub fn acknowledge_connections(
        &mut self,
        token: &str,
        request: &AcknowledgeAgentConnectionsRequest,
    ) -> Result<AcknowledgeAgentConnectionsResponse, Error> {
        let network = self.node_network(token)?;
        let transaction = super::account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Immediate,
        )?;
        let id = configuration::node_id(&transaction, &network, token)?;
        if request.revision == 0 || !local::valid_credential(&request.digest) {
            return Err(Error::Invalid(
                "invalid connection receipt revision or digest",
            ));
        }
        let current = projection(&admin_snapshot(&transaction, &network)?, &id)?;
        if request.revision != current.payload.revision || request.digest != current.digest {
            return Err(Error::Conflict);
        }
        let unchanged: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM connection_receipts WHERE node_id=?1 AND revision=?2 AND digest=?3 AND network_id=?4)", params![id, request.revision, request.digest,network.as_str()], |row| row.get(0))?;
        if !unchanged {
            transaction.execute("INSERT INTO connection_receipts (node_id,revision,digest,saved_at,network_id) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(node_id) DO UPDATE SET revision=excluded.revision,digest=excluded.digest,saved_at=excluded.saved_at WHERE connection_receipts.network_id=excluded.network_id", params![id,request.revision,request.digest,Utc::now().to_rfc3339(),network.as_str()])?;
        }
        transaction.commit()?;
        Ok(AcknowledgeAgentConnectionsResponse {
            revision: request.revision,
            digest: request.digest.clone(),
            status: ConnectionReceiptStatus::Saved,
        })
    }
}
