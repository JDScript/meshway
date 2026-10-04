use crate::error::Error;
use meshway_model::{Digest, NetworkId};
use rusqlite::{Transaction, params};

pub(crate) fn check_revocation(
    tx: &Transaction<'_>,
    network: NetworkId,
    kind: &str,
    fingerprint: Digest,
) -> Result<bool, Error> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM credential_revocations WHERE network_id=?1 AND kind=?2 AND fingerprint=?3)",
        params![network.as_uuid().as_bytes().as_slice(), kind, fingerprint.as_bytes().as_slice()],
        |row| row.get::<_, bool>(0),
    )?)
}
