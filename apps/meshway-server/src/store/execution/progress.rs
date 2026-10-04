//! SQL behind per-revision configuration progress.
use crate::error::Error;
use chrono::{DateTime, Utc};
use meshway_model::{ConfigurationRef, InstanceId, NetworkId, NodeId, Revision, Sequence, Version};
use meshway_protocol::live::ApplicationPhase;
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::BTreeMap;

pub(crate) fn execution_targets(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
) -> Result<BTreeMap<NodeId, ConfigurationRef>, Error> {
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

pub(crate) fn latest_receipt(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
    target: &ConfigurationRef,
) -> Result<Option<Receipt>, Error> {
    let receipt = tx.query_row(
        "SELECT phase,instance_id,attempt,sequence,received_at FROM execution_receipts WHERE network_id=?1 AND node_id=?2 AND publication_id=?3 AND digest=?4 ORDER BY generation DESC,attempt DESC,sequence DESC LIMIT 1",
        params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice(), target.execution_publication.as_uuid().as_bytes().as_slice(), target.digest.as_bytes().as_slice()],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?)),
    ).optional()?;
    receipt
        .map(|(phase, instance, attempt, sequence, at)| {
            Ok(Receipt {
                phase: match phase.as_str() {
                    "saved" => ApplicationPhase::Saved,
                    "applied" => ApplicationPhase::Applied,
                    "failed" => ApplicationPhase::Failed,
                    _ => return Err(Error::decode("execution_receipts.phase")),
                },
                instance_id: super::id(instance)?,
                attempt: Version::new(number(attempt, "execution_receipts.attempt")?)
                    .map_err(|_| Error::decode("execution_receipts.attempt"))?,
                sequence: Sequence::new(number(sequence, "execution_receipts.sequence")?)
                    .map_err(|_| Error::decode("execution_receipts.sequence"))?,
                received_at: DateTime::from_timestamp_millis(at)
                    .ok_or(Error::decode("execution_receipts.received_at"))?,
            })
        })
        .transpose()
}

pub(crate) struct FreshnessContext {
    pub(crate) publication_matches: bool,
    pub(crate) boot_matches: bool,
    pub(crate) instance_matches: bool,
    pub(crate) expires: i64,
    pub(crate) execution_expires: Option<i64>,
    pub(crate) withdrawn: Option<i64>,
    pub(crate) revoked: Option<i64>,
}

pub(crate) fn freshness_context(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
    target: &ConfigurationRef,
    instance_id: Option<InstanceId>,
    boot: InstanceId,
) -> Result<(Option<FreshnessContext>, bool), Error> {
    let context = tx.query_row(
        "SELECT e.target_publication_id,e.boot_id,e.instance_id,e.instance_expires_at,e.execution_expires_at,p.withdrawn_at,n.credentials_revoked_at FROM node_execution e JOIN nodes n ON n.network_id=e.network_id AND n.id=e.node_id JOIN execution_publications p ON p.network_id=e.network_id AND p.id=?3 WHERE e.network_id=?1 AND e.node_id=?2",
        params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice(), target.execution_publication.as_uuid().as_bytes().as_slice()],
        |row| Ok((row.get::<_, Option<Vec<u8>>>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, Vec<u8>>(2)?, row.get::<_, i64>(3)?, row.get::<_, Option<i64>>(4)?, row.get::<_, Option<i64>>(5)?, row.get::<_, Option<i64>>(6)?)),
    ).optional()?;
    let retired: bool = tx.query_row(
        "SELECT p.withdrawn_at IS NOT NULL OR n.credentials_revoked_at IS NOT NULL OR NOT EXISTS(SELECT 1 FROM node_keys k WHERE k.network_id=n.network_id AND k.node_id=n.id AND k.purpose='control' AND k.retired_at IS NULL AND k.revoked_at IS NULL) OR NOT EXISTS(SELECT 1 FROM node_keys k WHERE k.network_id=n.network_id AND k.node_id=n.id AND k.purpose='dataplane' AND k.retired_at IS NULL AND k.revoked_at IS NULL) FROM execution_publications p JOIN nodes n ON n.network_id=p.network_id AND n.id=?2 WHERE p.network_id=?1 AND p.id=?3",
        params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice(), target.execution_publication.as_uuid().as_bytes().as_slice()], |row| row.get(0),
    )?;
    Ok((
        context.map(
            |(
                publication,
                current_boot,
                instance,
                expires,
                execution_expires,
                withdrawn,
                revoked,
            )| FreshnessContext {
                publication_matches: publication.as_deref()
                    == Some(target.execution_publication.as_uuid().as_bytes().as_slice()),
                boot_matches: current_boot == boot.as_uuid().as_bytes(),
                instance_matches: instance_id.is_none_or(|id| instance == id.as_uuid().as_bytes()),
                expires,
                execution_expires,
                withdrawn,
                revoked,
            },
        ),
        retired,
    ))
}

fn number(value: i64, column: &'static str) -> Result<u64, Error> {
    value.try_into().map_err(|_| Error::decode(column))
}
