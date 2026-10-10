use crate::error::Error;
use crate::resources::bindings::Bindings;
use meshway_model::{NetworkId, Revision};
use rusqlite::{OptionalExtension, Transaction, params};

/// The refreshed binding of one configuration version, if any refresh bound one.
pub(crate) fn current(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
) -> Result<Option<Bindings>, Error> {
    let text: Option<String> = tx
        .query_row(
            "SELECT bindings FROM rule_set_bindings WHERE network_id=?1 AND revision=?2",
            params![
                network.as_uuid().as_bytes().as_slice(),
                revision.get() as i64
            ],
            |row| row.get(0),
        )
        .optional()?;
    text.map(|text| {
        serde_json::from_str(&text).map_err(|_| Error::decode("rule_set_bindings.bindings"))
    })
    .transpose()
}

/// Replaces the version's current binding.
pub(crate) fn record(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
    bindings: &Bindings,
    now: i64,
) -> Result<(), Error> {
    let text = serde_json::to_string(bindings).map_err(|_| Error::encode("rule set bindings"))?;
    tx.execute(
        "INSERT INTO rule_set_bindings(network_id,revision,bindings,bound_at) VALUES(?1,?2,?3,?4)
         ON CONFLICT(network_id,revision) DO UPDATE SET bindings=excluded.bindings,bound_at=excluded.bound_at",
        params![network.as_uuid().as_bytes().as_slice(), revision.get() as i64, text, now],
    )?;
    Ok(())
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
