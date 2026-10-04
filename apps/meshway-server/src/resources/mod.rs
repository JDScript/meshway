//! Bounded dataset conversion/download and immutable content. GeoIP datasets
//! are supplied by deployment, never by network administrators or Node reports.
pub(crate) mod bindings;
pub mod datasets;
pub(crate) mod download;
pub(crate) mod downloads;
mod geoip;
pub mod provider;
mod rulesets;
pub(crate) mod scheduler;
pub mod service;
pub use geoip::GeoIp;
pub use rulesets::{convert, normalize};
pub async fn download(
    source: &str,
) -> Result<Vec<u8>, meshway_protocol::resources::ResourceProblem> {
    download::download(source, &download::Https).await
}

use crate::error::Error;
use chrono::{DateTime, Utc};
use meshway_model::{ArtifactId, Digest, NetworkId};
use meshway_protocol::resources::{ArtifactSummary, RuleSetData};
use rusqlite::{OptionalExtension, Transaction, params};
use sha2::{Digest as _, Sha256};

pub const MAX_DATASET_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RULE_ENTRIES: usize = 100_000;

pub fn content_digest(bytes: &[u8]) -> Digest {
    let text: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    text.parse().expect("SHA-256 has a canonical digest")
}
/// Call in the owning configuration/execution transaction. The returned ID
/// always identifies immutable canonical bytes; URL refresh never overwrites it.
pub fn store_artifact(
    tx: &Transaction<'_>,
    network: NetworkId,
    source: &str,
    data: &RuleSetData,
    now: DateTime<Utc>,
) -> Result<ArtifactSummary, Error> {
    let data = normalize(data.clone()).map_err(|_| Error::InvalidRequest)?;
    let bytes = serde_json::to_vec(&data).map_err(|_| Error::encode("rule set data"))?;
    if bytes.len() > MAX_DATASET_BYTES {
        return Err(Error::StateLimit);
    }
    let digest = content_digest(&bytes);
    if let Some((id, created)) = tx.query_row("SELECT id, created_at FROM dataset_artifacts WHERE network_id = ?1 AND kind = 'ruleSet' AND digest = ?2", params![network.as_uuid().as_bytes().as_slice(), digest.to_string()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))).optional()? {
        return Ok(ArtifactSummary { id: id.parse().map_err(|_| Error::decode("dataset_artifacts.id"))?, digest, created_at: DateTime::from_timestamp_millis(created).ok_or(Error::decode("dataset_artifacts.created_at"))?, entries: (data.ip_cidr.len() + data.domain_suffix.len()) as u32 });
    }
    let id = ArtifactId::new(crate::accounts::random_uuid()?)
        .map_err(|_| Error::Internal("generated artifact ID"))?;
    tx.execute("INSERT INTO dataset_artifacts (id, network_id, kind, source, format, data, digest, created_at) VALUES (?1, ?2, 'ruleSet', ?3, 'meshway-rules-v1', ?4, ?5, ?6)", params![id.to_string(), network.as_uuid().as_bytes().as_slice(), source, bytes, digest.to_string(), now.timestamp_millis()])?;
    Ok(ArtifactSummary {
        id,
        digest,
        created_at: DateTime::from_timestamp_millis(now.timestamp_millis())
            .ok_or(Error::Internal("artifact creation timestamp"))?,
        entries: (data.ip_cidr.len() + data.domain_suffix.len()) as u32,
    })
}
pub fn load_artifact(
    tx: &Transaction<'_>,
    network: NetworkId,
    id: ArtifactId,
) -> Result<RuleSetData, Error> {
    let (bytes, digest): (Vec<u8>, String) = tx.query_row("SELECT data, digest FROM dataset_artifacts WHERE id = ?1 AND network_id = ?2 AND kind = 'ruleSet' AND format = 'meshway-rules-v1'", params![id.to_string(), network.as_uuid().as_bytes().as_slice()], |row| Ok((row.get(0)?, row.get(1)?))).optional()?.ok_or(Error::NotFound)?;
    if content_digest(&bytes).to_string() != digest {
        return Err(Error::decode("dataset_artifacts.digest"));
    }
    let data: RuleSetData =
        serde_json::from_slice(&bytes).map_err(|_| Error::decode("dataset_artifacts.data"))?;
    let normalized =
        normalize(data.clone()).map_err(|_| Error::decode("dataset_artifacts.data"))?;
    if data != normalized
        || serde_json::to_vec(&data).map_err(|_| Error::encode("rule set data"))? != bytes
    {
        return Err(Error::decode("dataset_artifacts.data"));
    }
    Ok(data)
}
