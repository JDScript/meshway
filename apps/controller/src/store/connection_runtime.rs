use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use meshway_protocol::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{Store, configuration, connections, timestamp};
use crate::Error;

#[cfg(test)]
mod tests;

pub(super) fn upgrade(connection: &mut Connection) -> Result<(), Error> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if (4..=7).contains(&version) {
        return Ok(());
    }
    if version != 3 {
        return Err(Error::Configuration(
            "runtime schema upgrade requires schema 3",
        ));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "CREATE TABLE connection_runtime_leases (
            node_id TEXT PRIMARY KEY REFERENCES nodes(id),
            instance_id TEXT NOT NULL,
            epoch INTEGER NOT NULL CHECK (epoch BETWEEN 1 AND 4294967295)
         );
         CREATE TABLE connection_runtime_reports (
            node_id TEXT PRIMARY KEY REFERENCES connection_runtime_leases(node_id),
            sequence INTEGER NOT NULL CHECK (sequence BETWEEN 1 AND 4294967295),
            body BLOB NOT NULL, reported_at TEXT NOT NULL
         );
         PRAGMA user_version = 4;",
    )?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn bound_node(connection: &Connection, token: &str) -> Result<String, Error> {
    let id = configuration::node_id(connection, token)?;
    let bound: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM connection_identities WHERE node_id = ?1)",
        [&id],
        |row| row.get(0),
    )?;
    if !bound {
        return Err(Error::Unprocessable(
            "connection identity must be bound before runtime reporting",
        ));
    }
    Ok(id)
}

struct StoredReport {
    sequence: u32,
    body: Vec<u8>,
    reported_at: DateTime<Utc>,
}

fn report(connection: &Connection, id: &str) -> Result<Option<StoredReport>, Error> {
    Ok(connection
        .query_row(
            "SELECT sequence, body, reported_at FROM connection_runtime_reports WHERE node_id = ?1",
            [id],
            |row| {
                Ok(StoredReport {
                    sequence: row.get(0)?,
                    body: row.get(1)?,
                    reported_at: timestamp(row, 2)?,
                })
            },
        )
        .optional()?)
}

impl Store {
    pub fn get_connection_runtime_epoch(
        &self,
        token: &str,
    ) -> Result<GetConnectionRuntimeEpochResponse, Error> {
        let id = bound_node(&self.connection, token)?;
        let epoch = self
            .connection
            .query_row(
                "SELECT epoch FROM connection_runtime_leases WHERE node_id = ?1",
                [&id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        Ok(GetConnectionRuntimeEpochResponse { epoch })
    }

    pub fn begin_connection_runtime(
        &mut self,
        token: &str,
        request: &BeginConnectionRuntimeRequest,
    ) -> Result<BeginConnectionRuntimeResponse, Error> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = bound_node(&transaction, token)?;
        if !local::valid_credential(&request.instance_id) {
            return Err(Error::Unprocessable(
                "instance_id must be 64 lowercase hexadecimal characters",
            ));
        }
        let lease: Option<(String, u32)> = transaction
            .query_row(
                "SELECT instance_id, epoch FROM connection_runtime_leases WHERE node_id = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((instance_id, epoch)) = &lease
            && instance_id == &request.instance_id
        {
            return Ok(BeginConnectionRuntimeResponse { epoch: *epoch });
        }
        let current_epoch = lease.map_or(0, |(_, epoch)| epoch);
        if request.expected_epoch != current_epoch {
            return Err(Error::Conflict);
        }
        let epoch = current_epoch
            .checked_add(1)
            .ok_or(Error::Capacity("connection runtime epoch exhausted"))?;
        transaction.execute(
            "INSERT INTO connection_runtime_leases (node_id,instance_id,epoch) VALUES (?1,?2,?3)
             ON CONFLICT(node_id) DO UPDATE SET instance_id=excluded.instance_id,epoch=excluded.epoch",
            params![id, request.instance_id, epoch],
        )?;
        transaction.execute(
            "DELETE FROM connection_runtime_reports WHERE node_id = ?1",
            [&id],
        )?;
        transaction.commit()?;
        Ok(BeginConnectionRuntimeResponse { epoch })
    }

    pub fn report_connection_runtime(
        &mut self,
        token: &str,
        request: &ReportConnectionRuntimeRequest,
    ) -> Result<ReportConnectionRuntimeResponse, Error> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = bound_node(&transaction, token)?;
        if request.epoch == 0
            || request.sequence == 0
            || request.revision == 0
            || !local::valid_credential(&request.digest)
        {
            return Err(Error::Unprocessable(
                "invalid runtime epoch, sequence, revision or digest",
            ));
        }
        let epoch: Option<u32> = transaction
            .query_row(
                "SELECT epoch FROM connection_runtime_leases WHERE node_id = ?1",
                [&id],
                |row| row.get(0),
            )
            .optional()?;
        if epoch != Some(request.epoch) {
            return Err(Error::Conflict);
        }
        let previous = report(&transaction, &id)?;
        let body = canonical_json(request)
            .map_err(|_| Error::Configuration("cannot encode runtime report"))?;
        if previous.as_ref().is_some_and(|previous| {
            request.sequence < previous.sequence
                || (request.sequence == previous.sequence && body != previous.body)
        }) {
            return Err(Error::Conflict);
        }
        let current = connections::projection(&connections::admin_snapshot(&transaction)?, &id)?;
        if request.revision != current.payload.revision || request.digest != current.digest {
            return Err(Error::Unprocessable(
                "runtime revision or digest does not match current node connections",
            ));
        }
        validate(request, &current.payload)?;
        if let Some(previous) = previous
            && request.sequence == previous.sequence
        {
            return Ok(ReportConnectionRuntimeResponse {
                epoch: request.epoch,
                sequence: request.sequence,
                reported_at: previous.reported_at,
            });
        }
        let reported_at = Utc::now();
        transaction.execute(
            "INSERT INTO connection_runtime_reports (node_id,sequence,body,reported_at) VALUES (?1,?2,?3,?4)
             ON CONFLICT(node_id) DO UPDATE SET sequence=excluded.sequence,body=excluded.body,reported_at=excluded.reported_at",
            params![id, request.sequence, body, reported_at.to_rfc3339()],
        )?;
        transaction.commit()?;
        Ok(ReportConnectionRuntimeResponse {
            epoch: request.epoch,
            sequence: request.sequence,
            reported_at,
        })
    }

    pub fn connection_runtimes(&self) -> Result<ConnectionRuntimesSnapshot, Error> {
        self.connection_runtimes_at(Utc::now())
    }

    pub(super) fn connection_runtimes_at(
        &self,
        now: DateTime<Utc>,
    ) -> Result<ConnectionRuntimesSnapshot, Error> {
        let snapshot = connections::admin_snapshot(&self.connection)?;
        let leases = self
            .connection
            .prepare("SELECT node_id, epoch FROM connection_runtime_leases ORDER BY node_id")?
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut nodes = Vec::with_capacity(leases.len());
        for (id, epoch) in leases {
            let node = snapshot
                .nodes
                .iter()
                .find(|node| node.node.id == id)
                .ok_or(Error::Configuration("runtime lease has no node"))?;
            let revoked = node.node.revoked_at.is_some();
            let mut observation = ConnectionRuntimeObservation {
                node_id: id.clone(),
                epoch,
                sequence: 0,
                revision: None,
                digest: None,
                state: if revoked {
                    ConnectionRuntimeState::Withdrawn
                } else {
                    ConnectionRuntimeState::AwaitingReport
                },
                reason: ConnectionRuntimeReason::None,
                listeners: Vec::new(),
                peers: Vec::new(),
                reported_at: None,
                stale_after_seconds: CONNECTION_RUNTIME_STALE_AFTER_SECONDS,
                current: false,
                stale: true,
            };
            // Revocation never exposes retained authenticated claims, even if a
            // malformed external database edit left an obsolete report behind.
            if !revoked && let Some(stored) = report(&self.connection, &id)? {
                let report: ReportConnectionRuntimeRequest =
                    serde_json::from_slice(&stored.body)
                        .map_err(|_| Error::Configuration("invalid stored runtime report"))?;
                let current = connections::projection(&snapshot, &id)?;
                observation.current = report.epoch == epoch
                    && report.revision == current.payload.revision
                    && report.digest == current.digest;
                let age = now.signed_duration_since(stored.reported_at);
                observation.stale = !observation.current
                    || age < chrono::Duration::zero()
                    || age
                        >= chrono::Duration::seconds(i64::from(
                            CONNECTION_RUNTIME_STALE_AFTER_SECONDS,
                        ));
                observation.sequence = stored.sequence;
                observation.revision = Some(report.revision);
                observation.digest = Some(report.digest);
                observation.state = report.state;
                observation.reason = report.reason;
                observation.listeners = report.listeners;
                observation.peers = report.peers;
                observation.reported_at = Some(stored.reported_at);
            }
            nodes.push(observation);
        }
        Ok(ConnectionRuntimesSnapshot { nodes })
    }
}

fn validate(
    request: &ReportConnectionRuntimeRequest,
    configuration: &NodeConnections,
) -> Result<(), Error> {
    use ConnectionRuntimeReason as Reason;
    use ConnectionRuntimeState as State;
    if request.listeners.len() > 128 || request.peers.len() > 256 {
        return Err(Error::Unprocessable(
            "runtime observation resource limit exceeded",
        ));
    }
    match request.state {
        State::AwaitingReport => {
            return Err(Error::Unprocessable("awaiting_report is server-only"));
        }
        State::Disabled | State::Withdrawn | State::Error => {
            if !request.listeners.is_empty() || !request.peers.is_empty() {
                return Err(Error::Unprocessable(
                    "inactive runtime states must not claim resources",
                ));
            }
        }
        State::Applied | State::Applying => {
            if request.reason != Reason::None {
                return Err(Error::Unprocessable(
                    "applied or applying runtime must have reason none",
                ));
            }
        }
    }
    if request.state == State::Error && request.reason == Reason::None {
        return Err(Error::Unprocessable("runtime error requires a reason"));
    }
    if !configuration.valid
        && (!matches!(request.state, State::Error | State::Withdrawn)
            || request.reason != Reason::InvalidConfiguration)
    {
        return Err(Error::Unprocessable(
            "invalid configuration requires error or withdrawal with invalid_configuration reason",
        ));
    }
    let mut listeners = BTreeSet::new();
    for listener in &request.listeners {
        if !listeners.insert(&listener.listener_id)
            || !configuration
                .listeners
                .iter()
                .any(|configured| configured.id == listener.listener_id)
            || (listener.state == ConnectionListenerState::Bound)
                != (listener.reason == Reason::None)
        {
            return Err(Error::Unprocessable(
                "invalid, duplicate or out-of-scope listener observation",
            ));
        }
    }
    if request.state == State::Applied
        && (request.listeners.len() != configuration.listeners.len()
            || request
                .listeners
                .iter()
                .any(|listener| listener.state != ConnectionListenerState::Bound))
    {
        return Err(Error::Unprocessable(
            "applied runtime requires every configured listener bound",
        ));
    }
    let mut peers = BTreeSet::new();
    for peer in &request.peers {
        let Some(listener_id) = peer.listener_id.as_deref() else {
            return Err(Error::Unprocessable(
                "neighbor observation requires a configured listener",
            ));
        };
        if !peers.insert((
            &peer.peer_node_id,
            peer.transport,
            &peer.initiator_node_id,
            listener_id,
            &peer.candidate_id,
        )) {
            return Err(Error::Unprocessable("duplicate neighbor observation"));
        }
        let approved = if peer.initiator_node_id == configuration.node_id {
            configuration.outgoing.iter().any(|candidate| {
                peer.candidate_id.as_deref() == Some(candidate.id.as_str())
                    && candidate.listener_id == listener_id
                    && candidate.peer_node_id == peer.peer_node_id
                    && candidate.transport == peer.transport
            })
        } else if peer.initiator_node_id == peer.peer_node_id {
            configuration.incoming.iter().any(|approval| {
                peer.candidate_id
                    .as_ref()
                    .is_none_or(|id| id == &approval.candidate_id)
                    && approval.listener_id == listener_id
                    && approval.peer_node_id == peer.peer_node_id
                    && approval.transport == peer.transport
            })
        } else {
            false
        };
        if !approved {
            return Err(Error::Unprocessable(
                "neighbor observation is outside configured adjacency or dialer role",
            ));
        }
        let peer_failure = matches!(
            peer.reason,
            Reason::ProfileUnavailable
                | Reason::ResolveFailed
                | Reason::TransportFailed
                | Reason::AuthenticationFailed
                | Reason::ProbeFailed
                | Reason::InternalError
        );
        let valid_reason = match peer.state {
            ConnectionNeighborState::Authenticated => peer.reason == Reason::None,
            ConnectionNeighborState::Waiting => peer.reason == Reason::None || peer_failure,
            ConnectionNeighborState::Unreachable => peer_failure,
        };
        if !valid_reason {
            return Err(Error::Unprocessable(
                "neighbor reason must match its state and describe a peer-local outcome",
            ));
        }
        match peer.state {
            ConnectionNeighborState::Authenticated if !peer.probe_fresh => {
                return Err(Error::Unprocessable(
                    "authenticated neighbor requires a fresh authenticated probe",
                ));
            }
            ConnectionNeighborState::Waiting | ConnectionNeighborState::Unreachable
                if peer.probe_fresh || peer.probe_rtt_ms.is_some() =>
            {
                return Err(Error::Unprocessable(
                    "unverified neighbor must not claim a probe or RTT",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}
