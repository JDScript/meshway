use crate::error::Error;
use meshway_model::{ArtifactId, Digest, NetworkId, Revision};
use rusqlite::{OptionalExtension, Transaction, params};

pub(crate) fn artifact_by_digest(
    tx: &Transaction<'_>,
    network: NetworkId,
    digest: Digest,
) -> Result<Option<ArtifactId>, Error> {
    let id: Option<String> = tx
        .query_row(
            "SELECT id FROM dataset_artifacts WHERE network_id=?1 AND kind='ruleSet' AND digest=?2",
            params![network.as_uuid().as_bytes().as_slice(), digest.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    id.map(|id| {
        id.parse()
            .map_err(|_| Error::decode("dataset_artifacts.id"))
    })
    .transpose()
}

pub(crate) fn record_check(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
    key: &str,
    now: i64,
) -> Result<(), Error> {
    tx.execute(
        "INSERT INTO rule_set_checks(network_id,revision,rule_set_key,checked_at,last_success_at,problem,failures)
         VALUES(?1,?2,?3,?4,?4,NULL,0) ON CONFLICT(network_id,revision,rule_set_key)
         DO UPDATE SET checked_at=excluded.checked_at,last_success_at=excluded.last_success_at,problem=NULL,failures=0,
             check_version=rule_set_checks.check_version+1",
        params![network.as_uuid().as_bytes().as_slice(),revision.get() as i64,key,now],
    )?;
    Ok(())
}
