//! One revision's immutable application denominator. Later demands/refreshes
//! never add targets or turn another publication's receipts into this result.
use crate::error::Error;
use chrono::{DateTime, Utc};
use meshway_model::*;
use meshway_protocol::{execution::ReceiptFreshness, live::*};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn read(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
    visible: &BTreeSet<NodeId>,
    manager: bool,
    boot: InstanceId,
    now: DateTime<Utc>,
) -> Result<ConfigurationApplyProgress, Error> {
    let encoded: String = tx.query_row(
        "SELECT execution_targets FROM configuration_versions WHERE network_id=?1 AND revision=?2",
        params![network.as_uuid().as_bytes().as_slice(), revision.get() as i64], |row| row.get(0),
    ).optional()?.ok_or(Error::NotFound)?;
    let targets: BTreeMap<NodeId, ConfigurationRef> = serde_json::from_str(&encoded)
        .map_err(|_| Error::decode("configuration_versions.execution_targets"))?;
    let mut nodes = BTreeMap::new();
    let mut summary = ConfigurationProgressSummary {
        target_count: 0,
        pending: 0,
        saved: 0,
        applied: 0,
        failed: 0,
    };
    for (node, target) in targets {
        if !manager && !visible.contains(&node) {
            continue;
        }
        if target.revision != revision {
            return Err(Error::decode("configuration_versions.execution_targets"));
        }
        let receipt = tx.query_row(
            "SELECT phase,instance_id,attempt,sequence,received_at FROM execution_receipts WHERE network_id=?1 AND node_id=?2 AND publication_id=?3 AND digest=?4 ORDER BY generation DESC,attempt DESC,sequence DESC LIMIT 1",
            params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice(), target.execution_publication.as_uuid().as_bytes().as_slice(), target.digest.as_bytes().as_slice()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?)),
        ).optional()?;
        let (phase, instance_id, attempt, sequence, received_at) = match receipt {
            None => (ApplicationPhase::Pending, None, None, None, None),
            Some((phase, instance, attempt, sequence, at)) => (
                match phase.as_str() {
                    "saved" => ApplicationPhase::Saved,
                    "applied" => ApplicationPhase::Applied,
                    "failed" => ApplicationPhase::Failed,
                    _ => return Err(Error::decode("execution_receipts.phase")),
                },
                Some(crate::store::execution::id(instance)?),
                Some(
                    Version::new(number(attempt, "execution_receipts.attempt")?)
                        .map_err(|_| Error::decode("execution_receipts.attempt"))?,
                ),
                Some(
                    Sequence::new(number(sequence, "execution_receipts.sequence")?)
                        .map_err(|_| Error::decode("execution_receipts.sequence"))?,
                ),
                Some(
                    DateTime::from_timestamp_millis(at)
                        .ok_or(Error::decode("execution_receipts.received_at"))?,
                ),
            ),
        };
        let freshness = freshness(tx, network, node, &target, instance_id, boot, now)?;
        summary.target_count += 1;
        match phase {
            ApplicationPhase::Pending => summary.pending += 1,
            ApplicationPhase::Saved => summary.saved += 1,
            ApplicationPhase::Applied => summary.applied += 1,
            ApplicationPhase::Failed => summary.failed += 1,
        }
        nodes.insert(
            node,
            ConfigurationNodeProgress {
                target,
                phase,
                instance_id,
                attempt,
                sequence,
                received_at,
                freshness,
            },
        );
    }
    Ok(ConfigurationApplyProgress {
        network_id: network,
        revision,
        scope: if manager {
            ProgressScope::Full
        } else {
            ProgressScope::Partial
        },
        summary,
        nodes,
    })
}

pub(crate) fn freshness(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
    target: &ConfigurationRef,
    instance_id: Option<InstanceId>,
    boot: InstanceId,
    now: DateTime<Utc>,
) -> Result<ReceiptFreshness, Error> {
    let context = tx.query_row(
            "SELECT e.target_publication_id,e.boot_id,e.instance_id,e.instance_expires_at,e.execution_expires_at,p.withdrawn_at,n.credentials_revoked_at FROM node_execution e JOIN nodes n ON n.network_id=e.network_id AND n.id=e.node_id JOIN execution_publications p ON p.network_id=e.network_id AND p.id=?3 WHERE e.network_id=?1 AND e.node_id=?2",
            params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice(), target.execution_publication.as_uuid().as_bytes().as_slice()],
            |row| Ok((row.get::<_, Option<Vec<u8>>>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, Vec<u8>>(2)?, row.get::<_, i64>(3)?, row.get::<_, Option<i64>>(4)?, row.get::<_, Option<i64>>(5)?, row.get::<_, Option<i64>>(6)?)),
        ).optional()?;
    let retired: bool = tx.query_row(
            "SELECT p.withdrawn_at IS NOT NULL OR n.credentials_revoked_at IS NOT NULL OR NOT EXISTS(SELECT 1 FROM node_keys k WHERE k.network_id=n.network_id AND k.node_id=n.id AND k.purpose='control' AND k.retired_at IS NULL AND k.revoked_at IS NULL) OR NOT EXISTS(SELECT 1 FROM node_keys k WHERE k.network_id=n.network_id AND k.node_id=n.id AND k.purpose='dataplane' AND k.retired_at IS NULL AND k.revoked_at IS NULL) FROM execution_publications p JOIN nodes n ON n.network_id=p.network_id AND n.id=?2 WHERE p.network_id=?1 AND p.id=?3",
            params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice(), target.execution_publication.as_uuid().as_bytes().as_slice()], |row| row.get(0),
        )?;
    let freshness = if retired {
        ReceiptFreshness::Superseded
    } else {
        match context {
            None => ReceiptFreshness::AwaitingInstance,
            Some((
                publication,
                current_boot,
                instance,
                expires,
                execution_expires,
                withdrawn,
                revoked,
            )) => {
                if publication.as_deref()
                    != Some(target.execution_publication.as_uuid().as_bytes().as_slice())
                    || withdrawn.is_some()
                    || revoked.is_some()
                {
                    ReceiptFreshness::Superseded
                } else if current_boot != boot.as_uuid().as_bytes()
                    || instance_id.is_some_and(|id: InstanceId| instance != id.as_uuid().as_bytes())
                {
                    ReceiptFreshness::AwaitingInstance
                } else if expires <= now.timestamp_millis()
                    || execution_expires.is_some_and(|at| at <= now.timestamp_millis())
                {
                    ReceiptFreshness::Expired
                } else {
                    ReceiptFreshness::Current
                }
            }
        }
    };
    Ok(freshness)
}
fn number(value: i64, column: &'static str) -> Result<u64, Error> {
    value.try_into().map_err(|_| Error::decode(column))
}
