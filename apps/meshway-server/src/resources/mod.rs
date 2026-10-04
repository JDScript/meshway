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
use crate::store;
use chrono::{DateTime, Utc};
use meshway_model::{ArtifactId, Digest, NetworkId};
use meshway_protocol::resources::{ArtifactSummary, RuleSetData};
use rusqlite::Transaction;
use sha2::{Digest as _, Sha256};

pub const MAX_DATASET_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RULE_ENTRIES: usize = 100_000;
/// Remote rule sets one configuration may need to download in a single edit.
pub const MAX_REMOTE_RULE_SETS: usize = 32;

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
    if let Some((id, created_at)) = store::resources::artifacts::find(tx, network, &digest)? {
        return Ok(ArtifactSummary {
            id,
            digest,
            created_at,
            entries: (data.ip_cidr.len() + data.domain_suffix.len()) as u32,
        });
    }
    let id = ArtifactId::new(crate::accounts::random_uuid()?)
        .map_err(|_| Error::Internal("generated artifact ID"))?;
    store::resources::artifacts::insert(
        tx,
        network,
        source,
        id,
        &store::resources::artifacts::RuleSetContent(bytes),
        &digest,
        now,
    )?;
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
    store::resources::artifacts::load(tx, network, id)
}
