//! Global content-addressed rule-set artifacts, their per-source records and
//! the accepted sequence of each signed catalog.
use crate::error::Error;
use chrono::{DateTime, Utc};
use meshway_rules::Digest;
use rusqlite::{OptionalExtension, Transaction, params};

/// Stores the artifact bytes once; an existing digest is left unchanged.
/// The caller has built or verified `bytes` and computed `digest` from them.
pub(crate) fn insert(
    tx: &Transaction<'_>,
    digest: &Digest,
    bytes: &[u8],
    now: i64,
) -> Result<(), Error> {
    tx.execute(
        "INSERT INTO rule_set_artifacts(digest,data,created_at) VALUES(?1,?2,?3) ON CONFLICT(digest) DO NOTHING",
        params![digest.as_bytes().as_slice(), bytes, now],
    )?;
    Ok(())
}

pub(crate) fn exists(tx: &Transaction<'_>, digest: &Digest) -> Result<bool, Error> {
    Ok(tx
        .query_row(
            "SELECT 1 FROM rule_set_artifacts WHERE digest=?1",
            [digest.as_bytes().as_slice()],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// The stored bytes, checked against their digest.
pub(crate) fn load(tx: &Transaction<'_>, digest: &Digest) -> Result<Vec<u8>, Error> {
    let bytes: Vec<u8> = tx
        .query_row(
            "SELECT data FROM rule_set_artifacts WHERE digest=?1",
            [digest.as_bytes().as_slice()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    if Digest::of(&bytes) != *digest {
        return Err(Error::decode("rule_set_artifacts.data"));
    }
    Ok(bytes)
}

pub(crate) fn created_at(tx: &Transaction<'_>, digest: &Digest) -> Result<DateTime<Utc>, Error> {
    let at: i64 = tx
        .query_row(
            "SELECT created_at FROM rule_set_artifacts WHERE digest=?1",
            [digest.as_bytes().as_slice()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    DateTime::from_timestamp_millis(at).ok_or(Error::decode("rule_set_artifacts.created_at"))
}

/// Records provenance of `digest` for `source` once; an existing record of
/// the same pair is kept.
pub(crate) fn record(
    tx: &Transaction<'_>,
    source: &str,
    digest: &Digest,
    record: &str,
    now: i64,
) -> Result<(), Error> {
    tx.execute(
        "INSERT INTO rule_set_records(source,digest,record,created_at) VALUES(?1,?2,?3,?4)
         ON CONFLICT(source,digest) DO NOTHING",
        params![source, digest.as_bytes().as_slice(), record, now],
    )?;
    Ok(())
}

pub(crate) fn load_record(
    tx: &Transaction<'_>,
    source: &str,
    digest: &Digest,
) -> Result<Option<String>, Error> {
    Ok(tx
        .query_row(
            "SELECT record FROM rule_set_records WHERE source=?1 AND digest=?2",
            params![source, digest.as_bytes().as_slice()],
            |row| row.get(0),
        )
        .optional()?)
}

/// Accept `sequence` of a catalog verified under `trust`. A lower sequence
/// than the accepted one is a regression and changes nothing.
pub(crate) fn accept_catalog(
    tx: &Transaction<'_>,
    catalog: &str,
    trust: &str,
    sequence: u64,
    now: i64,
) -> Result<bool, Error> {
    let accepted: Option<i64> = tx
        .query_row(
            "SELECT sequence FROM rule_set_catalogs WHERE catalog=?1 AND trust=?2",
            params![catalog, trust],
            |row| row.get(0),
        )
        .optional()?;
    let sequence = i64::try_from(sequence).map_err(|_| Error::InvalidRequest)?;
    if accepted.is_some_and(|accepted| sequence < accepted) {
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO rule_set_catalogs(catalog,trust,sequence,verified_at) VALUES(?1,?2,?3,?4)
         ON CONFLICT(catalog,trust) DO UPDATE SET sequence=excluded.sequence,verified_at=excluded.verified_at",
        params![catalog, trust, sequence, now],
    )?;
    Ok(true)
}

/// Records that `serial` of a Node's netmap references `digests`, and that
/// every earlier serial was superseded at `now`. Rows superseded longer than
/// the longest lease validity can no longer authorize anything.
pub(crate) fn reference(
    tx: &Transaction<'_>,
    network: &[u8],
    node: &[u8],
    serial: i64,
    digests: impl IntoIterator<Item = Digest>,
    now: i64,
) -> Result<(), Error> {
    tx.execute(
        "UPDATE node_netmap_artifacts SET superseded_at=?4
         WHERE network_id=?1 AND node_id=?2 AND serial<?3 AND superseded_at IS NULL",
        params![network, node, serial, now],
    )?;
    tx.execute(
        "DELETE FROM node_netmap_artifacts WHERE network_id=?1 AND node_id=?2 AND superseded_at<?3",
        params![network, node, now.saturating_sub(MAX_LEASE_MS)],
    )?;
    for digest in digests {
        tx.execute(
            "INSERT INTO node_netmap_artifacts(network_id,node_id,serial,digest,superseded_at)
             VALUES(?1,?2,?3,?4,NULL) ON CONFLICT DO UPDATE SET superseded_at=NULL",
            params![network, node, serial, digest.as_bytes().as_slice()],
        )?;
    }
    Ok(())
}

/// The Node's current netmap was withdrawn at `at`: no lease is issued for
/// it any more.
pub(crate) fn withdraw(
    tx: &Transaction<'_>,
    network: &[u8],
    node: &[u8],
    at: i64,
) -> Result<(), Error> {
    tx.execute(
        "UPDATE node_netmap_artifacts SET superseded_at=?3
         WHERE network_id=?1 AND node_id=?2 AND superseded_at IS NULL",
        params![network, node, at],
    )?;
    Ok(())
}

/// A withdrawn current netmap `serial` is current again.
pub(crate) fn restore(
    tx: &Transaction<'_>,
    network: &[u8],
    node: &[u8],
    serial: i64,
) -> Result<(), Error> {
    tx.execute(
        "UPDATE node_netmap_artifacts SET superseded_at=NULL
         WHERE network_id=?1 AND node_id=?2 AND serial=?3",
        params![network, node, serial],
    )?;
    Ok(())
}

const MAX_LEASE_MS: i64 = 24 * 60 * 60 * 1000;

/// Whether some netmap of this Node that references `digest` can still be
/// under a lease of `validity_ms` at `now`.
pub(crate) fn referenced(
    tx: &Transaction<'_>,
    network: &[u8],
    node: &[u8],
    digest: &Digest,
    now: i64,
    validity_ms: i64,
) -> Result<bool, Error> {
    Ok(tx
        .query_row(
            "SELECT 1 FROM node_netmap_artifacts WHERE network_id=?1 AND node_id=?2 AND digest=?3
             AND (superseded_at IS NULL OR superseded_at>?4) LIMIT 1",
            params![
                network,
                node,
                digest.as_bytes().as_slice(),
                now.saturating_sub(validity_ms)
            ],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}
