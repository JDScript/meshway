//! Durable source events and coverage. Tombstones outlive the complete admission
//! window, including clock skew; retries never recreate pruned payloads.
use crate::error::Error;
use crate::store::observations::history as store;
use crate::{
    resources::content_digest,
    state::{ReportAdmission, VisibilityScope},
};
use chrono::{DateTime, Duration, Utc};
use meshway_model::{
    InstanceId, NodeId,
    observations::{ObservationEvent, ObservationPayload},
    state::ProbeOutcome,
};
use meshway_protocol::observations::*;
use rusqlite::Transaction;
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
) -> Result<HistoryAccepted, Error> {
    accept_with_paths(tx, admission, batch, now, None)
}

/// A task result already admitted by the sole ObservationService owner.
/// Its exact approved path is retained through the same transaction; raw
/// Node history uploads cannot manufacture this task-specific path evidence.
pub(super) fn accept_task_result(
    tx: &Transaction<'_>,
    admission: &ReportAdmission,
    batch: HistoryReport,
    now: DateTime<Utc>,
    paths: &[Vec<meshway_model::NodeId>],
) -> Result<HistoryAccepted, Error> {
    accept_with_paths(tx, admission, batch, now, Some(paths))
}

fn accept_with_paths(
    tx: &Transaction<'_>,
    admission: &ReportAdmission,
    batch: HistoryReport,
    now: DateTime<Utc>,
    paths: Option<&[Vec<meshway_model::NodeId>]>,
) -> Result<HistoryAccepted, Error> {
    if admission.instance_id != batch.instance_id || admission.lease_expires_at <= now {
        return Err(Error::InvalidSession);
    }
    if batch.events.len() > 256
        || serde_json::to_vec(&batch)
            .map_err(|_| Error::encode("observation history report"))?
            .len()
            > 1024 * 1024
    {
        return Err(Error::StateLimit);
    }
    let mut accepted = 0;
    let mut duplicates = 0;
    for event in batch.events {
        let context = admission.context(&event.netmap)?;
        if event.occurred_at > now + Duration::seconds(30)
            || event.occurred_at + Duration::seconds(ADMISSION_SECONDS) <= now
            || !admission.configurations.contains(&event.netmap)
        {
            return Err(Error::InvalidRequest);
        }
        match &event.payload {
            ObservationPayload::ProbeCompleted { probe } => {
                if event.event_id != probe.measurement_id
                    || probe.netmap != event.netmap
                    || probe.completed_at != Some(event.occurred_at)
                {
                    return Err(Error::InvalidRequest);
                }
                crate::state::validation::validate_probe_context(
                    admission.node_id,
                    context,
                    probe,
                    now,
                    paths,
                )?;
            }
            ObservationPayload::CollectionGap { dropped, from, to } => {
                if dropped.get() == 0 || from >= to || *to > event.occurred_at {
                    return Err(Error::InvalidRequest);
                }
            }
            payload => {
                if peer(payload).is_none_or(|id| !context.peers.contains(&id)) {
                    return Err(Error::InvalidRequest);
                }
            }
        }
        let bytes = serde_json::to_vec(&event).map_err(|_| Error::encode("observation event"))?;
        let digest = content_digest(&bytes);
        let prior = store::previous(tx, admission.network_id, admission.node_id, event.event_id)?;
        if let Some((original, instance)) = prior {
            if original != digest || instance != batch.instance_id {
                return Err(Error::StateConflict);
            }
            duplicates += 1;
            continue;
        }
        store::insert(
            tx,
            store::EventWrite {
                network: admission.network_id,
                source: admission.node_id,
                instance: batch.instance_id,
                event: event.event_id,
                occurred_at: event.occurred_at,
                received_at: now,
                payload: &bytes,
                digest,
                admission_expires_at: event.occurred_at + Duration::seconds(ADMISSION_SECONDS + 30),
            },
        )?;
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
) -> Result<(), Error> {
    store::prune(
        tx,
        network,
        now - Duration::seconds(RETENTION_SECONDS),
        MAX_EVENTS_PER_NETWORK,
        now,
    )
}

/// Body of the sealed history cursor (`crate::cursor`); the envelope's
/// issue time is the listing's `as_of`.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadWindow {
    pub ceiling: i64,
    pub offset: usize,
    pub query_digest: meshway_model::Digest,
    pub authorization_digest: meshway_model::Digest,
    pub result_count: Option<usize>,
}
pub fn query_digest(query: &HistoryQuery) -> Result<meshway_model::Digest, Error> {
    let mut query = query.clone();
    query.cursor = None;
    Ok(content_digest(
        &serde_json::to_vec(&query).map_err(|_| Error::encode("observation history query"))?,
    ))
}
/// The service seals `ReadWindow` with the listing's start (milliseconds)
/// as a caller/network-bound cursor. Rows are read under the same
/// transaction as current authorization.
pub fn query(
    tx: &Transaction<'_>,
    scope: &VisibilityScope,
    query: &HistoryQuery,
    previous: Option<(ReadWindow, i64)>,
    now: DateTime<Utc>,
) -> Result<(HistoryPage, Option<(ReadWindow, i64)>), Error> {
    if scope.expires_at <= now {
        return Err(Error::InvalidSession);
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
        return Err(Error::InvalidRequest);
    }
    if query.mode == HistoryMode::Buckets
        && query
            .bucket_seconds
            .is_none_or(|seconds| !(1..=86400).contains(&seconds))
        || query.mode != HistoryMode::Buckets && query.bucket_seconds.is_some()
    {
        return Err(Error::InvalidRequest);
    }
    if query.kinds.as_ref().is_some_and(|kinds| kinds.len() > 6)
        || query
            .link_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256)
    {
        return Err(Error::InvalidRequest);
    }
    let query_digest = query_digest(query)?;
    let (mut window, started) = match previous {
        Some((window, started)) => {
            if window.query_digest != query_digest
                || window.authorization_digest != scope.authorization_digest
            {
                return Err(Error::InvalidCursor);
            }
            (window, started)
        }
        None => (
            ReadWindow {
                ceiling: store::upper(tx)?,
                offset: 0,
                query_digest,
                authorization_digest: scope.authorization_digest,
                result_count: None,
            },
            now.timestamp_millis(),
        ),
    };
    let as_of = DateTime::from_timestamp_millis(started).ok_or(Error::InvalidCursor)?;
    // Gaps may end after the requested display window. Read the bounded
    // retention window, then intersect gap intervals separately from samples.
    // Current source visibility and any narrower source selection precede the
    // SQL scan budget. Unrelated hidden traffic cannot exhaust this reader.
    let sources = scope
        .visible
        .iter()
        .copied()
        .filter(|node| {
            query
                .source_node_id
                .is_none_or(|selected| selected == *node)
        })
        .collect();
    let mut events = vec![];
    let mut coverage = vec![];
    store::events(
        tx,
        store::Scan {
            network: scope.network_id,
            sources: &sources,
            from: query.from,
            to: query.to,
            ceiling: window.ceiling,
            limit: MAX_QUERY_ROWS + 1,
        },
        |count, row| {
            if count >= MAX_QUERY_ROWS {
                return Err(Error::StateLimit);
            }
            let store::EventRow {
                source: node,
                instance,
                received_at,
                payload: bytes,
            } = row;
            if !scope.visible.contains(&node)
                || query.source_node_id.is_some_and(|source| source != node)
            {
                return Ok(());
            }
            let event: ObservationEvent = serde_json::from_slice(&bytes)
                .map_err(|_| Error::decode("observation_events.payload"))?;
            if let ObservationPayload::CollectionGap { from, to, dropped } = &event.payload {
                // A source-wide drop count has no narrower peer/link attribution.
                // Only readers of the complete source may see it, regardless of
                // display mode/kinds; never expose it as a peer-specific statistic.
                if scope.full.contains(&node) && *from < query.to && *to > query.from {
                    coverage.push(HistoryCoverage::CollectionGap {
                        source_node_id: node,
                        instance_id: instance,
                        event_id: event.event_id,
                        from: *from,
                        to: *to,
                        dropped: *dropped,
                    });
                }
            }
            if event.occurred_at >= query.to {
                return Ok(());
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
                return Ok(());
            }
            let peer = peer(&event.payload);
            if peer.is_some_and(|id| !scope.visible.contains(&id))
                || query.peer_node_id.is_some_and(|id| Some(id) != peer)
            {
                return Ok(());
            }
            // Observation history of a source is disclosed to readers that see
            // that source fully, over Nodes they can see.
            if !scope.full.contains(&node)
                || matches!(&event.payload, ObservationPayload::ProbeCompleted { probe }
                    if probe.path.iter().any(|node| !scope.visible.contains(node)))
            {
                return Ok(());
            }
            events.push(HistoricalEvent {
                source_node_id: node,
                instance_id: instance,
                received_at,
                event,
            });
            Ok(())
        },
    )?;
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
        return Err(Error::InvalidCursor);
    }
    window.result_count = Some(total);
    for store::CoverageRow {
        source,
        before,
        reason,
    } in store::coverage(tx, scope.network_id)?
    {
        if !scope.visible.contains(&source) || query.source_node_id.is_some_and(|id| id != source) {
            continue;
        }
        coverage.push(HistoryCoverage::Pruned {
            source_node_id: source,
            before,
            reason,
        });
    }
    let page = HistoryPage {
        from: query.from,
        to: query.to,
        as_of,
        next_cursor: None,
        coverage,
        result,
    };
    window.offset += limit;
    Ok((page, (window.offset < total).then_some((window, started))))
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
) -> Result<Vec<HistoryBucket>, Error> {
    let seconds = i64::from(query.bucket_seconds.ok_or(Error::InvalidRequest)?);
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
                netmap: probe.netmap,
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
                        netmap: probe.netmap,
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
    netmap: meshway_model::NetmapRef,
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
