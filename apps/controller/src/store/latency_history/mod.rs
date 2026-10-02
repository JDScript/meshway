//! Immutable probe events, independent of heartbeat/runtime-report refreshes.
use chrono::{DateTime, Utc};
use meshway_protocol::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{NetworkId, Store, connection_runtime, connections};
use crate::Error;

#[cfg(test)]
mod tests;

pub(crate) fn upgrade(connection: &mut Connection) -> Result<(), Error> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(include_str!("schema.sql"))?;
    transaction.commit()?;
    Ok(())
}

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
    connection.execute("INSERT INTO latency_runtime_epochs(node_id,network_id,epoch,started_ms) VALUES (?1,?2,?3,?4) ON CONFLICT(node_id) DO UPDATE SET epoch=excluded.epoch,started_ms=excluded.started_ms WHERE latency_runtime_epochs.epoch<>excluded.epoch AND latency_runtime_epochs.network_id=excluded.network_id", params![source,network.as_str(),epoch,Utc::now().timestamp_millis()])?;
    Ok(())
}

impl Store {
    /// P2b read-network permission integration point. Scope is resolved only
    /// after authentication; callers never interpret a raw ID as permission.
    pub(crate) fn authorize_latency_history_read(
        &self,
        token: &str,
        network: &str,
    ) -> Result<NetworkId, Error> {
        self.authorize_admin(token)?;
        self.network_id(network)
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
        // Retention makes bounded progress even when admission later rejects a
        // full source budget. Insert idempotency/atomicity remains in the tx.
        cleanup(&self.connection, now)?;
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
        let current =
            connections::projection(&connections::admin_snapshot(&tx, &network)?, &source)?;
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
            validate(&tx, &network, &source, &current.payload, sample, now)?;
            if sample.measured_at.timestamp_millis() < started_ms - CLOCK_SKEW_SECONDS * 1000 {
                return Err(Error::Unprocessable(
                    "latency measurement predates runtime epoch",
                ));
            }
            let recent: u32 = tx.query_row(
                "SELECT COUNT(*) FROM latency_samples WHERE network_id=?1 AND source_node_id=?2 AND received_ms>=?3",
                params![network.as_str(),source,now.timestamp_millis()-60_000], |row| row.get(0),
            )?;
            let rows: u32 = tx.query_row(
                "SELECT COUNT(*) FROM latency_samples WHERE network_id=?1 AND source_node_id=?2",
                params![network.as_str(), source],
                |row| row.get(0),
            )?;
            if recent >= 512 || rows >= NODE_ROW_BUDGET {
                return Err(Error::Capacity("latency sample storage budget reached"));
            }
            tx.execute(
                "INSERT INTO latency_samples(network_id,source_node_id,peer_node_id,measurement_id,runtime_epoch,measured_ms,received_ms,body) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![network.as_str(),source,sample.peer_node_id,sample.measurement_id,epoch,sample.measured_at.timestamp_millis(),now.timestamp_millis(),body],
            )?;
            response.inserted += 1;
        }
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
        cleanup(&self.connection, now)?;
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
            next,
            retention_seconds: LATENCY_RETENTION_SECONDS,
            expected_interval_seconds: LATENCY_SAMPLE_INTERVAL_SECONDS,
        })
    }
}

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
