//! One revision's immutable application denominator. Later demands/refreshes
//! never add targets or turn another publication's receipts into this result.
use crate::error::Error;
use crate::store::execution::progress as store;
use chrono::{DateTime, Utc};
use meshway_model::*;
use meshway_protocol::{execution::ReceiptFreshness, live::*};
use rusqlite::Transaction;
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
    let targets = store::execution_targets(tx, network, revision)?;
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
        let receipt = store::latest_receipt(tx, network, node, &target)?;
        let (phase, instance_id, attempt, sequence, received_at) = match receipt {
            None => (ApplicationPhase::Pending, None, None, None, None),
            Some(receipt) => (
                receipt.phase,
                Some(receipt.instance_id),
                Some(receipt.attempt),
                Some(receipt.sequence),
                Some(receipt.received_at),
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
    let (context, retired) =
        store::freshness_context(tx, network, node, target, instance_id, boot)?;
    let freshness = if retired {
        ReceiptFreshness::Superseded
    } else {
        match context {
            None => ReceiptFreshness::AwaitingInstance,
            Some(context) => {
                if !context.publication_matches
                    || context.withdrawn.is_some()
                    || context.revoked.is_some()
                {
                    ReceiptFreshness::Superseded
                } else if !context.boot_matches || !context.instance_matches {
                    ReceiptFreshness::AwaitingInstance
                } else if context.expires <= now.timestamp_millis()
                    || context
                        .execution_expires
                        .is_some_and(|at| at <= now.timestamp_millis())
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
