//! Global content-addressed artifacts: one stored copy per digest and one
//! loaded `Arc<RuleSet>` per digest in this process, shared by every network
//! and Node compilation.
use super::rulesets::{Prepared, Record, Source};
use crate::{error::Error, store::resources::artifacts as sql};
use meshway_protocol::resources::{ArtifactSummary, ResourceProblem};
use meshway_rules::{Digest, RuleSet};
use rusqlite::Transaction;
use std::{
    collections::BTreeMap,
    sync::{Arc, LazyLock, Mutex},
};

/// Loaded sets kept beyond their last user, by total bytes.
const CACHE_BYTES: usize = 512 << 20;
static LOADED: LazyLock<Mutex<BTreeMap<Digest, Arc<RuleSet>>>> = LazyLock::new(Mutex::default);

/// The process's one handle of `digest`, read and verified once.
pub(crate) fn load(tx: &Transaction<'_>, digest: &Digest) -> Result<Arc<RuleSet>, Error> {
    if let Some(set) = LOADED.lock().map_err(|_| Error::RUNTIME)?.get(digest) {
        return Ok(Arc::clone(set));
    }
    let bytes = sql::load(tx, digest)?;
    let set =
        Arc::new(RuleSet::from_vec(bytes).map_err(|_| Error::decode("rule_set_artifacts.data"))?);
    if set.digest() != *digest {
        return Err(Error::decode("rule_set_artifacts.digest"));
    }
    let mut loaded = LOADED.lock().map_err(|_| Error::RUNTIME)?;
    let mut total: usize = loaded.values().map(|s| s.as_bytes().len()).sum();
    if total + set.as_bytes().len() > CACHE_BYTES {
        loaded.retain(|_, s| {
            let keep = Arc::strong_count(s) > 1;
            if !keep {
                total -= s.as_bytes().len();
            }
            keep
        });
    }
    Ok(Arc::clone(
        loaded.entry(*digest).or_insert_with(|| Arc::clone(&set)),
    ))
}

pub(crate) fn summary(tx: &Transaction<'_>, digest: &Digest) -> Result<ArtifactSummary, Error> {
    let set = load(tx, digest)?;
    Ok(ArtifactSummary {
        digest: digest.to_string(),
        format: meshway_rules::MAJOR,
        bytes: meshway_protocol::resources::count(set.as_bytes().len() as u64),
        entries: meshway_protocol::resources::count(set.counts().entries()),
        created_at: sql::created_at(tx, digest)?,
    })
}

/// Stores prepared content and its record, and accepts its catalog
/// sequence. A catalog regression stores nothing.
pub(crate) fn store(
    tx: &Transaction<'_>,
    source: &Source,
    prepared: &Prepared,
    now: i64,
) -> Result<Result<Digest, ResourceProblem>, Error> {
    if let Some(catalog) = &prepared.catalog
        && !sql::accept_catalog(
            tx,
            &catalog.catalog,
            &catalog.trust,
            catalog.sequence,
            (&catalog.document, &catalog.signature),
            now,
        )?
    {
        return Ok(Err(ResourceProblem::CatalogRegressed));
    }
    let artifact = &prepared.artifact;
    sql::insert(tx, &artifact.digest(), artifact.bytes(), now)?;
    let record =
        serde_json::to_string(&prepared.record).map_err(|_| Error::encode("rule-set record"))?;
    sql::record(tx, &source.identity(), &artifact.digest(), &record, now)?;
    Ok(Ok(artifact.digest()))
}

/// Deletes the artifacts nothing can use any more, with their records, and
/// returns how many. Retention follows configuration history: versions are
/// never deleted, so every artifact a version of a live network binds (its
/// initial binding, restorable as `initial`, or its current refreshed one,
/// restorable as `refreshed`) stays. What a refresh replaced, and what only a
/// deleted network bound, stays while a Node netmap that references it can
/// still be under a lease (`MAX_LEASE_MS`), the same window in which the
/// artifact endpoint still serves it.
pub(crate) fn collect(
    tx: &Transaction<'_>,
    sealer: &crate::secrets::Sealer,
    now: i64,
) -> Result<usize, Error> {
    let mut unused = sql::stored(tx)?;
    if unused.is_empty() {
        return Ok(0);
    }
    for digest in sql::leased(tx, now)? {
        unused.remove(&digest);
    }
    for text in crate::store::resources::artifacts::refreshed_bindings(tx)? {
        let bindings: super::bindings::Bindings =
            serde_json::from_str(&text).map_err(|_| Error::decode("rule_set_bindings.bindings"))?;
        for digest in bindings.values() {
            unused.remove(digest);
        }
    }
    crate::store::configuration::versions::each_restorable_bindings(tx, sealer, |bindings| {
        for digest in bindings.rule_sets.values() {
            unused.remove(digest);
        }
        Ok(())
    })?;
    for digest in &unused {
        sql::delete(tx, digest)?;
    }
    if !unused.is_empty() {
        LOADED
            .lock()
            .map_err(|_| Error::RUNTIME)?
            .retain(|digest, _| !unused.contains(digest));
    }
    Ok(unused.len())
}

/// The current record of `digest` as bound for `source` (the latest
/// preparation that produced it); inline content is described by converting
/// its declaration again.
pub(crate) fn record(
    tx: &Transaction<'_>,
    source: &Source,
    digest: &Digest,
) -> Result<Option<Record>, Error> {
    if let Source::Inline(set) = source {
        return Ok(super::rulesets::inline(set)
            .ok()
            .filter(|prepared| prepared.artifact.digest() == *digest)
            .map(|prepared| prepared.record));
    }
    sql::load_record(tx, &source.identity(), digest)?
        .map(|text| {
            serde_json::from_str(&text).map_err(|_| Error::decode("rule_set_record_heads.record"))
        })
        .transpose()
}
