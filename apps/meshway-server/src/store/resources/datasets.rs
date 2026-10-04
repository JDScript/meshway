use crate::error::Error;
use crate::resources::{content_digest, datasets::IpDatasetKind};
use chrono::{DateTime, Utc};
use meshway_model::{ArtifactId, Digest};
use rusqlite::{OptionalExtension, Transaction, params};

pub(crate) struct MmdbContent(pub(crate) Vec<u8>);

fn key(kind: IpDatasetKind) -> &'static str {
    match kind {
        IpDatasetKind::GeoIp => "geoip",
        IpDatasetKind::Asn => "asn",
    }
}

pub(crate) fn latest(
    tx: &Transaction<'_>,
    kind: IpDatasetKind,
) -> Result<Option<(ArtifactId, MmdbContent)>, Error> {
    let row: Option<(String, Vec<u8>, String)> = tx.query_row(
        "SELECT id,data,digest FROM dataset_artifacts WHERE network_id IS NULL AND kind=?1 AND format='mmdb' ORDER BY created_at DESC,id DESC LIMIT 1",
        [key(kind)], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
    row.map(|(id, bytes, digest)| {
        if content_digest(&bytes).to_string() != digest {
            return Err(Error::decode("dataset_artifacts.digest"));
        }
        let id = id
            .parse()
            .map_err(|_| Error::decode("dataset_artifacts.id"))?;
        Ok((id, MmdbContent(bytes)))
    })
    .transpose()
}

pub(crate) fn insert(
    tx: &Transaction<'_>,
    kind: IpDatasetKind,
    source: &str,
    id: ArtifactId,
    content: &MmdbContent,
    digest: &Digest,
    now: DateTime<Utc>,
) -> Result<(), Error> {
    tx.execute("INSERT INTO dataset_artifacts(id,network_id,kind,source,format,data,digest,created_at) VALUES (?1,NULL,?2,?3,'mmdb',?4,?5,?6)",
        params![id.to_string(), key(kind), source, content.0, digest.to_string(), now.timestamp_millis()])?;
    Ok(())
}
