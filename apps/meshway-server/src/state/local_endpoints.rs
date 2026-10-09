//! State-owned, volatile projection read by the previous publication chain
//! when another input triggers it; a report never triggers a compilation.
//! TEMP storage shares the single SQLite owner and transaction with execution
//! publication; it never survives a Server restart or becomes
//! configuration/identity authority. The netmap chain uses `state::facts`.
use super::{REPORT_TTL_SECONDS, ReportAdmission};
use crate::{error::Error, secrets::Sealer, store::execution};
use chrono::{DateTime, Duration, Utc};
use meshway_configuration::{
    NetworkConfiguration,
    execution::{CompiledPublication, DialEndpoint, DialEndpointSource},
};
use meshway_model::{
    Digest, NetworkId, NodeId,
    state::{ListenerPhase, MAX_LOCAL_ENDPOINTS, ResourceRef},
};
use meshway_protocol::state::StateReport;
use rusqlite::{OptionalExtension, Transaction, params};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
};

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct ListenerEndpoints {
    listener: Digest,
    addresses: BTreeSet<SocketAddr>,
}
type Endpoints = BTreeMap<String, ListenerEndpoints>;

fn initialize(tx: &Transaction<'_>) -> Result<(), Error> {
    tx.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS state_local_endpoints (
        network_id BLOB NOT NULL, node_id BLOB NOT NULL, instance_id BLOB NOT NULL,
        generation INTEGER NOT NULL, expires_at INTEGER NOT NULL, payload BLOB NOT NULL,
        PRIMARY KEY(network_id,node_id)
    ) STRICT;",
    )?;
    Ok(())
}

pub(super) fn replace(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    admission: &ReportAdmission,
    report: &StateReport,
    now: DateTime<Utc>,
) -> Result<bool, Error> {
    initialize(tx)?;
    // Installation-only samples do not withdraw an earlier actual listener
    // observation or renew its expiry. This avoids publication feedback while
    // the reporting node is applying a peer's newly learned candidates.
    if report.state.configuration.applied.is_none()
        || report.state.configuration.applied.as_ref() != admission.execution.as_ref()
    {
        return Ok(false);
    }
    let mut endpoints = Endpoints::new();
    if report.state.configuration.applied.as_ref() == admission.execution.as_ref()
        && let Some((_, target)) =
            execution::target_for_node(tx, sealer, admission.network_id, admission.node_id)?
    {
        for (key, listener) in &report.state.listeners {
            if listener.phase != ListenerPhase::Listening {
                continue;
            }
            let Some(declaration) = target.configuration.listeners.get(key) else {
                continue;
            };
            let addresses: BTreeSet<_> = listener
                .socket_ids
                .iter()
                .filter_map(|id| report.state.sockets.get(id))
                .filter(|socket| socket.resource == ResourceRef::Listener { key: key.clone() })
                .flat_map(|socket| {
                    socket
                        .local_endpoints
                        .iter()
                        .flatten()
                        .map(|endpoint| endpoint.address)
                })
                .take(MAX_LOCAL_ENDPOINTS)
                .collect();
            if !addresses.is_empty() {
                endpoints.insert(
                    key.clone(),
                    ListenerEndpoints {
                        listener: meshway_configuration::digest(declaration)?,
                        addresses,
                    },
                );
            }
        }
    }
    let payload = execution::bytes(&endpoints)?;
    let previous: Option<(Vec<u8>, Vec<u8>, i64, i64)> = tx.query_row(
        "SELECT payload,instance_id,generation,expires_at FROM temp.state_local_endpoints WHERE network_id=?1 AND node_id=?2",
        params![admission.network_id.as_uuid().as_bytes(), admission.node_id.as_uuid().as_bytes()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).optional()?;
    let changed = previous.as_ref().map_or(
        !endpoints.is_empty(),
        |(old, instance, generation, expires)| {
            old != &payload
                || (!endpoints.is_empty()
                    && (instance != admission.instance_id.as_uuid().as_bytes()
                        || *generation != admission.instance_generation.get() as i64
                        || *expires <= now.timestamp_millis()))
        },
    );
    let expires = (report.observed_at + Duration::seconds(REPORT_TTL_SECONDS))
        .min(now + Duration::seconds(REPORT_TTL_SECONDS))
        .min(admission.lease_expires_at);
    tx.execute(
        "INSERT INTO temp.state_local_endpoints VALUES(?1,?2,?3,?4,?5,?6)
        ON CONFLICT(network_id,node_id) DO UPDATE SET instance_id=excluded.instance_id,
        generation=excluded.generation,expires_at=excluded.expires_at,payload=excluded.payload",
        params![
            admission.network_id.as_uuid().as_bytes(),
            admission.node_id.as_uuid().as_bytes(),
            admission.instance_id.as_uuid().as_bytes(),
            admission.instance_generation.get() as i64,
            expires.timestamp_millis(),
            payload
        ],
    )?;
    Ok(changed)
}

pub(crate) fn apply(
    tx: &Transaction<'_>,
    network: NetworkId,
    cfg: &NetworkConfiguration,
    compiled: &mut CompiledPublication,
    now: i64,
) -> Result<(), Error> {
    initialize(tx)?;
    // No stale incarnation, expired report, disabled node or changed listener
    // can contribute an address to a newly published target.
    tx.execute(
        "DELETE FROM temp.state_local_endpoints WHERE expires_at<=?1",
        [now],
    )?;
    let mut statement = tx.prepare(
        "SELECT s.node_id,s.payload FROM temp.state_local_endpoints s
        JOIN node_execution e ON e.network_id=s.network_id AND e.node_id=s.node_id
        AND e.instance_id=s.instance_id AND e.generation=s.generation
        WHERE s.network_id=?1 AND s.expires_at>?2 AND e.instance_expires_at>?2",
    )?;
    let mut rows = statement.query(params![network.as_uuid().as_bytes(), now])?;
    let mut reported = BTreeMap::<NodeId, Endpoints>::new();
    while let Some(row) = rows.next()? {
        reported.insert(
            execution::id(row.get(0)?)?,
            execution::json(&row.get::<_, Vec<u8>>(1)?)?,
        );
    }
    for node in compiled.nodes.values_mut() {
        for candidate in node.configuration.dialers.values_mut() {
            let Some(local) = reported
                .get(&candidate.peer)
                .and_then(|listeners| listeners.get(&candidate.listener_key))
            else {
                continue;
            };
            let Some(listener) = cfg
                .nodes
                .get(&candidate.peer)
                .filter(|node| node.enabled)
                .and_then(|node| node.listeners.get(&candidate.listener_key))
            else {
                continue;
            };
            if meshway_configuration::digest(listener)? != local.listener {
                continue;
            }
            for address in local.addresses.iter().take(MAX_LOCAL_ENDPOINTS) {
                if candidate.endpoints.iter().any(|endpoint| {
                    endpoint.host.parse::<std::net::IpAddr>().ok() == Some(address.ip())
                        && endpoint.port.get() == address.port()
                }) {
                    continue;
                }
                candidate.endpoints.push(DialEndpoint {
                    host: address.ip().to_string(),
                    port: address
                        .port()
                        .try_into()
                        .map_err(|_| Error::InvalidRequest)?,
                    source: DialEndpointSource::Local,
                });
            }
        }
        node.digest = meshway_configuration::digest(&node.configuration)?;
    }
    Ok(())
}
