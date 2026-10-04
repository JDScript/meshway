use crate::error::Error;
use crate::resources::bindings::Bindings;
use meshway_model::{NetworkId, PublicationId, Revision};
use rusqlite::{OptionalExtension, Transaction, params};

pub(crate) fn latest_publication(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
) -> Result<Option<PublicationId>, Error> {
    let id: Option<Vec<u8>> = tx.query_row(
        "SELECT id FROM execution_publications WHERE network_id=?1 AND revision=?2 ORDER BY sequence DESC LIMIT 1",
        params![network.as_uuid().as_bytes().as_slice(), revision.get() as i64],
        |row| row.get(0),
    ).optional()?;
    id.map(crate::store::execution::id).transpose()
}

pub(crate) fn at_publication(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
    publication: PublicationId,
) -> Result<Bindings, Error> {
    let bytes: Vec<u8> = tx.query_row(
        "SELECT rule_set_bindings FROM execution_publications WHERE network_id=?1 AND revision=?2 AND id=?3",
        params![
            network.as_uuid().as_bytes().as_slice(),
            revision.get() as i64,
            publication.as_uuid().as_bytes().as_slice(),
        ],
        |row| row.get(0),
    ).optional()?.ok_or(Error::DependencyUnavailable)?;
    serde_json::from_slice(&bytes)
        .map_err(|_| Error::decode("execution_publications.rule_set_bindings"))
}

pub(crate) fn begin(tx: &Transaction<'_>) -> Result<(), Error> {
    tx.execute_batch("SAVEPOINT resource_binding")?;
    Ok(())
}

pub(crate) fn release(tx: &Transaction<'_>) -> Result<(), Error> {
    tx.execute_batch("RELEASE resource_binding")?;
    Ok(())
}

pub(crate) fn rollback(tx: &Transaction<'_>) -> Result<(), Error> {
    tx.execute_batch("ROLLBACK TO resource_binding; RELEASE resource_binding")?;
    Ok(())
}
