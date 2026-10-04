//! Durable source events and coverage. Tombstones outlive the complete admission
//! window, including clock skew; retries never recreate pruned payloads.
use crate::{
    resources::content_digest,
    state::{ReportAdmission, StateError, VisibilityScope},
};
use chrono::{DateTime, Duration, Utc};
use meshway_model::{
    InstanceId, NodeId,
    observations::{ObservationEvent, ObservationPayload},
    state::ProbeOutcome,
};
use meshway_protocol::observations::*;
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::BTreeMap;

pub const ADMISSION_SECONDS: i64 = 600;
pub const RETENTION_SECONDS: i64 = 7 * 86400;
pub const MAX_EVENTS_PER_NETWORK: i64 = 100_000;
pub const MAX_QUERY_ROWS: usize = 10_000;

pub fn accept(
    tx: &Transaction<'_>,
    admission: &ReportAdmission,
    batch: HistoryReport,
    now: DateTime<Utc>,
) -> Result<HistoryAccepted, StateError> {
    if admission.instance_id != batch.instance_id || admission.lease_expires_at <= now {
        return Err(StateError::Unauthorized);
    }
    if batch.events.len() > 256
        || serde_json::to_vec(&batch)
            .map_err(|_| StateError::Invalid)?
            .len()
            > 1024 * 1024
    {
        return Err(StateError::Budget);
    }
    let mut accepted = 0;
    let mut duplicates = 0;
    for event in batch.events {
        let context = admission.context(&event.configuration)?;
        if event.occurred_at > now + Duration::seconds(30)
            || event.occurred_at + Duration::seconds(ADMISSION_SECONDS) <= now
            || !admission.configurations.contains(&event.configuration)
        {
            return Err(StateError::Invalid);
        }
        match &event.payload {
            ObservationPayload::ProbeCompleted { probe } => {
                if event.event_id != probe.measurement_id
                    || probe.publication_id != event.configuration.execution_publication
                    || probe.completed_at != Some(event.occurred_at)
                {
                    return Err(StateError::Invalid);
                }
                crate::state::validation::validate_probe_context(
                    admission.node_id,
                    context,
                    probe,
                    now,
                )?;
            }
            ObservationPayload::CollectionGap { dropped, from, to } => {
                if dropped.get() == 0 || from >= to || *to > event.occurred_at {
                    return Err(StateError::Invalid);
                }
            }
            payload => {
                if peer(payload).is_none_or(|id| !context.peers.contains(&id)) {
                    return Err(StateError::Invalid);
                }
            }
        }
        let bytes = serde_json::to_vec(&event).map_err(|_| StateError::Invalid)?;
        let digest = content_digest(&bytes).to_string();
        let prior: Option<(String, String)> = tx.query_row("SELECT payload_digest, instance_id FROM observation_events WHERE network_id = ?1 AND source_node_id = ?2 AND event_id = ?3", params![admission.network_id.as_uuid().as_bytes().as_slice(), admission.node_id.to_string(), event.event_id.to_string()], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
        if let Some((original, instance)) = prior {
            if original != digest || instance != batch.instance_id.to_string() {
                return Err(StateError::Conflict);
            }
            duplicates += 1;
            continue;
        }
        tx.execute("INSERT INTO observation_events (network_id, source_node_id, instance_id, event_id, occurred_at, received_at, payload, payload_digest, admission_expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)", params![admission.network_id.as_uuid().as_bytes().as_slice(), admission.node_id.to_string(), batch.instance_id.to_string(), event.event_id.to_string(), event.occurred_at.timestamp_millis(), now.timestamp_millis(), bytes, digest, (event.occurred_at + Duration::seconds(ADMISSION_SECONDS + 30)).timestamp_millis()])?;
        accepted += 1;
    }
    prune(tx, admission.network_id, now)?;
    Ok(HistoryAccepted {
        accepted,
        duplicates,
    })
}

pub fn prune(
    tx: &Transaction<'_>,
    network: meshway_model::NetworkId,
    now: DateTime<Utc>,
) -> Result<(), StateError> {
    let network = network.as_uuid().as_bytes().as_slice();
    let cutoff = (now - Duration::seconds(RETENTION_SECONDS)).timestamp_millis();
    let capacity_row: i64 = tx.query_row("SELECT coalesce((SELECT row_id FROM observation_events WHERE network_id = ?1 AND payload IS NOT NULL ORDER BY row_id DESC LIMIT 1 OFFSET ?2), 0)", params![network, MAX_EVENTS_PER_NETWORK], |row| row.get(0))?;
    for (predicate, reason, bound) in [
        ("occurred_at < ?2", "retention", cutoff),
        ("row_id <= ?2", "capacity", capacity_row),
    ] {
        let sql = format!(
            "INSERT INTO history_coverage(network_id, source_node_id, before_at, reason) SELECT network_id, source_node_id, max(occurred_at) + 1, ?3 FROM observation_events WHERE network_id = ?1 AND payload IS NOT NULL AND {predicate} GROUP BY source_node_id ON CONFLICT(network_id, source_node_id, reason) DO UPDATE SET before_at = max(before_at, excluded.before_at)"
        );
        tx.execute(&sql, params![network, bound, reason])?;
        tx.execute(
            &format!(
                "UPDATE observation_events SET payload = NULL WHERE network_id = ?1 AND {predicate}"
            ),
            params![network, bound],
        )?;
    }
    tx.execute("DELETE FROM observation_events WHERE network_id = ?1 AND payload IS NULL AND admission_expires_at <= ?2", params![network, now.timestamp_millis()])?;
    Ok(())
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadWindow {
    pub ceiling: i64,
    pub offset: usize,
    pub as_of: DateTime<Utc>,
    pub query_digest: meshway_model::Digest,
    pub authorization_digest: meshway_model::Digest,
    pub result_count: Option<usize>,
}
pub fn query_digest(query: &HistoryQuery) -> Result<meshway_model::Digest, StateError> {
    let mut query = query.clone();
    query.cursor = None;
    Ok(content_digest(
        &serde_json::to_vec(&query).map_err(|_| StateError::Invalid)?,
    ))
}
/// The service encrypts `ReadWindow` with caller/network-bound cursor AAD.
/// Rows are read under the same transaction as current authorization.
pub fn query(
    tx: &Transaction<'_>,
    scope: &VisibilityScope,
    query: &HistoryQuery,
    previous: Option<ReadWindow>,
    now: DateTime<Utc>,
) -> Result<(HistoryPage, Option<ReadWindow>), StateError> {
    if scope.expires_at <= now {
        return Err(StateError::Unauthorized);
    }
    if query.from >= query.to
        || query.to - query.from > Duration::days(7)
        || query.to > now + Duration::seconds(30)
        || !(1..=200).contains(&query.limit)
        || query
            .source_node_id
            .is_some_and(|id| !scope.visible.contains(&id))
        || query
            .peer_node_id
            .is_some_and(|id| !scope.visible.contains(&id))
    {
        return Err(StateError::Invalid);
    }
    if query.mode == HistoryMode::Buckets
        && query
            .bucket_seconds
            .is_none_or(|seconds| !(1..=86400).contains(&seconds))
        || query.mode != HistoryMode::Buckets && query.bucket_seconds.is_some()
    {
        return Err(StateError::Invalid);
    }
    if query.kinds.as_ref().is_some_and(|kinds| kinds.len() > 6)
        || query
            .link_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256)
    {
        return Err(StateError::Invalid);
    }
    let query_digest = query_digest(query)?;
    let mut window = match previous {
        Some(window) => {
            if window.query_digest != query_digest
                || window.authorization_digest != scope.authorization_digest
                || window.as_of + Duration::minutes(15) <= now
            {
                return Err(StateError::Cursor);
            }
            window
        }
        None => ReadWindow {
            ceiling: tx.query_row(
                "SELECT coalesce(max(row_id), 0) FROM observation_events",
                [],
                |row| row.get(0),
            )?,
            offset: 0,
            as_of: now,
            query_digest,
            authorization_digest: scope.authorization_digest,
            result_count: None,
        },
    };
    // Gaps may end after the requested display window. Read the bounded
    // retention window, then intersect gap intervals separately from samples.
    let mut statement = tx.prepare("SELECT source_node_id, instance_id, received_at, payload FROM observation_events WHERE network_id = ?1 AND occurred_at >= ?2 AND row_id <= ?4 AND payload IS NOT NULL AND (occurred_at < ?3 OR json_extract(CAST(payload AS TEXT),'$.payload.kind') = 'collection_gap') ORDER BY occurred_at, row_id LIMIT ?5")?;
    let rows = statement.query_map(
        params![
            scope.network_id.as_uuid().as_bytes().as_slice(),
            query.from.timestamp_millis(),
            query.to.timestamp_millis(),
            window.ceiling,
            (MAX_QUERY_ROWS + 1) as i64
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
    let mut events = vec![];
    let mut coverage = vec![];
    for (count, row) in rows.enumerate() {
        if count >= MAX_QUERY_ROWS {
            return Err(StateError::Budget);
        }
        let (node, instance, received_at, bytes) = row?;
        let node: NodeId = node.parse().map_err(|_| StateError::Storage)?;
        if !scope.visible.contains(&node)
            || query.source_node_id.is_some_and(|source| source != node)
        {
            continue;
        }
        let event: ObservationEvent =
            serde_json::from_slice(&bytes).map_err(|_| StateError::Storage)?;
        if let ObservationPayload::CollectionGap { from, to, dropped } = &event.payload {
            // A source-wide drop count has no narrower peer/link attribution.
            // Only readers of the complete source may see it, regardless of
            // display mode/kinds; never expose it as a peer-specific statistic.
            if scope.full.contains(&node) && *from < query.to && *to > query.from {
                coverage.push(HistoryCoverage::CollectionGap {
                    source_node_id: node,
                    instance_id: instance.parse().map_err(|_| StateError::Storage)?,
                    event_id: event.event_id,
                    from: *from,
                    to: *to,
                    dropped: *dropped,
                });
            }
        }
        if event.occurred_at >= query.to {
            continue;
        }
        let (kind, link) = match &event.payload {
            ObservationPayload::DialStarted { .. } => (HistoryKind::DialStarted, None),
            ObservationPayload::DialFailed { .. } => (HistoryKind::DialFailed, None),
            ObservationPayload::LinkOpened { link_id, .. } => {
                (HistoryKind::LinkOpened, Some(link_id))
            }
            ObservationPayload::LinkClosed { link_id, .. } => {
                (HistoryKind::LinkClosed, Some(link_id))
            }
            ObservationPayload::ProbeCompleted { probe } => {
                (HistoryKind::ProbeCompleted, probe.link_id.as_ref())
            }
            ObservationPayload::CollectionGap { .. } => (HistoryKind::CollectionGap, None),
        };
        if query
            .kinds
            .as_ref()
            .is_some_and(|kinds| !kinds.contains(&kind))
            || query.link_id.as_ref().is_some_and(|id| Some(id) != link)
        {
            continue;
        }
        let peer = peer(&event.payload);
        if peer.is_some_and(|id| !scope.visible.contains(&id))
            || query.peer_node_id.is_some_and(|id| Some(id) != peer)
        {
            continue;
        }
        if let ObservationPayload::ProbeCompleted { probe } = &event.payload
            && (probe.path.iter().any(|node| !scope.visible.contains(node))
                || !scope.full.contains(&node) && !scope.observation_paths.contains(&probe.path))
        {
            continue;
        }
        if !scope.full.contains(&node)
            && peer.is_none_or(|peer| {
                !scope.observation_paths.iter().any(|path| {
                    path.windows(2)
                        .any(|pair| pair == [node, peer] || pair == [peer, node])
                })
            })
        {
            continue;
        }
        events.push(HistoricalEvent {
            source_node_id: node,
            instance_id: instance
                .parse::<InstanceId>()
                .map_err(|_| StateError::Storage)?,
            received_at: DateTime::from_timestamp_millis(received_at).ok_or(StateError::Storage)?,
            event,
        });
    }
    let all = match query.mode {
        HistoryMode::Raw => HistoryItems::Raw { items: events },
        HistoryMode::Buckets => HistoryItems::Buckets {
            items: buckets(events, query)?,
        },
        HistoryMode::RecentLinks => HistoryItems::RecentLinks {
            items: recent(events),
        },
    };
    let limit = usize::from(query.limit);
    let (result, total) = match all {
        HistoryItems::Raw { items } => {
            let total = items.len();
            (
                HistoryItems::Raw {
                    items: items.into_iter().skip(window.offset).take(limit).collect(),
                },
                total,
            )
        }
        HistoryItems::Buckets { items } => {
            let total = items.len();
            (
                HistoryItems::Buckets {
                    items: items.into_iter().skip(window.offset).take(limit).collect(),
                },
                total,
            )
        }
        HistoryItems::RecentLinks { items } => {
            let total = items.len();
            (
                HistoryItems::RecentLinks {
                    items: items.into_iter().skip(window.offset).take(limit).collect(),
                },
                total,
            )
        }
    };
    if window.offset > total || window.result_count.is_some_and(|count| count != total) {
        return Err(StateError::Cursor);
    }
    window.result_count = Some(total);
    let mut statement = tx.prepare(
        "SELECT source_node_id, before_at, reason FROM history_coverage WHERE network_id = ?1",
    )?;
    let rows = statement.query_map([scope.network_id.as_uuid().as_bytes().as_slice()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (source, before, reason) = row?;
        let source: NodeId = source.parse().map_err(|_| StateError::Storage)?;
        if !scope.visible.contains(&source) || query.source_node_id.is_some_and(|id| id != source) {
            continue;
        }
        coverage.push(HistoryCoverage::Pruned {
            source_node_id: source,
            before: DateTime::from_timestamp_millis(before).ok_or(StateError::Storage)?,
            reason: match reason.as_str() {
                "capacity" => CoverageReason::Capacity,
                "retention" => CoverageReason::Retention,
                _ => return Err(StateError::Storage),
            },
        });
    }
    let page = HistoryPage {
        from: query.from,
        to: query.to,
        as_of: window.as_of,
        next_cursor: None,
        coverage,
        result,
    };
    window.offset += limit;
    Ok((page, (window.offset < total).then_some(window)))
}
fn peer(payload: &ObservationPayload) -> Option<NodeId> {
    match payload {
        ObservationPayload::DialStarted { peer_node_id, .. }
        | ObservationPayload::DialFailed { peer_node_id, .. }
        | ObservationPayload::LinkOpened { peer_node_id, .. }
        | ObservationPayload::LinkClosed { peer_node_id, .. } => Some(*peer_node_id),
        ObservationPayload::ProbeCompleted { probe } => probe.path.last().copied(),
        ObservationPayload::CollectionGap { .. } => None,
    }
}
fn buckets(
    events: Vec<HistoricalEvent>,
    query: &HistoryQuery,
) -> Result<Vec<HistoryBucket>, StateError> {
    let seconds = i64::from(query.bucket_seconds.ok_or(StateError::Invalid)?);
    let mut buckets: BTreeMap<BucketKey, (HistoryBucket, u128, u32)> = BTreeMap::new();
    for event in events {
        let ObservationPayload::ProbeCompleted { probe } = event.event.payload else {
            continue;
        };
        let index = (probe.started_at - query.from)
            .num_seconds()
            .div_euclid(seconds);
        let start = query.from + Duration::seconds(index * seconds);
        let (bucket, sum, timed) = buckets
            .entry(BucketKey {
                source: event.source_node_id,
                instance: event.instance_id,
                publication: probe.publication_id,
                path: probe.path.clone(),
                kind: probe.kind,
                link: probe.link_id.clone(),
                transport: probe.transport.clone(),
                index,
            })
            .or_insert_with(|| {
                (
                    HistoryBucket {
                        from: start,
                        to: (start + Duration::seconds(seconds)).min(query.to),
                        source_node_id: event.source_node_id,
                        sample_count: 0,
                        success: 0,
                        timeout: 0,
                        failed: 0,
                        mean_success_duration_us: None,
                        instance_id: event.instance_id,
                        publication_id: probe.publication_id,
                        path: probe.path.clone(),
                        kind: probe.kind,
                        link_id: probe.link_id.clone(),
                        transport: probe.transport.clone(),
                    },
                    0,
                    0,
                )
            });
        bucket.sample_count += 1;
        match probe.outcome {
            ProbeOutcome::Success => {
                bucket.success += 1;
                if let Some(duration) = probe.duration_us {
                    *sum += u128::from(duration.get());
                    *timed += 1;
                }
            }
            ProbeOutcome::Timeout => bucket.timeout += 1,
            _ => bucket.failed += 1,
        }
    }
    Ok(buckets
        .into_values()
        .map(|(mut bucket, sum, timed)| {
            bucket.mean_success_duration_us = (timed > 0).then(|| sum as f64 / f64::from(timed));
            bucket
        })
        .collect())
}
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct BucketKey {
    source: NodeId,
    instance: InstanceId,
    publication: meshway_model::PublicationId,
    path: Vec<NodeId>,
    kind: meshway_model::state::ProbeKind,
    link: Option<String>,
    transport: Option<String>,
    index: i64,
}
fn recent(events: Vec<HistoricalEvent>) -> Vec<RecentLink> {
    let mut links = BTreeMap::new();
    for event in events {
        let (peer, link, transport, closed) = match event.event.payload {
            ObservationPayload::LinkOpened {
                peer_node_id,
                link_id,
                transport,
            } => (peer_node_id, link_id, transport, false),
            ObservationPayload::LinkClosed {
                peer_node_id,
                link_id,
                transport,
                ..
            } => (peer_node_id, link_id, transport, true),
            _ => continue,
        };
        links.insert(
            (event.source_node_id, link.clone()),
            RecentLink {
                source_node_id: event.source_node_id,
                peer_node_id: peer,
                link_id: link,
                transport,
                last_event_at: event.event.occurred_at,
                closed,
            },
        );
    }
    links.into_values().collect()
}
