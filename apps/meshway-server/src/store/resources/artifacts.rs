//! Global content-addressed rule-set artifacts, their per-source records and
//! the accepted sequence of each signed catalog.
use crate::error::Error;
use chrono::{DateTime, Utc};
use meshway_protocol::resources::ResourceProblem;
use meshway_rules::Digest;
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::BTreeSet;

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

/// Records `record` for `digest` of `source`: the first one as immutable
/// provenance, and every one as the pair's current head.
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
    tx.execute(
        "INSERT INTO rule_set_record_heads(source,digest,record,updated_at) VALUES(?1,?2,?3,?4)
         ON CONFLICT(source,digest) DO UPDATE SET record=excluded.record,updated_at=excluded.updated_at",
        params![source, digest.as_bytes().as_slice(), record, now],
    )?;
    Ok(())
}

/// The current record of (`source`, `digest`): the latest preparation's.
pub(crate) fn load_record(
    tx: &Transaction<'_>,
    source: &str,
    digest: &Digest,
) -> Result<Option<String>, Error> {
    Ok(tx
        .query_row(
            "SELECT record FROM rule_set_record_heads WHERE source=?1 AND digest=?2",
            params![source, digest.as_bytes().as_slice()],
            |row| row.get(0),
        )
        .optional()?)
}

/// Accept `sequence` of a catalog verified under `trust`, keeping the
/// verified bytes. A lower sequence than the accepted one is a regression
/// and changes nothing.
pub(crate) fn accept_catalog(
    tx: &Transaction<'_>,
    catalog: &str,
    trust: &str,
    sequence: u64,
    (document, signature): (&[u8], &[u8]),
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
        "INSERT INTO rule_set_catalogs(catalog,trust,sequence,verified_at,document,signature)
         VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(catalog,trust) DO UPDATE SET sequence=excluded.sequence,
         verified_at=excluded.verified_at,document=excluded.document,signature=excluded.signature",
        params![catalog, trust, sequence, now, document, signature],
    )?;
    Ok(true)
}

/// Accepted `sequence` and verification time of (`catalog`, `trust`), if
/// this Server ever verified that catalog.
pub(crate) fn catalog_head(
    tx: &Transaction<'_>,
    catalog: &str,
    trust: &str,
) -> Result<Option<(u64, i64)>, Error> {
    tx.query_row(
        "SELECT sequence,verified_at FROM rule_set_catalogs WHERE catalog=?1 AND trust=?2",
        params![catalog, trust],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )
    .optional()?
    .map(|(sequence, at)| {
        u64::try_from(sequence)
            .map(|sequence| (sequence, at))
            .map_err(|_| Error::decode("rule_set_catalogs.sequence"))
    })
    .transpose()
}

fn digest_column(bytes: Vec<u8>) -> Result<Digest, Error> {
    <[u8; 32]>::try_from(bytes.as_slice())
        .map(Digest::from_bytes)
        .map_err(|_| Error::decode("rule_set_artifacts.digest"))
}

/// Every stored artifact.
pub(crate) fn stored(tx: &Transaction<'_>) -> Result<BTreeSet<Digest>, Error> {
    let mut statement = tx.prepare("SELECT digest FROM rule_set_artifacts")?;
    let rows = statement.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
    rows.map(|row| digest_column(row?)).collect()
}

/// Artifacts some Node netmap references that can still be under a lease at
/// `now`: the current one, or one superseded less than the longest lease ago.
pub(crate) fn leased(tx: &Transaction<'_>, now: i64) -> Result<BTreeSet<Digest>, Error> {
    let mut statement = tx.prepare(
        "SELECT DISTINCT digest FROM node_netmap_artifacts
         WHERE superseded_at IS NULL OR superseded_at>=?1",
    )?;
    let rows = statement.query_map([now.saturating_sub(MAX_LEASE_MS)], |row| {
        row.get::<_, Vec<u8>>(0)
    })?;
    rows.map(|row| digest_column(row?)).collect()
}

/// The refreshed binding of every version of a network that is not deleted.
pub(crate) fn refreshed_bindings(tx: &Transaction<'_>) -> Result<Vec<String>, Error> {
    let mut statement = tx.prepare(
        "SELECT b.bindings FROM rule_set_bindings b JOIN networks n ON n.id=b.network_id
         WHERE n.deleted_at IS NULL",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Removes an artifact and every record of it.
pub(crate) fn delete(tx: &Transaction<'_>, digest: &Digest) -> Result<(), Error> {
    let digest = digest.as_bytes().as_slice();
    tx.execute(
        "DELETE FROM rule_set_record_heads WHERE digest=?1",
        [digest],
    )?;
    tx.execute("DELETE FROM rule_set_records WHERE digest=?1", [digest])?;
    tx.execute("DELETE FROM rule_set_artifacts WHERE digest=?1", [digest])?;
    Ok(())
}

/// The Server's last own fetch of (`catalog`, `trust`): when, and the
/// consecutive failures since the last success. The problem is kept for
/// diagnosis only.
pub(crate) struct CatalogCheck {
    pub(crate) checked_at: i64,
    pub(crate) failures: u32,
}

pub(crate) fn catalog_check(
    tx: &Transaction<'_>,
    catalog: &str,
    trust: &str,
) -> Result<Option<CatalogCheck>, Error> {
    Ok(tx
        .query_row(
            "SELECT checked_at,failures FROM rule_set_catalog_checks WHERE catalog=?1 AND trust=?2",
            params![catalog, trust],
            |row| {
                Ok(CatalogCheck {
                    checked_at: row.get(0)?,
                    failures: row.get(1)?,
                })
            },
        )
        .optional()?)
}

/// Records one fetch of (`catalog`, `trust`); a success resets the failures.
pub(crate) fn record_catalog_check(
    tx: &Transaction<'_>,
    catalog: &str,
    trust: &str,
    problem: Option<ResourceProblem>,
    now: i64,
) -> Result<(), Error> {
    let stored = problem
        .map(|problem| serde_json::to_string(&problem))
        .transpose()
        .map_err(|_| Error::encode("rule-set catalog check problem"))?;
    tx.execute(
        "INSERT INTO rule_set_catalog_checks(catalog,trust,problem,checked_at,failures)
         VALUES(?1,?2,?3,?4,?5) ON CONFLICT(catalog,trust) DO UPDATE SET
         problem=excluded.problem,checked_at=excluded.checked_at,
         failures=CASE WHEN excluded.problem IS NULL THEN 0 ELSE min(rule_set_catalog_checks.failures+1,16) END",
        params![catalog, trust, stored, now, i64::from(problem.is_some())],
    )?;
    Ok(())
}

/// The accepted `index.json` and `index.json.sig` bytes.
/// The accepted signature bytes of (`catalog`, `trust`): small, and with the
/// sequence the identity of the accepted document.
pub(crate) fn catalog_signature(
    tx: &Transaction<'_>,
    catalog: &str,
    trust: &str,
) -> Result<Vec<u8>, Error> {
    tx.query_row(
        "SELECT signature FROM rule_set_catalogs WHERE catalog=?1 AND trust=?2",
        params![catalog, trust],
        |row| row.get(0),
    )
    .optional()?
    .ok_or(Error::NotFound)
}

pub(crate) fn catalog_document(
    tx: &Transaction<'_>,
    catalog: &str,
    trust: &str,
) -> Result<(Vec<u8>, Vec<u8>), Error> {
    tx.query_row(
        "SELECT document,signature FROM rule_set_catalogs WHERE catalog=?1 AND trust=?2",
        params![catalog, trust],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()?
    .ok_or(Error::NotFound)
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

/// The longest netmap lease validity (`netmap::SyncTiming`): a netmap
/// superseded longer ago than this can no longer authorize or need anything.
pub(crate) const MAX_LEASE_MS: i64 = 24 * 60 * 60 * 1000;

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
