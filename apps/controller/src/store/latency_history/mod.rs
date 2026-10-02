//! Immutable probe events, independent of heartbeat/runtime-report refreshes.
use chrono::{DateTime, Utc};
use meshway_protocol::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use super::{NetworkId, Store, connection_runtime, connections};
use crate::Error;

#[cfg(test)]
mod tests;

const REPORT_MAX_AGE_SECONDS: i64 = 120;
const CLOCK_SKEW_SECONDS: i64 = 5;
const CLEANUP_BUDGET: u32 = 1024;
const NODE_ROW_BUDGET: u32 = 50_000;

pub(super) fn begin_epoch(
    connection: &Connection,
    network: &NetworkId,
    source: &str,
    epoch: u32,
) -> Result<(), Error> {
    // The coordinator owns production migration. Existing enrollment/runtime
    // operations remain usable before that migration installs telemetry tables.
    let installed: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='latency_runtime_epochs')",[],|r|r.get(0))?;
    if !installed {
        return Ok(());
    }
    let changed = connection.execute("INSERT INTO latency_runtime_epochs(node_id,network_id,epoch,started_ms) VALUES (?1,?2,?3,?4) ON CONFLICT(node_id) DO UPDATE SET epoch=excluded.epoch,started_ms=excluded.started_ms WHERE latency_runtime_epochs.epoch<>excluded.epoch AND latency_runtime_epochs.network_id=excluded.network_id", params![source,network.as_str(),epoch,Utc::now().timestamp_millis()])?;
    if changed == 0 {
        // A same-epoch retry intentionally leaves the floor unchanged; an
        // ignored new epoch must not commit the enclosing runtime lease change.
        let saved: Option<u32> = connection
            .query_row(
                "SELECT epoch FROM latency_runtime_epochs WHERE node_id=?1 AND network_id=?2",
                params![source, network.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        if saved != Some(epoch) {
            return Err(Error::Configuration("latency epoch write was not applied"));
        }
    } else {
        expect_one(changed)?;
    }
    Ok(())
}

impl Store {
    /// Keep the current session/membership check and history read in one root
    /// transaction, like the other network-scoped management reads.
    pub(crate) fn authorized_latency_history(
        &mut self,
        token: &str,
        network: &str,
        query: &QueryLatencyHistoryRequest,
    ) -> Result<LatencyHistoryResponse, Error> {
        self.network_operation(
            token,
            network,
            crate::accounts::NetworkPermission::Read,
            |store, scope| store.latency_history(scope, query),
        )
    }

    pub(crate) fn report_latency_samples(
        &mut self,
        token: &str,
        request: &ReportLatencySamplesRequest,
    ) -> Result<ReportLatencySamplesResponse, Error> {
        self.report_latency_samples_at(token, request, Utc::now())
    }

    fn report_latency_samples_at(
        &mut self,
        token: &str,
        request: &ReportLatencySamplesRequest,
        now: DateTime<Utc>,
    ) -> Result<ReportLatencySamplesResponse, Error> {
        if request.samples.is_empty() || request.samples.len() > LATENCY_BATCH_LIMIT {
            return Err(Error::Unprocessable(
                "latency batch must contain 1..64 events",
            ));
        }
        let network = self.node_network(token)?;
        // Cheap eligibility/shape rejection precedes cleanup/full projection;
        // repeat credential binding under the write transaction for races.
        connection_runtime::bound_node(&self.connection, &network, token)?;
        for sample in &request.samples {
            validate_shape(sample)?;
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let source = connection_runtime::bound_node(&tx, &network, token)?;
        let runtime: Option<(u32, Vec<u8>, String)> = tx.query_row(
            "SELECT l.epoch,r.body,r.reported_at FROM connection_runtime_leases l JOIN connection_runtime_reports r ON r.node_id=l.node_id AND r.network_id=l.network_id WHERE l.node_id=?1 AND l.network_id=?2",
            params![source, network.as_str()],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional()?;
        let (epoch, runtime_body, reported_at) = runtime.ok_or(Error::Conflict)?;
        let started_ms: Option<i64> = tx.query_row("SELECT started_ms FROM latency_runtime_epochs WHERE node_id=?1 AND network_id=?2 AND epoch=?3", params![source,network.as_str(),epoch], |row| row.get(0)).optional()?;
        let started_ms = started_ms.ok_or(Error::Conflict)?;
        let report: ReportConnectionRuntimeRequest = serde_json::from_slice(&runtime_body)
            .map_err(|_| Error::Configuration("invalid stored runtime report"))?;
        let reported_at = DateTime::parse_from_rfc3339(&reported_at)
            .map_err(|_| Error::Configuration("invalid stored runtime timestamp"))?
            .with_timezone(&Utc);
        if report.epoch != epoch
            || report.state != ConnectionRuntimeState::Applied
            || now < reported_at
            || now - reported_at
                >= chrono::Duration::seconds(i64::from(CONNECTION_RUNTIME_STALE_AFTER_SECONDS))
        {
            return Err(Error::Conflict);
        }
        for sample in &request.samples {
            if sample.runtime_epoch != epoch
                || sample.revision != report.revision
                || sample.digest != report.digest
            {
                return Err(Error::Conflict);
            }
        }
        let current =
            connections::projection(&connections::effective_snapshot(&tx, &network)?, &source)?;
        if !current.payload.valid
            || current.digest != report.digest
            || current.payload.revision != report.revision
        {
            return Err(Error::Conflict);
        }
        let mut response = ReportLatencySamplesResponse {
            inserted: 0,
            duplicates: 0,
        };
        for sample in &request.samples {
            if sample.runtime_epoch != epoch
                || sample.revision != report.revision
                || sample.digest != report.digest
            {
                return Err(Error::Conflict);
            }
            let body = canonical_json(sample)
                .map_err(|_| Error::Configuration("cannot encode latency sample"))?;
            let previous: Option<Vec<u8>> = tx.query_row(
                "SELECT body FROM latency_samples WHERE network_id=?1 AND source_node_id=?2 AND measurement_id=?3",
                params![network.as_str(),source,sample.measurement_id], |row| row.get(0),
            ).optional()?;
            if let Some(previous) = previous {
                if previous != body {
                    return Err(Error::Conflict);
                }
                response.duplicates += 1;
                continue;
            }
            let digest = Sha256::digest(&body).to_vec();
            let tombstone: Option<Vec<u8>> = tx.query_row("SELECT body_digest FROM latency_admissions WHERE network_id=?1 AND source_node_id=?2 AND measurement_id=?3 AND expires_ms>=?4", params![network.as_str(),source,sample.measurement_id,now.timestamp_millis()], |r| r.get(0)).optional()?;
            if let Some(previous) = tombstone {
                if previous != digest {
                    return Err(Error::Conflict);
                }
                response.duplicates += 1;
                continue;
            }
            validate(&tx, &network, &source, &current.payload, sample, now)?;
            if sample.measured_at.timestamp_millis() < started_ms - CLOCK_SKEW_SECONDS * 1000 {
                return Err(Error::Unprocessable(
                    "latency measurement predates runtime epoch",
                ));
            }
            let recent: u32 = tx.query_row(
                "SELECT COUNT(*) FROM (SELECT measurement_id FROM latency_admissions WHERE network_id=?1 AND source_node_id=?2 AND received_ms>=?3 UNION SELECT measurement_id FROM latency_samples WHERE network_id=?1 AND source_node_id=?2 AND received_ms>=?3)",
                params![network.as_str(),source,now.timestamp_millis()-60_000], |row| row.get(0),
            )?;
            if recent >= 512 {
                return Err(Error::Capacity("latency new-event insert budget reached"));
            }
            let expiry =
                (sample.measured_at.timestamp_millis() + REPORT_MAX_AGE_SECONDS * 1000 + 1)
                    .max(now.timestamp_millis() + 60_001);
            expect_one(tx.execute("INSERT INTO latency_admissions(network_id,source_node_id,measurement_id,body_digest,received_ms,expires_ms) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(network_id,source_node_id,measurement_id) DO UPDATE SET body_digest=excluded.body_digest,received_ms=excluded.received_ms,expires_ms=excluded.expires_ms",params![network.as_str(),source,sample.measurement_id,digest,now.timestamp_millis(),expiry])?)?;
            expect_one(tx.execute(
                "INSERT INTO latency_samples(network_id,source_node_id,peer_node_id,measurement_id,runtime_epoch,measured_ms,received_ms,body) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![network.as_str(),source,sample.peer_node_id,sample.measurement_id,epoch,sample.measured_at.timestamp_millis(),now.timestamp_millis(),body],
            )?)?;
            response.inserted += 1;
        }
        cleanup_source(&tx, &network, &source, now)?;
        roll_source(&tx, &network, &source)?;
        tx.commit()?;
        Ok(response)
    }

    pub(crate) fn latency_history(
        &mut self,
        network: &NetworkId,
        query: &QueryLatencyHistoryRequest,
    ) -> Result<LatencyHistoryResponse, Error> {
        self.latency_history_at(network, query, Utc::now())
    }

    fn latency_history_at(
        &mut self,
        network: &NetworkId,
        query: &QueryLatencyHistoryRequest,
        now: DateTime<Utc>,
    ) -> Result<LatencyHistoryResponse, Error> {
        if !(1..=1000).contains(&query.limit)
            || query.until <= query.from
            || query.until - query.from > chrono::Duration::hours(24)
            || query.from < now - chrono::Duration::seconds(i64::from(LATENCY_RETENTION_SECONDS))
            || query.until > now + chrono::Duration::seconds(CLOCK_SKEW_SECONDS)
        {
            return Err(Error::Unprocessable(
                "latency query requires a retained window up to 24 hours and 1..1000 points",
            ));
        }
        let after = query
            .after
            .as_ref()
            .map(|s| {
                if s.is_empty() || s.len() > 20 || !s.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(Error::Unprocessable("invalid latency cursor"));
                }
                s.parse::<i64>()
                    .ok()
                    .filter(|id| *id >= 0)
                    .ok_or(Error::Unprocessable("invalid latency cursor"))
            })
            .transpose()?
            .unwrap_or(0);
        for id in [&query.source_node_id, &query.peer_node_id] {
            let exists: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM nodes WHERE id=?1 AND network_id=?2)",
                params![id, network.as_str()],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(Error::NotFound);
            }
        }
        cleanup_source(&self.connection, network, &query.source_node_id, now)?;
        let capacity_ms: Option<i64> = self.connection.query_row("SELECT capacity_before_ms FROM latency_coverage WHERE network_id=?1 AND source_node_id=?2", params![network.as_str(),query.source_node_id],|r|r.get(0)).optional()?;
        let mut statement = self.connection.prepare(
            "SELECT row_id,body,received_ms FROM latency_samples WHERE network_id=?1 AND source_node_id=?2 AND peer_node_id=?3 AND measured_ms>=?4 AND measured_ms<?5 AND row_id>?6 ORDER BY row_id LIMIT ?7",
        )?;
        let rows = statement
            .query_map(
                params![
                    network.as_str(),
                    query.source_node_id,
                    query.peer_node_id,
                    query.from.timestamp_millis(),
                    query.until.timestamp_millis(),
                    after,
                    query.limit + 1
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let more = rows.len() > query.limit as usize;
        let mut records = Vec::new();
        for (id, body, received) in rows.into_iter().take(query.limit as usize) {
            records.push(LatencyHistoryRecord {
                cursor: id.to_string(),
                network_id: network.as_str().into(),
                source_node_id: query.source_node_id.clone(),
                sample: serde_json::from_slice(&body)
                    .map_err(|_| Error::Configuration("invalid stored latency event"))?,
                received_at: DateTime::from_timestamp_millis(received)
                    .ok_or(Error::Configuration("invalid latency receive timestamp"))?,
                historical: true,
            });
        }
        let next = more.then(|| {
            records
                .last()
                .expect("positive query budget")
                .cursor
                .clone()
        });
        Ok(LatencyHistoryResponse {
            records,
            retention_from: now - chrono::Duration::seconds(i64::from(LATENCY_RETENTION_SECONDS)),
            capacity_trimmed_before: capacity_ms
                .map(|ms| {
                    DateTime::from_timestamp_millis(ms)
                        .ok_or(Error::Configuration("invalid capacity watermark"))
                })
                .transpose()?,
            capacity_trimmed: capacity_ms.is_some_and(|ms| query.from.timestamp_millis() < ms),
            next,
            retention_seconds: LATENCY_RETENTION_SECONDS,
            expected_interval_seconds: LATENCY_SAMPLE_INTERVAL_SECONDS,
        })
    }
}

fn cleanup_source(
    connection: &Connection,
    network: &NetworkId,
    source: &str,
    now: DateTime<Utc>,
) -> Result<(), Error> {
    connection.execute("DELETE FROM latency_samples WHERE row_id IN (SELECT row_id FROM latency_samples WHERE network_id=?1 AND source_node_id=?2 AND measured_ms<?3 ORDER BY measured_ms,row_id LIMIT ?4)",params![network.as_str(),source,now.timestamp_millis()-i64::from(LATENCY_RETENTION_SECONDS)*1000,CLEANUP_BUDGET])?;
    connection.execute("DELETE FROM latency_admissions WHERE rowid IN (SELECT rowid FROM latency_admissions WHERE network_id=?1 AND source_node_id=?2 AND expires_ms<?3 ORDER BY expires_ms LIMIT ?4)",params![network.as_str(),source,now.timestamp_millis(),CLEANUP_BUDGET])?;
    Ok(())
}

fn roll_source(connection: &Connection, network: &NetworkId, source: &str) -> Result<(), Error> {
    let rows: u32 = connection.query_row(
        "SELECT COUNT(*) FROM latency_samples WHERE network_id=?1 AND source_node_id=?2",
        params![network.as_str(), source],
        |r| r.get(0),
    )?;
    let excess = rows.saturating_sub(NODE_ROW_BUDGET);
    if excess == 0 {
        return Ok(());
    }
    if excess > LATENCY_BATCH_LIMIT as u32 {
        return Err(Error::Configuration(
            "latency source occupancy exceeds bounded admission",
        ));
    }
    let victims = connection.prepare("SELECT row_id,measurement_id,measured_ms,received_ms,body FROM latency_samples WHERE network_id=?1 AND source_node_id=?2 ORDER BY measured_ms,row_id LIMIT ?3")?.query_map(params![network.as_str(),source,excess],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,Vec<u8>>(4)?)))?.collect::<Result<Vec<_>,_>>()?;
    let watermark = victims
        .iter()
        .map(|v| v.2 + 1)
        .max()
        .ok_or(Error::Configuration("missing latency quota victims"))?;
    for (row, id, measured, received, body) in victims {
        // Also protect recently received fixture/previous-version rows that did
        // not yet have an admission ledger. Never rewrite a prior live identity.
        let digest = Sha256::digest(&body).to_vec();
        let changed = connection.execute("INSERT OR IGNORE INTO latency_admissions(network_id,source_node_id,measurement_id,body_digest,received_ms,expires_ms) VALUES (?1,?2,?3,?4,?5,?6)",params![network.as_str(),source,id,digest,received,(measured+REPORT_MAX_AGE_SECONDS*1000+1).max(received+60_001)])?;
        if changed == 0 {
            // OR IGNORE is legitimate only if the identity/receipt already has
            // protection. It cannot silently lose a victim's retry/rate guard.
            let protected: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM latency_admissions WHERE network_id=?1 AND source_node_id=?2 AND measurement_id=?3 AND body_digest=?4 AND received_ms=?5)",params![network.as_str(),source,id,digest,received],|r|r.get(0))?;
            if !protected {
                return Err(Error::Configuration(
                    "latency victim identity write was not applied",
                ));
            }
        } else {
            expect_one(changed)?;
        }
        expect_one(connection.execute(
            "DELETE FROM latency_samples WHERE row_id=?1 AND network_id=?2 AND source_node_id=?3",
            params![row, network.as_str(), source],
        )?)?;
    }
    expect_one(connection.execute("INSERT INTO latency_coverage(network_id,source_node_id,capacity_before_ms) VALUES (?1,?2,?3) ON CONFLICT(network_id,source_node_id) DO UPDATE SET capacity_before_ms=MAX(latency_coverage.capacity_before_ms,excluded.capacity_before_ms)",params![network.as_str(),source,watermark])?)?;
    Ok(())
}

fn expect_one(changed: usize) -> Result<(), Error> {
    if changed != 1 {
        return Err(Error::Configuration(
            "latency write was not applied exactly once",
        ));
    }
    Ok(())
}

fn validate_shape(sample: &LatencySample) -> Result<(), Error> {
    if !local::valid_credential(&sample.measurement_id)
        || !local::valid_credential(&sample.sampler_generation)
        || !local::valid_credential(&sample.peer_node_id)
        || !local::valid_credential(&sample.digest)
        || sample.runtime_epoch == 0
        || sample.revision == 0
        || sample.channel_associations.len() > 1
        || sample.failure_reasons.len() > 1
        || sample.completed_at < sample.measured_at
        || sample.completed_at - sample.measured_at > chrono::Duration::seconds(60)
    {
        return Err(Error::Unprocessable("invalid latency event shape"));
    }
    let valid_outcome = match sample.outcome {
        LatencyOutcome::Success => {
            sample.channel_associations.len() == 1
                && sample.failure_reasons.is_empty()
                && sample
                    .rtt_us
                    .is_some_and(|rtt| (1..=60_000_000).contains(&rtt))
        }
        LatencyOutcome::Timeout => {
            sample.channel_associations.is_empty()
                && sample.failure_reasons.is_empty()
                && sample.rtt_us.is_none()
        }
        LatencyOutcome::Failed => {
            sample.channel_associations.is_empty()
                && sample.failure_reasons.len() == 1
                && sample.rtt_us.is_none()
        }
    };
    if !valid_outcome {
        return Err(Error::Unprocessable("invalid latency outcome shape"));
    }
    Ok(())
}

#[cfg(test)]
fn cleanup(connection: &Connection, now: DateTime<Utc>) -> Result<usize, Error> {
    Ok(connection.execute("DELETE FROM latency_samples WHERE row_id IN (SELECT row_id FROM latency_samples WHERE measured_ms<?1 ORDER BY measured_ms LIMIT ?2)",
        params![now.timestamp_millis()-i64::from(LATENCY_RETENTION_SECONDS)*1000,CLEANUP_BUDGET])?)
}

fn validate(
    connection: &Connection,
    network: &NetworkId,
    source: &str,
    config: &NodeConnections,
    sample: &LatencySample,
    now: DateTime<Utc>,
) -> Result<(), Error> {
    if !local::valid_credential(&sample.measurement_id)
        || !local::valid_credential(&sample.sampler_generation)
        || sample.measured_at > now + chrono::Duration::seconds(CLOCK_SKEW_SECONDS)
        || sample.measured_at < now - chrono::Duration::seconds(REPORT_MAX_AGE_SECONDS)
        || sample.completed_at < sample.measured_at
        || sample.completed_at > now + chrono::Duration::seconds(CLOCK_SKEW_SECONDS)
        || sample.completed_at - sample.measured_at > chrono::Duration::seconds(60)
    {
        return Err(Error::Unprocessable(
            "invalid latency identity or measurement clock",
        ));
    }
    let active: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM nodes WHERE id=?1 AND network_id=?2 AND revoked_at IS NULL)",
        params![sample.peer_node_id, network.as_str()],
        |row| row.get(0),
    )?;
    if !active
        || sample.peer_node_id == source
        || !config
            .peers
            .iter()
            .any(|p| p.node_id == sample.peer_node_id)
        || !(config
            .outgoing
            .iter()
            .any(|p| p.peer_node_id == sample.peer_node_id)
            || config
                .incoming
                .iter()
                .any(|p| p.peer_node_id == sample.peer_node_id))
    {
        return Err(Error::Unprocessable(
            "latency peer is outside active authorized adjacency",
        ));
    }
    match sample.outcome {
        LatencyOutcome::Success => {
            if !sample.failure_reasons.is_empty() {
                return Err(Error::Unprocessable("success cannot have failure reasons"));
            }
            if sample.channel_associations.len() != 1 {
                return Err(Error::Unprocessable(
                    "successful probe requires exactly one channel association",
                ));
            }
            let channel = &sample.channel_associations[0];
            let rtt = sample
                .rtt_us
                .ok_or(Error::Unprocessable("successful probe requires RTT"))?;
            if rtt == 0
                || rtt > 60_000_000
                || i64::from(rtt)
                    > (sample.completed_at - sample.measured_at)
                        .num_microseconds()
                        .unwrap_or(0)
                        + 1000
            {
                return Err(Error::Unprocessable("invalid probe RTT"));
            }
            let approved = if channel.initiator_node_id == source {
                config.outgoing.iter().any(|p| {
                    p.peer_node_id == sample.peer_node_id
                        && p.transport == channel.transport
                        && p.listener_id == channel.listener_id
                        && channel.candidate_id.as_deref() == Some(p.id.as_str())
                })
            } else if channel.initiator_node_id == sample.peer_node_id {
                config.incoming.iter().any(|p| {
                    p.peer_node_id == sample.peer_node_id
                        && p.transport == channel.transport
                        && p.listener_id == channel.listener_id
                        && channel
                            .candidate_id
                            .as_ref()
                            .is_none_or(|id| id == &p.candidate_id)
                })
            } else {
                false
            };
            if !approved {
                return Err(Error::Unprocessable(
                    "latency channel is outside configured adjacency",
                ));
            }
        }
        LatencyOutcome::Timeout
            if sample.rtt_us.is_some()
                || !sample.channel_associations.is_empty()
                || !sample.failure_reasons.is_empty() =>
        {
            return Err(Error::Unprocessable(
                "timeout cannot claim RTT or unproven channel metadata",
            ));
        }
        LatencyOutcome::Timeout => {}
        LatencyOutcome::Failed => {
            if sample.rtt_us.is_some()
                || !sample.channel_associations.is_empty()
                || sample.failure_reasons.len() != 1
            {
                return Err(Error::Unprocessable(
                    "failed attempt requires one reason and no RTT/channel proof",
                ));
            }
        }
    }
    Ok(())
}
