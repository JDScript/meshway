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

pub(crate) fn current(
    tx: &Transaction<'_>,
    kind: IpDatasetKind,
) -> Result<Option<(ArtifactId, MmdbContent)>, Error> {
    let row: Option<(String, Vec<u8>, String)> = tx.query_row(
        "SELECT a.id,a.data,a.digest FROM ip_dataset_heads h JOIN dataset_artifacts a ON a.id=h.artifact_id WHERE h.kind=?1 AND a.kind=h.kind AND a.network_id IS NULL AND a.format='mmdb'",
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
    tx.execute(
        "INSERT INTO ip_dataset_heads(kind,artifact_id) VALUES(?1,?2) ON CONFLICT(kind) DO UPDATE SET artifact_id=excluded.artifact_id",
        params![key(kind), id.to_string()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_follows_commit_order_across_equal_or_reversed_clocks_vacuum_and_rollback() {
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        crate::store::schema::tests::initialize_fixture(&mut db).unwrap();
        let first = "ffffffff-ffff-4fff-8fff-ffffffffffff".parse().unwrap();
        let second = "11111111-1111-4111-8111-111111111111".parse().unwrap();
        let third = "22222222-2222-4222-8222-222222222222".parse().unwrap();
        let content = MmdbContent(vec![1, 2, 3]);
        let digest = content_digest(&content.0);
        for (id, time) in [(first, 2000), (second, 2000), (third, 1000)] {
            let tx = db.transaction().unwrap();
            insert(
                &tx,
                IpDatasetKind::GeoIp,
                "test",
                id,
                &content,
                &digest,
                DateTime::from_timestamp_millis(time).unwrap(),
            )
            .unwrap();
            tx.commit().unwrap();
            let tx = db.transaction().unwrap();
            assert_eq!(current(&tx, IpDatasetKind::GeoIp).unwrap().unwrap().0, id);
            tx.rollback().unwrap();
        }
        db.execute_batch("VACUUM").unwrap();
        let tx = db.transaction().unwrap();
        let rolled_back = "33333333-3333-4333-8333-333333333333".parse().unwrap();
        insert(
            &tx,
            IpDatasetKind::GeoIp,
            "rolled back",
            rolled_back,
            &content,
            &digest,
            Utc::now(),
        )
        .unwrap();
        tx.rollback().unwrap();
        let tx = db.transaction().unwrap();
        assert_eq!(
            current(&tx, IpDatasetKind::GeoIp).unwrap().unwrap().0,
            third
        );
        assert!(current(&tx, IpDatasetKind::Asn).unwrap().is_none());
        tx.rollback().unwrap();
        crate::store::schema::check(&mut db).unwrap();
    }
}
