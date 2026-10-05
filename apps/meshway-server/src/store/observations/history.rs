use crate::error::Error;
use chrono::{DateTime, Utc};
use meshway_model::{Digest, EventId, InstanceId, NetworkId, NodeId};
use meshway_protocol::observations::CoverageReason;
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::BTreeSet;

pub(crate) fn previous(
    tx: &Transaction<'_>,
    network: NetworkId,
    source: NodeId,
    event: EventId,
) -> Result<Option<(Digest, InstanceId)>, Error> {
    let row: Option<(String, String)> = tx.query_row(
        "SELECT payload_digest,instance_id FROM observation_events WHERE network_id=?1 AND source_node_id=?2 AND event_id=?3",
        params![network.as_uuid().as_bytes().as_slice(), source.to_string(), event.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    row.map(|(digest, instance)| {
        Ok((
            digest
                .parse()
                .map_err(|_| Error::decode("observation_events.payload_digest"))?,
            instance
                .parse()
                .map_err(|_| Error::decode("observation_events.instance_id"))?,
        ))
    })
    .transpose()
}

pub(crate) struct EventWrite<'a> {
    pub network: NetworkId,
    pub source: NodeId,
    pub instance: InstanceId,
    pub event: EventId,
    pub occurred_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub payload: &'a [u8],
    pub digest: Digest,
    pub admission_expires_at: DateTime<Utc>,
}

pub(crate) fn insert(tx: &Transaction<'_>, event: EventWrite<'_>) -> Result<(), Error> {
    tx.execute(
        "INSERT INTO observation_events(network_id,source_node_id,instance_id,event_id,occurred_at,received_at,payload,payload_digest,admission_expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![event.network.as_uuid().as_bytes().as_slice(), event.source.to_string(), event.instance.to_string(), event.event.to_string(), event.occurred_at.timestamp_millis(), event.received_at.timestamp_millis(), event.payload, event.digest.to_string(), event.admission_expires_at.timestamp_millis()],
    )?;
    Ok(())
}

pub(crate) fn prune(
    tx: &Transaction<'_>,
    network: NetworkId,
    cutoff: DateTime<Utc>,
    max_events: i64,
    now: DateTime<Utc>,
) -> Result<(), Error> {
    let network = network.as_uuid().as_bytes().as_slice();
    let capacity_row: i64 = tx.query_row("SELECT coalesce((SELECT row_id FROM observation_events WHERE network_id=?1 AND payload IS NOT NULL ORDER BY row_id DESC LIMIT 1 OFFSET ?2),0)", params![network, max_events], |row| row.get(0))?;
    for (predicate, reason, bound) in [
        ("occurred_at < ?2", "retention", cutoff.timestamp_millis()),
        ("row_id <= ?2", "capacity", capacity_row),
    ] {
        tx.execute(
            &format!("INSERT INTO history_coverage(network_id,source_node_id,before_at,reason) SELECT network_id,source_node_id,max(occurred_at)+1,?3 FROM observation_events WHERE network_id=?1 AND payload IS NOT NULL AND {predicate} GROUP BY source_node_id ON CONFLICT(network_id,source_node_id,reason) DO UPDATE SET before_at=max(before_at,excluded.before_at)"),
            params![network, bound, reason],
        )?;
        tx.execute(
            &format!(
                "UPDATE observation_events SET payload=NULL WHERE network_id=?1 AND {predicate}"
            ),
            params![network, bound],
        )?;
    }
    tx.execute("DELETE FROM observation_events WHERE network_id=?1 AND payload IS NULL AND admission_expires_at<=?2", params![network, now.timestamp_millis()])?;
    Ok(())
}

pub(crate) fn upper(tx: &Transaction<'_>) -> Result<i64, Error> {
    Ok(tx.query_row(
        "SELECT coalesce(max(row_id),0) FROM observation_events",
        [],
        |row| row.get(0),
    )?)
}

pub(crate) struct Scan<'a> {
    pub network: NetworkId,
    pub sources: &'a BTreeSet<NodeId>,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub ceiling: i64,
    pub limit: usize,
}

pub(crate) struct EventRow {
    pub source: NodeId,
    pub instance: InstanceId,
    pub received_at: DateTime<Utc>,
    pub payload: Vec<u8>,
}

/// Source authorization is applied before LIMIT. Hidden sources cannot consume
/// a reader's scan budget; peer/path checks still belong to the domain projection.
pub(crate) fn events(
    tx: &Transaction<'_>,
    scan: Scan<'_>,
    mut visit: impl FnMut(usize, EventRow) -> Result<(), Error>,
) -> Result<(), Error> {
    let sources = serde_json::to_string(scan.sources)
        .map_err(|_| Error::encode("observation history sources"))?;
    let limit = i64::try_from(scan.limit).map_err(|_| Error::StateLimit)?;
    let mut statement = tx.prepare(
        "SELECT source_node_id,instance_id,received_at,payload FROM observation_events WHERE network_id=?1 AND occurred_at>=?2 AND row_id<=?4 AND payload IS NOT NULL AND source_node_id IN (SELECT value FROM json_each(?6)) AND (occurred_at<?3 OR json_extract(CAST(payload AS TEXT),'$.payload.kind')='collection_gap') ORDER BY occurred_at,row_id LIMIT ?5",
    )?;
    let rows = statement.query_map(
        params![
            scan.network.as_uuid().as_bytes().as_slice(),
            scan.from.timestamp_millis(),
            scan.to.timestamp_millis(),
            scan.ceiling,
            limit,
            sources
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        },
    )?;
    // Keep the previous streaming read: do not buffer all encoded payloads in
    // addition to the domain's selected/decoded results.
    for (index, row) in rows.enumerate() {
        let (source, instance, received, payload) = row?;
        visit(
            index,
            EventRow {
                source: source
                    .parse()
                    .map_err(|_| Error::decode("observation_events.source_node_id"))?,
                instance: instance
                    .parse()
                    .map_err(|_| Error::decode("observation_events.instance_id"))?,
                received_at: DateTime::from_timestamp_millis(received)
                    .ok_or(Error::decode("observation_events.received_at"))?,
                payload,
            },
        )?;
    }
    Ok(())
}

pub(crate) struct CoverageRow {
    pub source: NodeId,
    pub before: DateTime<Utc>,
    pub reason: CoverageReason,
}

pub(crate) fn coverage(
    tx: &Transaction<'_>,
    network: NetworkId,
) -> Result<Vec<CoverageRow>, Error> {
    let mut statement = tx.prepare(
        "SELECT source_node_id,before_at,reason FROM history_coverage WHERE network_id=?1",
    )?;
    statement
        .query_map([network.as_uuid().as_bytes().as_slice()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .map(|row| {
            let (source, before, reason) = row?;
            Ok(CoverageRow {
                source: source
                    .parse()
                    .map_err(|_| Error::decode("history_coverage.source_node_id"))?,
                before: DateTime::from_timestamp_millis(before)
                    .ok_or(Error::decode("history_coverage.before_at"))?,
                reason: match reason.as_str() {
                    "capacity" => CoverageReason::Capacity,
                    "retention" => CoverageReason::Retention,
                    _ => return Err(Error::decode("history_coverage.reason")),
                },
            })
        })
        .collect()
}
