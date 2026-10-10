//! SQL behind per-revision configuration progress: one ordered set of netmap
//! receipt facts per Node, read for the version's fixed netmap targets.
use crate::error::Error;
use chrono::{DateTime, Utc};
use meshway_model::{InstanceId, NetmapRef, NetworkId, NodeId, Revision, Sequence, Version};
use meshway_protocol::live::ApplicationPhase;
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::BTreeMap;

pub(crate) fn execution_targets(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
) -> Result<BTreeMap<NodeId, NetmapRef>, Error> {
    let encoded: String = tx.query_row(
        "SELECT execution_targets FROM configuration_versions WHERE network_id=?1 AND revision=?2",
        params![network.as_uuid().as_bytes().as_slice(), revision.get() as i64], |row| row.get(0),
    ).optional()?.ok_or(Error::NotFound)?;
    serde_json::from_str(&encoded)
        .map_err(|_| Error::decode("configuration_versions.execution_targets"))
}

pub(crate) struct Receipt {
    pub(crate) phase: ApplicationPhase,
    pub(crate) instance_id: InstanceId,
    pub(crate) attempt: Version,
    pub(crate) sequence: Sequence,
    pub(crate) received_at: DateTime<Utc>,
}

/// The latest receipt of `target` by instance generation, attempt and
/// sequence; arrival order is not application order.
pub(crate) fn latest_receipt(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
    target: &NetmapRef,
) -> Result<Option<Receipt>, Error> {
    let receipt = tx.query_row(
        "SELECT r.phase,r.instance_id,r.attempt,r.sequence,r.received_at FROM netmap_receipts r
         JOIN node_netmap_history m ON m.network_id=r.network_id AND m.node_id=r.node_id AND m.serial=r.serial
         WHERE r.network_id=?1 AND r.node_id=?2 AND r.serial=?3 AND m.digest=?4
         ORDER BY r.generation DESC,r.attempt DESC,r.sequence DESC LIMIT 1",
        params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice(), target.serial.get() as i64, target.digest.as_bytes().as_slice()],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?)),
    ).optional()?;
    receipt
        .map(|(phase, instance, attempt, sequence, at)| {
            Ok(Receipt {
                phase: match phase.as_str() {
                    "saved" => ApplicationPhase::Saved,
                    "applied" => ApplicationPhase::Applied,
                    "failed" => ApplicationPhase::Failed,
                    _ => return Err(Error::decode("netmap_receipts.phase")),
                },
                instance_id: super::id(instance)?,
                attempt: Version::new(number(attempt, "netmap_receipts.attempt")?)
                    .map_err(|_| Error::decode("netmap_receipts.attempt"))?,
                sequence: Sequence::new(number(sequence, "netmap_receipts.sequence")?)
                    .map_err(|_| Error::decode("netmap_receipts.sequence"))?,
                received_at: DateTime::from_timestamp_millis(at)
                    .ok_or(Error::decode("netmap_receipts.received_at"))?,
            })
        })
        .transpose()
}

pub(crate) struct FreshnessContext {
    pub(crate) boot_matches: bool,
    pub(crate) instance_matches: bool,
    pub(crate) expires: i64,
}

/// The instance context of a receipt and whether `target` is retired: no
/// longer the Node's current netmap, or its identity revoked.
pub(crate) fn freshness_context(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
    target: &NetmapRef,
    instance_id: Option<InstanceId>,
    boot: InstanceId,
) -> Result<(Option<FreshnessContext>, bool), Error> {
    let context = tx.query_row(
        "SELECT e.boot_id,e.instance_id,e.instance_expires_at FROM node_execution e WHERE e.network_id=?1 AND e.node_id=?2",
        params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice()],
        |row| Ok((row.get::<_, Option<Vec<u8>>>(0)?, row.get::<_, Option<Vec<u8>>>(1)?, row.get::<_, Option<i64>>(2)?)),
    ).optional()?;
    let revoked: bool = tx.query_row(
        "SELECT n.credentials_revoked_at IS NOT NULL OR NOT EXISTS(SELECT 1 FROM node_keys k WHERE k.network_id=n.network_id AND k.node_id=n.id AND k.purpose='control' AND k.retired_at IS NULL AND k.revoked_at IS NULL) OR NOT EXISTS(SELECT 1 FROM node_keys k WHERE k.network_id=n.network_id AND k.node_id=n.id AND k.purpose='dataplane' AND k.retired_at IS NULL AND k.revoked_at IS NULL) FROM nodes n WHERE n.network_id=?1 AND n.id=?2",
        params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice()], |row| row.get(0),
    )?;
    let current = crate::store::netmap::current(tx, network, node)?.map(|c| c.current);
    let retired = revoked || current != Some(*target);
    Ok((
        context.and_then(|(current_boot, instance, expires)| {
            let (current_boot, instance, expires) = (current_boot?, instance?, expires?);
            Some(FreshnessContext {
                boot_matches: current_boot == boot.as_uuid().as_bytes(),
                instance_matches: instance_id.is_none_or(|id| instance == id.as_uuid().as_bytes()),
                expires,
            })
        }),
        retired,
    ))
}

fn number(value: i64, column: &'static str) -> Result<u64, Error> {
    value.try_into().map_err(|_| Error::decode(column))
}
