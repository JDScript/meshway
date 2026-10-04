use crate::error::Error;
use crate::resources::{content_digest, normalize};
use chrono::{DateTime, Utc};
use meshway_model::{ArtifactId, Digest, NetworkId};
use meshway_protocol::resources::RuleSetData;
use rusqlite::{OptionalExtension, Transaction, params};

pub(crate) struct RuleSetContent(pub(crate) Vec<u8>);

pub(crate) fn find(
    tx: &Transaction<'_>,
    network: NetworkId,
    digest: &Digest,
) -> Result<Option<(ArtifactId, DateTime<Utc>)>, Error> {
    let row = tx.query_row("SELECT id, created_at FROM dataset_artifacts WHERE network_id = ?1 AND kind = 'ruleSet' AND digest = ?2", params![network.as_uuid().as_bytes().as_slice(), digest.to_string()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))).optional()?;
    row.map(|(id, created)| {
        Ok((
            id.parse()
                .map_err(|_| Error::decode("dataset_artifacts.id"))?,
            DateTime::from_timestamp_millis(created)
                .ok_or(Error::decode("dataset_artifacts.created_at"))?,
        ))
    })
    .transpose()
}

pub(crate) fn insert(
    tx: &Transaction<'_>,
    network: NetworkId,
    source: &str,
    id: ArtifactId,
    content: &RuleSetContent,
    digest: &Digest,
    now: DateTime<Utc>,
) -> Result<(), Error> {
    tx.execute("INSERT INTO dataset_artifacts (id, network_id, kind, source, format, data, digest, created_at) VALUES (?1, ?2, 'ruleSet', ?3, 'meshway-rules-v1', ?4, ?5, ?6)", params![id.to_string(), network.as_uuid().as_bytes().as_slice(), source, content.0, digest.to_string(), now.timestamp_millis()])?;
    Ok(())
}

pub(crate) fn load(
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

pub(crate) fn metadata(
    tx: &Transaction<'_>,
    network: NetworkId,
    id: ArtifactId,
) -> Result<(Digest, DateTime<Utc>), Error> {
    let (digest, created): (String, i64) = tx.query_row(
        "SELECT digest,created_at FROM dataset_artifacts WHERE network_id=?1 AND id=?2 AND kind='ruleSet'",
        params![network.as_uuid().as_bytes().as_slice(), id.to_string()], |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok((
        digest
            .parse()
            .map_err(|_| Error::decode("dataset_artifacts.digest"))?,
        DateTime::from_timestamp_millis(created)
            .ok_or(Error::decode("dataset_artifacts.created_at"))?,
    ))
}
